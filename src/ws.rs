use axum::{
    extract::ws::{Message, WebSocketUpgrade},
    response::IntoResponse,
};
use futures_util::{stream::StreamExt, SinkExt};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskType {
    RefreshIndex,
    DownloadVideo,
}

#[derive(Debug, Clone)]
pub struct TaskStatus {
    pub id: TaskId,
    pub task_type: TaskType,
    pub title: String,
    pub created_at: Instant,
    pub removed: bool,
    pub removed_at: Option<Instant>,
    pub failed: bool,
    pub error_message: Option<String>,
}

// Serializable version of TaskStatus for sending over WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTaskStatus {
    pub id: TaskId,
    pub task_type: TaskType,
    pub title: String,
    pub removed: bool,
    pub failed: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskUpdate {
    pub tasks: Vec<SerializableTaskStatus>,
}

#[derive(Clone)]
pub struct TaskManager {
    pub tasks: Arc<Mutex<HashMap<TaskId, TaskStatus>>>,
    pub tx: broadcast::Sender<TaskUpdate>,
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskManager {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            tx,
        }
    }

    pub fn add_task(&self, task: TaskStatus) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task.id.clone(), task);
        } // Lock is released here
        self.broadcast_update();
    }

    pub fn remove_task(&self, id: &str) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                // Mark the task as removed
                task.removed = true;
                task.removed_at = Some(Instant::now());
            }
        } // Lock is released here
        self.broadcast_update();
    }

    pub fn mark_task_failed(&self, id: &str, error_message: String) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.failed = true;
                task.error_message = Some(error_message);
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
                    removed: task.removed,
                    failed: task.failed,
                    error_message: task.error_message.clone(),
                })
                .collect::<Vec<SerializableTaskStatus>>()
        }; // Mutex lock is released here

        // Now send without holding the lock
        let _ = self.tx.send(TaskUpdate { tasks: task_list });
    }

    // Clean up tasks that have been marked as removed for over 5 seconds
    // (or 30 seconds for failed tasks)
    pub fn cleanup_old_tasks(&self) {
        let now = Instant::now();
        let task_ids_to_remove = {
            // First, identify tasks to remove while holding the lock
            let tasks = self.tasks.lock().unwrap();
            tasks
                .iter()
                .filter(|(_, task)| {
                    if !task.removed || task.removed_at.is_none() {
                        return false;
                    }

                    let removed_time = task.removed_at.unwrap();
                    let timeout_duration = if task.failed {
                        // Keep failed tasks for 30 seconds
                        Duration::from_secs(30)
                    } else {
                        // Normal tasks for 5 seconds
                        Duration::from_secs(5)
                    };

                    now.duration_since(removed_time) > timeout_duration
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
                        removed: task.removed,
                        failed: task.failed,
                        error_message: task.error_message.clone(),
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
pub fn register_download_task(title: String) -> TaskId {
    let id = uuid::Uuid::new_v4().to_string();
    let task = TaskStatus {
        id: id.clone(),
        task_type: TaskType::DownloadVideo,
        title,
        created_at: Instant::now(),
        removed: false,
        removed_at: None,
        failed: false,
        error_message: None,
    };
    TaskManager::global().add_task(task);
    id
}

// Helper function to register a task for index refresh
pub fn register_refresh_task(title: String) -> TaskId {
    let id = uuid::Uuid::new_v4().to_string();
    let task = TaskStatus {
        id: id.clone(),
        task_type: TaskType::RefreshIndex,
        title,
        created_at: Instant::now(),
        removed: false,
        removed_at: None,
        failed: false,
        error_message: None,
    };
    TaskManager::global().add_task(task);
    id
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
