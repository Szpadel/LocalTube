use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;
use tracing::info;

use crate::gluetun::controller::{GluetunError, GluetunRestartOutcome};
use crate::job_tracking::{
    metrics::{AllMetrics, TaskMetricData, TaskMetrics},
    task::{QueuedTask, SerializableTaskStatus, Task, TaskState, TaskStatus, TaskType, TaskUpdate},
};

// Global task manager instance without automatic cleanup task.
static TASK_MANAGER: std::sync::LazyLock<TaskManager> = std::sync::LazyLock::new(|| {
    let manager = TaskManager::new();
    info!("Task Manager initialized");
    manager
});

#[derive(Clone)]
pub struct TaskManager {
    pub tasks: Arc<Mutex<HashMap<String, TaskStatus>>>,
    pub tx: broadcast::Sender<TaskUpdate>,
    pub(crate) metrics: Arc<RwLock<HashMap<TaskType, TaskMetricData>>>,
    metrics_tx: broadcast::Sender<AllMetrics>,
    gluetun_enabled: Arc<AtomicBool>,
    gluetun_restart_in_progress: Arc<AtomicBool>,
}

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
            .finish_non_exhaustive()
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskManager {
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        let (metrics_tx, _) = broadcast::channel(100);
        let mut metrics = HashMap::new();
        for task_type in &[TaskType::RefreshIndex, TaskType::DownloadVideo] {
            metrics.insert(task_type.clone(), TaskMetricData::default());
        }
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            tx,
            metrics: Arc::new(RwLock::new(metrics)),
            metrics_tx,
            gluetun_enabled: Arc::new(AtomicBool::new(false)),
            gluetun_restart_in_progress: Arc::new(AtomicBool::new(false)),
        }
    }

    #[must_use]
    pub fn global() -> &'static TaskManager {
        &TASK_MANAGER
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned.
    #[must_use]
    pub fn add_task(&self, task_type: TaskType, title: String) -> QueuedTask {
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
        }
        self.broadcast_update();
        QueuedTask {
            inner: Task::new(id, self.clone()),
        }
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned.
    pub fn update_task_title(&self, id: &str, title: String) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.title = title;
            }
        }
        self.broadcast_update();
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned.
    pub fn update_task_status(&self, id: &str, status: String) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.status = Some(status);
            }
        }
        self.broadcast_update();
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned.
    pub fn mark_task_started(&self, id: &str) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.state = TaskState::InProgress;
            }
        }
        self.broadcast_update();
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned.
    pub fn mark_task_failed(&self, id: &str, error_message: String) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                task.state = TaskState::Failed(error_message);
                task.completed_at = Some(Instant::now());
            }
        }
        self.broadcast_update();
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned.
    pub fn remove_task(&self, id: &str) {
        let (task_type, final_state) = {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(id) {
                if task.completed_at.is_none() {
                    task.completed_at = Some(Instant::now());
                }
                Some((task.task_type.clone(), task.state.clone()))
            } else {
                None
            }
        }
        .map_or((None, None), |(tt, fs)| (Some(tt), Some(fs)));

        let now = Instant::now();

        if let (Some(task_type), Some(state)) = (&task_type, &final_state) {
            let mut metrics = self.metrics.write().unwrap();
            if let Some(data) = metrics.get_mut(task_type) {
                match state {
                    TaskState::Failed(_) => {
                        data.failure += 1;
                        data.consecutive_failures += 1;
                        data.last_failure = Some(now);
                    }
                    TaskState::Completed => {
                        data.success += 1;
                        data.consecutive_failures = 0;
                        data.last_success = Some(now);
                    }
                    _ => {}
                }
            }
        }

        self.broadcast_update();
        self.broadcast_metrics();
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned.
    pub fn cleanup_old_tasks(&self) {
        let now = Instant::now();
        let task_ids_to_remove = {
            let tasks = self.tasks.lock().unwrap();
            tasks
                .iter()
                .filter(|(_, task)| match &task.state {
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
                    TaskState::Queued => task.completed_at.is_some_and(|completed_time| {
                        now.duration_since(completed_time) > Duration::from_secs(5)
                    }),
                    TaskState::InProgress => task.completed_at.is_some_and(|completed_time| {
                        // Dropped tasks mark a completion timestamp without updating the state.
                        now.duration_since(completed_time) > Duration::from_secs(5)
                    }),
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<String>>()
        };

        if !task_ids_to_remove.is_empty() {
            {
                let mut tasks = self.tasks.lock().unwrap();
                for id in &task_ids_to_remove {
                    tasks.remove(id);
                }
            }

            self.broadcast_update();
        }
    }

    /// # Panics
    ///
    /// Panics if the metrics map lock is poisoned.
    #[must_use]
    pub fn get_metrics(&self) -> AllMetrics {
        let metrics = self.metrics.read().unwrap();
        let now = Instant::now();
        let tasks = metrics
            .iter()
            .map(|(task_type, data)| {
                let last_success_seconds_ago = data.last_success.and_then(|t| {
                    now.checked_duration_since(t)
                        .map(|duration| duration.as_secs())
                });
                let last_failure_seconds_ago = data.last_failure.and_then(|t| {
                    now.checked_duration_since(t)
                        .map(|duration| duration.as_secs())
                });
                let last_restart_reference = if data.restart.in_progress {
                    data.restart.last_started
                } else {
                    data.restart.last_completed.or(data.restart.last_started)
                };
                let last_restart_seconds_ago = last_restart_reference.and_then(|t| {
                    now.checked_duration_since(t)
                        .map(|duration| duration.as_secs())
                });
                (
                    task_type.clone(),
                    TaskMetrics {
                        success_count: data.success,
                        failure_count: data.failure,
                        consecutive_failures: data.consecutive_failures,
                        last_success_seconds_ago,
                        last_failure_seconds_ago,
                        restart_count: data.restart.count,
                        last_restart_seconds_ago,
                        last_restart_outcome: data.restart.last_outcome.clone(),
                        last_restart_error: data.restart.last_error.clone(),
                        restart_in_progress: data.restart.in_progress,
                    },
                )
            })
            .collect();

        AllMetrics {
            tasks,
            gluetun_enabled: self.gluetun_enabled_internal(),
        }
    }

    #[must_use]
    ///
    /// # Panics
    ///
    /// Panics if the metrics map lock is poisoned.
    pub fn gluetun_enabled(&self) -> bool {
        self.gluetun_enabled_internal()
    }

    /// # Panics
    ///
    /// Panics if the metrics map lock is poisoned.
    pub fn set_gluetun_enabled(&self, enabled: bool) {
        self.gluetun_enabled.store(enabled, Ordering::SeqCst);

        if !enabled {
            self.gluetun_restart_in_progress
                .store(false, Ordering::SeqCst);
            let mut metrics = self.metrics.write().unwrap();
            if let Some(data) = metrics.get_mut(&TaskType::DownloadVideo) {
                data.restart.in_progress = false;
            }
        }

        self.broadcast_update();
        self.broadcast_metrics();
    }

    #[must_use]
    pub fn subscribe_metrics(&self) -> broadcast::Receiver<AllMetrics> {
        self.metrics_tx.subscribe()
    }

    #[must_use]
    ///
    /// # Panics
    ///
    /// Panics if the metrics map lock is poisoned.
    pub fn begin_gluetun_restart(&self) -> bool {
        if !self.gluetun_enabled() {
            return false;
        }

        if self
            .gluetun_restart_in_progress
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }

        let now = Instant::now();
        {
            let mut metrics = self.metrics.write().unwrap();
            if let Some(data) = metrics.get_mut(&TaskType::DownloadVideo) {
                data.restart.in_progress = true;
                data.restart.last_started = Some(now);
                data.restart.last_error = None;
                data.restart.last_outcome = None;
            }
        }

        self.broadcast_metrics();
        true
    }

    ///
    /// # Panics
    ///
    /// Panics if the metrics map lock is poisoned.
    pub fn finish_gluetun_restart(
        &self,
        outcome: std::result::Result<GluetunRestartOutcome, GluetunError>,
    ) {
        let now = Instant::now();
        {
            let mut metrics = self.metrics.write().unwrap();
            if let Some(data) = metrics.get_mut(&TaskType::DownloadVideo) {
                data.restart.in_progress = false;
                data.restart.last_completed = Some(now);
                match outcome {
                    Ok(result) => {
                        data.restart.count += 1;
                        data.restart.last_outcome = Some(result.to_string());
                        data.restart.last_error = None;
                        data.consecutive_failures = 0;
                    }
                    Err(err) => {
                        data.restart.last_error = Some(err.to_string());
                    }
                }
            }
        }

        self.gluetun_restart_in_progress
            .store(false, Ordering::SeqCst);
        self.broadcast_metrics();
    }

    fn gluetun_enabled_internal(&self) -> bool {
        self.gluetun_enabled.load(Ordering::SeqCst)
    }

    fn broadcast_metrics(&self) {
        let snapshot = self.get_metrics();
        let _ = self.metrics_tx.send(snapshot);
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned.
    pub fn broadcast_update(&self) {
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
        };

        let _ = self.tx.send(TaskUpdate { tasks: task_list });
    }
}

#[must_use]
/// # Panics
///
/// Panics if the task registry mutex is poisoned.
pub fn register_download_task(title: String) -> QueuedTask {
    TaskManager::global().add_task(TaskType::DownloadVideo, title)
}

#[must_use]
/// # Panics
///
/// Panics if the task registry mutex is poisoned.
pub fn register_refresh_task(title: String) -> QueuedTask {
    TaskManager::global().add_task(TaskType::RefreshIndex, title)
}

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
