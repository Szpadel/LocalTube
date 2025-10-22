use std::{future::Future, time::Duration};

use loco_rs::Result;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

/// Utility for scheduling retry logic with a guard check before executing the action.
pub struct RetryScheduler;

impl RetryScheduler {
    #[must_use]
    pub fn spawn<Check, CheckFut, Action, ActionFut>(
        delay: Duration,
        check: Check,
        action: Action,
    ) -> JoinHandle<()>
    where
        Check: FnOnce() -> CheckFut + Send + 'static,
        CheckFut: Future<Output = Result<bool>> + Send + 'static,
        Action: FnOnce() -> ActionFut + Send + 'static,
        ActionFut: Future<Output = Result<()>> + Send + 'static,
    {
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;

            match check().await {
                Ok(true) => {
                    debug!("retry condition satisfied; executing action");
                    if let Err(err) = action().await {
                        error!(error = ?err, "retry action failed");
                    }
                }
                Ok(false) => {
                    debug!("retry check reported no pending work; skipping action");
                }
                Err(err) => {
                    info!(error = ?err, "retry check failed; skipping action");
                }
            }
        })
    }

    pub fn spawn_detached<Check, CheckFut, Action, ActionFut>(
        delay: Duration,
        check: Check,
        action: Action,
    ) where
        Check: FnOnce() -> CheckFut + Send + 'static,
        CheckFut: Future<Output = Result<bool>> + Send + 'static,
        Action: FnOnce() -> ActionFut + Send + 'static,
        ActionFut: Future<Output = Result<()>> + Send + 'static,
    {
        let handle = Self::spawn(delay, check, action);
        drop(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::RetryScheduler;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        sync::Arc,
        time::Duration,
    };
    use tokio::time::sleep;

    #[tokio::test]
    async fn executes_action_when_check_passes() {
        let check_calls = Arc::new(AtomicUsize::new(0));
        let action_calls = Arc::new(AtomicUsize::new(0));

        let check_counter = Arc::clone(&check_calls);
        let action_counter = Arc::clone(&action_calls);

        let handle = RetryScheduler::spawn(
            Duration::from_millis(5),
            move || {
                let check_counter = Arc::clone(&check_counter);
                async move {
                    check_counter.fetch_add(1, Ordering::SeqCst);
                    Ok(true)
                }
            },
            move || {
                let action_counter = Arc::clone(&action_counter);
                async move {
                    action_counter.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );

        sleep(Duration::from_millis(1)).await;
        assert_eq!(action_calls.load(Ordering::SeqCst), 0);

        sleep(Duration::from_millis(10)).await;
        handle.await.expect("retry task panicked");

        assert_eq!(check_calls.load(Ordering::SeqCst), 1);
        assert_eq!(action_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn skips_action_when_check_returns_false() {
        let action_calls = Arc::new(AtomicUsize::new(0));
        let action_counter = Arc::clone(&action_calls);

        let handle = RetryScheduler::spawn(
            Duration::from_millis(5),
            || async move { Ok(false) },
            move || {
                let action_counter = Arc::clone(&action_counter);
                async move {
                    action_counter.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );

        sleep(Duration::from_millis(10)).await;
        handle.await.expect("retry task panicked");

        assert_eq!(action_calls.load(Ordering::SeqCst), 0);
    }
}
