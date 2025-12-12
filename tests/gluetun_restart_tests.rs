use async_trait::async_trait;
use localtube::gluetun::{
    controller::{GluetunController, GluetunError, GluetunRestartOutcome},
    supervisor,
};
use localtube::job_tracking::{
    manager::TaskManager, metrics::MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART, task::TaskType,
};
use std::{
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, time::timeout};

static TEST_SERIAL_GUARD: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Debug)]
struct MockController {
    notify: Arc<Notify>,
    result: Mutex<Option<std::result::Result<GluetunRestartOutcome, GluetunError>>>,
}

impl MockController {
    fn success(notify: Arc<Notify>) -> Self {
        Self {
            notify,
            result: Mutex::new(Some(Ok(GluetunRestartOutcome {
                stop_outcome: Some("stopped".to_string()),
                start_outcome: Some("running".to_string()),
            }))),
        }
    }

    fn failure(notify: Arc<Notify>) -> Self {
        Self {
            notify,
            result: Mutex::new(Some(Err(GluetunError::PollTimeout))),
        }
    }
}

#[async_trait]
impl GluetunController for MockController {
    async fn restart(&self) -> std::result::Result<GluetunRestartOutcome, GluetunError> {
        self.notify.notify_waiters();
        self.result.lock().expect("lock").take().unwrap_or_else(|| {
            Ok(GluetunRestartOutcome {
                stop_outcome: None,
                start_outcome: None,
            })
        })
    }
}

async fn spin_until<F>(limit: Duration, mut predicate: F)
where
    F: FnMut() -> bool,
{
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if predicate() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("condition not met within {:?}", limit);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn triggers_restart_after_threshold() {
    let _guard = TEST_SERIAL_GUARD.lock().await;
    let manager = TaskManager::new();
    let notify = Arc::new(Notify::new());
    let controller: Arc<dyn GluetunController> = Arc::new(MockController::success(notify.clone()));
    supervisor::activate(&controller, &manager);

    for idx in 0..MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART {
        let queued = manager.add_task(TaskType::DownloadVideo, format!("fail-{idx}"));
        let id = queued.id().to_string();
        manager.mark_task_failed(&id, "simulated failure".to_string());
        manager.remove_task(&id);
    }

    timeout(Duration::from_secs(1), notify.notified())
        .await
        .expect("restart not triggered in time");

    spin_until(Duration::from_secs(1), || {
        let snapshot = manager.get_metrics();
        if let Some(download) = snapshot.tasks.get(&TaskType::DownloadVideo) {
            !download.restart_in_progress
                && download.restart_count == 1
                && download.consecutive_failures == 0
                && download.last_restart_error.is_none()
        } else {
            false
        }
    })
    .await;
    supervisor::deactivate(&manager);
}

#[tokio::test]
async fn records_restart_failure() {
    let _guard = TEST_SERIAL_GUARD.lock().await;
    let manager = TaskManager::new();
    let notify = Arc::new(Notify::new());
    let controller: Arc<dyn GluetunController> = Arc::new(MockController::failure(notify.clone()));
    supervisor::activate(&controller, &manager);

    for idx in 0..MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART {
        let queued = manager.add_task(TaskType::DownloadVideo, format!("fail-{idx}"));
        let id = queued.id().to_string();
        manager.mark_task_failed(&id, "simulated failure".to_string());
        manager.remove_task(&id);
    }

    timeout(Duration::from_secs(1), notify.notified())
        .await
        .expect("restart not triggered in time");

    spin_until(Duration::from_secs(1), || {
        let snapshot = manager.get_metrics();
        if let Some(download) = snapshot.tasks.get(&TaskType::DownloadVideo) {
            !download.restart_in_progress
                && download.restart_count == 0
                && download.last_restart_error.is_some()
        } else {
            false
        }
    })
    .await;
    supervisor::deactivate(&manager);
}

#[tokio::test]
async fn triggers_restart_after_threshold_on_refresh_failures() {
    let _guard = TEST_SERIAL_GUARD.lock().await;
    let manager = TaskManager::new();
    let notify = Arc::new(Notify::new());
    let controller: Arc<dyn GluetunController> = Arc::new(MockController::success(notify.clone()));
    supervisor::activate(&controller, &manager);

    for idx in 0..MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART {
        let queued = manager.add_task(TaskType::RefreshIndex, format!("fail-refresh-{idx}"));
        let id = queued.id().to_string();
        manager.mark_task_failed(&id, "simulated refresh failure".to_string());
        manager.remove_task(&id);
    }

    timeout(Duration::from_secs(1), notify.notified())
        .await
        .expect("restart not triggered in time");

    spin_until(Duration::from_secs(1), || {
        let snapshot = manager.get_metrics();
        if let Some(refresh) = snapshot.tasks.get(&TaskType::RefreshIndex) {
            !refresh.restart_in_progress
                && refresh.restart_count == 1
                && refresh.consecutive_failures == 0
                && refresh.last_restart_error.is_none()
        } else {
            false
        }
    })
    .await;
    supervisor::deactivate(&manager);
}

#[tokio::test]
async fn refresh_restart_does_not_reset_download_failure_streak() {
    let _guard = TEST_SERIAL_GUARD.lock().await;
    let manager = TaskManager::new();
    let notify = Arc::new(Notify::new());
    let controller: Arc<dyn GluetunController> = Arc::new(MockController::success(notify.clone()));
    supervisor::activate(&controller, &manager);

    let queued = manager.add_task(TaskType::DownloadVideo, "download-fail".to_string());
    let id = queued.id().to_string();
    manager.mark_task_failed(&id, "simulated download failure".to_string());
    manager.remove_task(&id);

    for idx in 0..MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART {
        let queued = manager.add_task(TaskType::RefreshIndex, format!("fail-refresh-{idx}"));
        let id = queued.id().to_string();
        manager.mark_task_failed(&id, "simulated refresh failure".to_string());
        manager.remove_task(&id);
    }

    timeout(Duration::from_secs(1), notify.notified())
        .await
        .expect("restart not triggered in time");

    spin_until(Duration::from_secs(1), || {
        let snapshot = manager.get_metrics();

        let Some(download) = snapshot.tasks.get(&TaskType::DownloadVideo) else {
            return false;
        };

        let Some(refresh) = snapshot.tasks.get(&TaskType::RefreshIndex) else {
            return false;
        };

        !refresh.restart_in_progress
            && refresh.restart_count == 1
            && refresh.consecutive_failures == 0
            && refresh.last_restart_error.is_none()
            && download.restart_count == 0
            && download.consecutive_failures == 1
    })
    .await;
    supervisor::deactivate(&manager);
}
