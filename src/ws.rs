use axum::{
    extract::ws::{Message, WebSocketUpgrade},
    response::IntoResponse,
};
use futures_util::{stream::StreamExt, SinkExt};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::broadcast;
use tracing::info;

// Global task manager instance without automatic cleanup task
static TASK_MANAGER: Lazy<TaskManager> = Lazy::new(|| {
    let manager = TaskManager::new();
    info!("Task Manager initialized");
    manager
});

pub type TaskId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TaskType {
    RefreshIndex,
    DownloadVideo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskState {
    Queued,
    InProgress,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct TaskStatus {
    pub id: TaskId,
    pub task_type: TaskType,
    pub title: String,
    pub created_at: Instant,
    pub state: TaskState,
    pub completed_at: Option<Instant>,
    pub status: Option<String>,
}

// Serializable version of TaskStatus for sending over WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTaskStatus {
    pub id: TaskId,
    pub task_type: TaskType,
    pub title: String,
    pub state: TaskState,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskUpdate {
    pub tasks: Vec<SerializableTaskStatus>,
}

#[derive(Default)]
struct TaskMetricData {
    success: u64,
    failure: u64,
    consecutive_failures: u64,
    last_success: Option<Instant>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMetrics {
    pub success_count: u64,
    pub failure_count: u64,
    pub consecutive_failures: u64,
    pub last_success_seconds_ago: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllMetrics {
    pub tasks: HashMap<TaskType, TaskMetrics>,
}

#[derive(Clone)]
pub struct TaskManager {
    pub tasks: Arc<Mutex<HashMap<TaskId, TaskStatus>>>,
    pub tx: broadcast::Sender<TaskUpdate>,
    metrics: Arc<RwLock<HashMap<TaskType, TaskMetricData>>>,
}

// Manual implementation of Debug for TaskManager
impl std::fmt::Debug for TaskManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskManager")
            .field(
                "tasks",
                &format!(
                    "Arc<Mutex<HashMap<TaskId, TaskStatus>>> with {} entries",
                    self.tasks.lock().map(|t| t.len()).unwrap_or(0)
                ),
            )
            .field("tx", &"broadcast::Sender<TaskUpdate>")
            .finish()
    }
}

/// A task that is queued, waiting for semaphore permit
#[derive(Debug)]
pub struct QueuedTask {
    inner: Task,
}

/// A task that is actively running (holds semaphore permit)
#[derive(Debug)]
pub struct ActiveTask {
    inner: Task,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

// RAII Task handle - automatically completes the task when dropped
#[derive(Debug)]
pub struct Task {
    id: TaskId,
    manager: &'static TaskManager,
    completed: bool, // Flag to avoid double-completion
}

impl Task {
    fn new(id: TaskId, manager: &'static TaskManager) -> Self {
        Self {
            id,
            manager,
            completed: false,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    // Update the task title
    pub fn update_title(&self, title: String) {
        self.manager.update_task_title(&self.id, title);
    }

    // Set a status message on the task
    pub fn update_status(&self, status: String) {
        self.manager.update_task_status(&self.id, status);
    }

    // Mark the task as started (transition from Queued to InProgress)
    pub fn mark_started(&self) {
        self.manager.mark_task_started(&self.id);
    }

    // Mark the task as failed
    pub fn mark_failed(&self, error_message: String) {
        self.manager.mark_task_failed(&self.id, error_message);
    }

    // Explicitly mark as complete
    pub fn complete(mut self) {
        if !self.completed {
            self.completed = true;
            // Mark as completed in the task status for metrics
            {
                let mut tasks = self.manager.tasks.lock().unwrap();
                if let Some(task) = tasks.get_mut(&self.id) {
                    task.state = TaskState::Completed;
                    task.completed_at = Some(Instant::now());
                }
            }
            self.manager.remove_task(&self.id);
        }
    }

    // Prevent automatic completion on drop
    pub fn forget(mut self) {
        self.completed = true;
    }
}

// Automatically complete the task when dropped
impl Drop for Task {
    fn drop(&mut self) {
        if !self.completed {
            self.manager.remove_task(&self.id);
        }
    }
}

impl QueuedTask {
    pub fn id(&self) -> &str {
        self.inner.id()
    }

    pub fn update_title(&self, title: String) {
        self.inner.update_title(title);
    }

    /// Transition to active state by acquiring semaphore permit.
    /// This is where the task actually waits if semaphore is full.
    ///
    /// # Arguments
    /// * `sem` - The semaphore to acquire a permit from. This allows different
    ///   task types to use different concurrency limits.
    pub async fn start(self, sem: &'static std::sync::Arc<tokio::sync::Semaphore>) -> ActiveTask {
        let permit = sem.clone().acquire_owned().await.unwrap();

        // Automatically mark as started now that we have the permit
        self.inner.manager.mark_task_started(&self.inner.id);

        ActiveTask {
            inner: self.inner,
            _permit: permit,
        }
    }
}

impl ActiveTask {
    pub fn id(&self) -> &str {
        self.inner.id()
    }

    pub fn update_status(&self, status: String) {
        self.inner.update_status(status);
    }

    pub fn complete(self) {
        self.inner.complete();
        // _permit automatically released on drop
    }

    pub fn mark_failed(self, error_message: String) {
        self.inner.mark_failed(error_message);
        // _permit automatically released on drop
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskManager {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        let mut metrics = HashMap::new();
        for task_type in &[TaskType::RefreshIndex, TaskType::DownloadVideo] {
            metrics.insert(task_type.clone(), TaskMetricData::default());
        }
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            tx,
            metrics: Arc::new(RwLock::new(metrics)),
        }
    }

    // Create a new task and return a QueuedTask handle
    pub fn add_task(&'static self, task_type: TaskType, title: String) -> QueuedTask {
        let id = uuid::Uuid::new_v4().to_string();
        let task = TaskStatus {
            id: id.clone(),
            task_type,
            title,
            created_at: Instant::now(),
            state: TaskState::Queued,
            completed_at: None,
            status: None,
        };
        {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(id.clone(), task);
        } // Lock is released here
        self.broadcast_update();
        QueuedTask {
            inner: Task::new(id, self),
        }
    }

    // Update the task title
    pub fn update_task_title(&self, id: &str, title: String) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.title = title;
            }
        } // Lock is released here
        self.broadcast_update();
    }

    // Update the task status message
    pub fn update_task_status(&self, id: &str, status: String) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.status = Some(status);
            }
        } // Lock is released here
        self.broadcast_update();
    }

    // Mark task as started (transition from Queued to InProgress)
    pub fn mark_task_started(&self, id: &str) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.state = TaskState::InProgress;
            }
        } // Lock is released here
        self.broadcast_update();
    }

    // Internal method called by Task when dropped or completed
    pub fn remove_task(&self, id: &str) {
        let (task_type, final_state) = {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                // Always set completed_at if not already set
                if task.completed_at.is_none() {
                    task.completed_at = Some(Instant::now());
                }
                Some((task.task_type.clone(), task.state.clone()))
            } else {
                None
            }
        }
        .map(|(tt, fs)| (Some(tt), Some(fs)))
        .unwrap_or((None, None));

        // Update metrics based on final state
        if let (Some(task_type), Some(state)) = (&task_type, &final_state) {
            let mut metrics = self.metrics.write().unwrap();
            if let Some(data) = metrics.get_mut(task_type) {
                match state {
                    TaskState::Failed(_) => {
                        data.failure += 1;
                        data.consecutive_failures += 1;
                    }
                    TaskState::Completed => {
                        data.success += 1;
                        data.consecutive_failures = 0;
                        data.last_success = Some(Instant::now());
                    }
                    _ => {}
                }
            }
        }

        // Always broadcast the final state to all clients
        self.broadcast_update();
    }

    pub fn mark_task_failed(&self, id: &str, error_message: String) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.state = TaskState::Failed(error_message);
                task.completed_at = Some(Instant::now());
            }
        } // Lock is released here
        self.broadcast_update();
    }

    pub fn broadcast_update(&self) {
        // Create a copy of the task list without holding the lock during broadcast
        let task_list = {
            let tasks = self.tasks.lock().unwrap();
            tasks
                .values()
                .map(|task| SerializableTaskStatus {
                    id: task.id.clone(),
                    task_type: task.task_type.clone(),
                    title: task.title.clone(),
                    state: task.state.clone(),
                    status: task.status.clone(),
                })
                .collect::<Vec<SerializableTaskStatus>>()
        }; // Mutex lock is released here

        // Now send without holding the lock
        let _ = self.tx.send(TaskUpdate { tasks: task_list });
    }

    // Clean up tasks that have been marked as completed/failed for over 5 seconds
    // (or 30 seconds for failed tasks)
    pub fn cleanup_old_tasks(&self) {
        let now = Instant::now();
        let task_ids_to_remove = {
            // First, identify tasks to remove while holding the lock
            let tasks = self.tasks.lock().unwrap();
            tasks
                .iter()
                .filter(|(_, task)| {
                    // Only cleanup completed or failed tasks
                    match &task.state {
                        TaskState::Completed | TaskState::Failed(_) => {
                            if let Some(completed_time) = task.completed_at {
                                let timeout_duration = match task.state {
                                    TaskState::Failed(_) => Duration::from_secs(30),
                                    _ => Duration::from_secs(5),
                                };
                                now.duration_since(completed_time) > timeout_duration
                            } else {
                                false
                            }
                        }
                        _ => false,
                    }
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<String>>()
        }; // Lock is released here

        // Only lock again if there's work to do
        if !task_ids_to_remove.is_empty() {
            {
                let mut tasks = self.tasks.lock().unwrap();
                for id in &task_ids_to_remove {
                    tasks.remove(id);
                }
            } // Lock is released here

            // Broadcast after releasing the lock
            self.broadcast_update();
        }
    }

    // Get the global task manager instance
    pub fn global() -> &'static TaskManager {
        &TASK_MANAGER
    }

    pub fn get_metrics(&self) -> AllMetrics {
        let metrics = self.metrics.read().unwrap();
        let now = Instant::now();
        let tasks = metrics
            .iter()
            .map(|(task_type, data)| {
                let last_success_seconds_ago =
                    data.last_success.map(|t| now.duration_since(t).as_secs());
                (
                    task_type.clone(),
                    TaskMetrics {
                        success_count: data.success,
                        failure_count: data.failure,
                        consecutive_failures: data.consecutive_failures,
                        last_success_seconds_ago,
                    },
                )
            })
            .collect();

        AllMetrics { tasks }
    }
}

pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    info!("WebSocket connection request received at /ws/status");
    ws.on_upgrade(move |socket| async move {
        info!("WebSocket connection established successfully");
        let task_manager = TaskManager::global();

        // Log current task count
        let task_count = task_manager.tasks.lock().unwrap().len();
        info!("Current task count: {}", task_count);

        let (mut sender, mut receiver) = socket.split();
        let mut rx = task_manager.tx.subscribe();

        // Send initial task list - use the broadcast_update method which is now safe
        task_manager.broadcast_update();

        // Create a local task update message for immediate response
        let task_update = {
            let task_list = {
                let tasks = task_manager.tasks.lock().unwrap();
                tasks
                    .values()
                    .map(|task| SerializableTaskStatus {
                        id: task.id.clone(),
                        task_type: task.task_type.clone(),
                        title: task.title.clone(),
                        state: task.state.clone(),
                        status: task.status.clone(),
                    })
                    .collect::<Vec<SerializableTaskStatus>>()
            };
            TaskUpdate { tasks: task_list }
        };

        if let Ok(msg) = serde_json::to_string(&task_update) {
            if let Err(e) = sender.send(Message::Text(msg.into())).await {
                info!("Error sending initial message: {:?}", e);
                return;
            }
        }

        // Debug test tasks removed - we now use real tasks only

        // Simple ping every 5 seconds to keep connection alive
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Send a WebSocket ping to check connection instead of task data
                        if let Err(e) = sender.send(Message::Ping(vec![1, 2, 3, 4].into())).await {
                            info!("Ping failed: {:?}", e);
                            break;
                        }
                    }
                    update = rx.recv() => {
                        if let Ok(update) = update {
                            if let Ok(msg) = serde_json::to_string(&update) {
                                if sender.send(Message::Text(msg.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });

        while let Some(Ok(_)) = receiver.next().await {
            // Just keep the connection alive
        }
    })
}

// Helper function to register a task for download
pub fn register_download_task(title: String) -> QueuedTask {
    TaskManager::global().add_task(TaskType::DownloadVideo, title)
}

// Helper function to register a task for index refresh
pub fn register_refresh_task(title: String) -> QueuedTask {
    TaskManager::global().add_task(TaskType::RefreshIndex, title)
}

// Start the background task that will clean up old tasks
pub fn start_cleanup_task(task_manager: TaskManager) {
    info!("Starting task cleanup background process");
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            task_manager.cleanup_old_tasks();
        }
    });
}
