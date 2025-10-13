use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::job_tracking::manager::TaskManager;

pub type TaskId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TaskType {
    RefreshIndex,
    DownloadVideo,
}

impl TaskType {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskType::RefreshIndex => "refresh_index",
            TaskType::DownloadVideo => "download_video",
        }
    }
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

#[derive(Debug)]
pub struct QueuedTask {
    pub(crate) inner: Task,
}

#[derive(Debug)]
pub struct ActiveTask {
    pub(crate) inner: Task,
    pub(crate) _permit: OwnedSemaphorePermit,
}

/// RAII Task handle - automatically completes the task when dropped.
#[derive(Debug, Clone)]
pub struct Task {
    pub(crate) id: TaskId,
    pub(crate) manager: TaskManager,
    pub(crate) completed: bool, // Flag to avoid double-completion
}

impl Task {
    #[must_use]
    pub fn new(id: TaskId, manager: TaskManager) -> Self {
        Self {
            id,
            manager,
            completed: false,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn update_title(&self, title: String) {
        self.manager.update_task_title(&self.id, title);
    }

    pub fn update_status(&self, status: String) {
        self.manager.update_task_status(&self.id, status);
    }

    pub fn mark_started(&self) {
        self.manager.mark_task_started(&self.id);
    }

    pub fn mark_failed(&self, error_message: String) {
        self.manager.mark_task_failed(&self.id, error_message);
    }

    /// # Panics
    ///
    /// Panics if the task registry mutex is poisoned while marking the task complete.
    pub fn complete(mut self) {
        if !self.completed {
            self.completed = true;
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

    pub fn forget(mut self) {
        self.completed = true;
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if !self.completed {
            self.manager.remove_task(&self.id);
        }
    }
}

impl QueuedTask {
    #[must_use]
    pub fn id(&self) -> &str {
        self.inner.id()
    }

    pub fn update_title(&self, title: String) {
        self.inner.update_title(title);
    }

    /// Transition to active state by acquiring semaphore permit.
    /// This is where the task actually waits if semaphore is full.
    ///
    /// # Panics
    ///
    /// Panics if the semaphore acquisition fails unexpectedly.
    pub async fn start(self, sem: Arc<Semaphore>) -> ActiveTask {
        let permit = sem.acquire_owned().await.unwrap();

        self.inner.manager.mark_task_started(&self.inner.id);

        ActiveTask {
            inner: self.inner,
            _permit: permit,
        }
    }
}

impl ActiveTask {
    #[must_use]
    pub fn id(&self) -> &str {
        self.inner.id()
    }

    pub fn update_status(&self, status: String) {
        self.inner.update_status(status);
    }

    pub fn complete(self) {
        self.inner.complete();
    }

    pub fn mark_failed(self, error_message: String) {
        self.inner.mark_failed(error_message);
    }
}
