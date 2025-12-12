use std::sync::{Arc, LazyLock, Mutex};

use tokio::sync::oneshot;
use tracing::{error, info, warn};

use crate::gluetun::controller::GluetunController;
use crate::job_tracking::{
    manager::TaskManager,
    metrics::{
        AllMetrics, TaskMetrics, MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART,
        MIN_SUCCESS_AGE_BEFORE_RESTART,
    },
    task::TaskType,
};

static SUPERVISOR: LazyLock<Mutex<Option<GluetunSupervisorHandle>>> =
    LazyLock::new(|| Mutex::new(None));

/// # Panics
///
/// Panics if the supervisor state mutex is poisoned.
pub fn activate(controller: &Arc<dyn GluetunController>, task_manager: &TaskManager) {
    let mut guard = SUPERVISOR.lock().unwrap();
    if let Some(handle) = guard.take() {
        drop(handle);
    }
    let handle = GluetunSupervisorHandle::spawn(controller, task_manager);
    *guard = Some(handle);
}

/// # Panics
///
/// Panics if the supervisor state mutex is poisoned.
pub fn deactivate(task_manager: &TaskManager) {
    let mut guard = SUPERVISOR.lock().unwrap();
    if let Some(handle) = guard.take() {
        drop(handle);
    }
    task_manager.set_gluetun_enabled(false);
}

/// Returns the active Gluetun controller when integration is enabled.
///
/// # Panics
///
/// Panics if the supervisor state mutex is poisoned.
#[must_use]
pub fn controller() -> Option<Arc<dyn GluetunController>> {
    SUPERVISOR
        .lock()
        .unwrap()
        .as_ref()
        .map(|handle| Arc::clone(&handle.controller))
}

struct GluetunSupervisorHandle {
    shutdown: Option<oneshot::Sender<()>>,
    controller: Arc<dyn GluetunController>,
}

impl GluetunSupervisorHandle {
    fn spawn(controller: &Arc<dyn GluetunController>, task_manager: &TaskManager) -> Self {
        let mut metrics_rx = task_manager.subscribe_metrics();
        task_manager.set_gluetun_enabled(true);
        let initial_metrics = task_manager.get_metrics();

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let controller_for_task = Arc::clone(controller);
        let controller_for_handle = Arc::clone(controller);
        let manager_clone = task_manager.clone();

        tokio::spawn(async move {
            handle_metrics(&initial_metrics, &manager_clone, &controller_for_task);
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    recv_result = metrics_rx.recv() => {
                        match recv_result {
                            Ok(metrics) => handle_metrics(&metrics, &manager_clone, &controller_for_task),
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                warn!("Gluetun supervisor lagged {skipped} metrics updates");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });

        Self {
            shutdown: Some(shutdown_tx),
            controller: controller_for_handle,
        }
    }
}

impl Drop for GluetunSupervisorHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
    }
}

fn handle_metrics(
    all_metrics: &AllMetrics,
    task_manager: &TaskManager,
    controller: &Arc<dyn GluetunController>,
) {
    if !all_metrics.gluetun_enabled {
        return;
    }

    let Some(trigger_task) = select_restart_trigger(all_metrics) else {
        return;
    };

    if task_manager.begin_gluetun_restart(Some(trigger_task.clone())) {
        info!(
            "Triggering Gluetun VPN restart after sustained failures: task={}",
            trigger_task.as_str()
        );
        let manager = task_manager.clone();
        let controller = Arc::clone(controller);
        let trigger_task = trigger_task.clone();
        tokio::spawn(async move {
            let outcome = controller.restart().await;
            match &outcome {
                Ok(result) => info!("Gluetun VPN restart succeeded: {}", result),
                Err(err) => error!("Gluetun VPN restart failed: {}", err),
            }
            manager.finish_gluetun_restart(Some(trigger_task), &outcome);
        });
    }
}

fn select_restart_trigger(metrics: &AllMetrics) -> Option<TaskType> {
    let download = metrics.tasks.get(&TaskType::DownloadVideo)?;
    if should_trigger_restart(download) {
        return Some(TaskType::DownloadVideo);
    }

    let refresh = metrics.tasks.get(&TaskType::RefreshIndex)?;
    if should_trigger_restart(refresh) {
        return Some(TaskType::RefreshIndex);
    }

    None
}

fn should_trigger_restart(metrics: &TaskMetrics) -> bool {
    if metrics.restart_in_progress {
        return false;
    }

    if metrics.consecutive_failures < MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART {
        return false;
    }

    if !restart_gate_allows(metrics) {
        return false;
    }

    true
}

fn restart_gate_allows(metrics: &TaskMetrics) -> bool {
    let threshold_secs = MIN_SUCCESS_AGE_BEFORE_RESTART.as_secs();
    if threshold_secs == 0 {
        return true;
    }

    match (
        metrics.last_success_seconds_ago,
        metrics.last_restart_seconds_ago,
    ) {
        (None, None) => true,
        (Some(success), None) => success >= threshold_secs,
        (None, Some(restart)) => restart >= threshold_secs,
        (Some(success), Some(restart)) => success.min(restart) >= threshold_secs,
    }
}
