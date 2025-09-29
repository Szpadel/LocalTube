use localtube::ws::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

// Helper to create a static task manager for tests
fn test_manager() -> &'static TaskManager {
    Box::leak(Box::new(TaskManager::new()))
}

// Helper to create a test semaphore with 2 permits
fn test_semaphore() -> &'static Arc<Semaphore> {
    Box::leak(Box::new(Arc::new(Semaphore::new(2))))
}

#[tokio::test]
async fn test_queued_to_active_transition() {
    let manager = test_manager();
    let sem = test_semaphore();
    let queued = manager.add_task(TaskType::DownloadVideo, "Test Task".into());

    // Task should start in Queued state
    let tasks = manager.tasks.lock().unwrap();
    let task_status = tasks.get(queued.id()).unwrap();
    assert!(
        matches!(task_status.state, TaskState::Queued),
        "Expected Queued state, got {:?}",
        task_status.state
    );
    drop(tasks);

    // Transition to active by acquiring permit
    let active = queued.start(sem).await;

    // Should now be InProgress
    let tasks = manager.tasks.lock().unwrap();
    let task_status = tasks.get(active.id()).unwrap();
    assert!(
        matches!(task_status.state, TaskState::InProgress),
        "Expected InProgress state, got {:?}",
        task_status.state
    );
}

#[tokio::test]
async fn test_tasks_queue_when_semaphore_full() {
    let manager = test_manager();
    let sem = test_semaphore();

    // Acquire both permits to fill the semaphore
    let _p1 = sem.acquire().await.unwrap();
    let _p2 = sem.acquire().await.unwrap();

    // Create third task - should queue
    let queued = manager.add_task(TaskType::DownloadVideo, "Queued Task".into());
    let id = queued.id().to_string();

    // Try to start it in background (will block waiting for semaphore)
    let handle = tokio::spawn(async move { queued.start(sem).await });

    // Give it time to hit the semaphore wait
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should still be Queued (blocked on semaphore)
    let tasks = manager.tasks.lock().unwrap();
    let task_status = tasks.get(&id).unwrap();
    assert!(
        matches!(task_status.state, TaskState::Queued),
        "Expected Queued state while waiting, got {:?}",
        task_status.state
    );
    drop(tasks);

    // Release one permit
    drop(_p1);

    // Task should now transition to InProgress
    let _active = handle.await.unwrap();
    let tasks = manager.tasks.lock().unwrap();
    let task_status = tasks.get(&id).unwrap();
    assert!(
        matches!(task_status.state, TaskState::InProgress),
        "Expected InProgress after permit available, got {:?}",
        task_status.state
    );
}

#[tokio::test]
async fn test_active_task_complete() {
    let manager = test_manager();
    let sem = test_semaphore();
    let queued = manager.add_task(TaskType::DownloadVideo, "Complete Task".into());
    let id = queued.id().to_string();
    let active = queued.start(sem).await;

    // Complete the task
    active.complete();

    // Should be in Completed state
    let tasks = manager.tasks.lock().unwrap();
    let task_status = tasks.get(&id).unwrap();
    assert!(
        matches!(task_status.state, TaskState::Completed),
        "Expected Completed state, got {:?}",
        task_status.state
    );
}

#[tokio::test]
async fn test_active_task_failed() {
    let manager = test_manager();
    let sem = test_semaphore();
    let queued = manager.add_task(TaskType::DownloadVideo, "Failed Task".into());
    let id = queued.id().to_string();
    let active = queued.start(sem).await;

    // Mark as failed
    active.mark_failed("Test error message".to_string());

    // Should be in Failed state with error message
    let tasks = manager.tasks.lock().unwrap();
    let task_status = tasks.get(&id).unwrap();
    match &task_status.state {
        TaskState::Failed(msg) => {
            assert_eq!(msg, "Test error message", "Error message mismatch");
        }
        other => panic!("Expected Failed state, got {:?}", other),
    }
}

#[tokio::test]
async fn test_permit_released_on_drop() {
    let manager = test_manager();
    let sem = test_semaphore();

    // Acquire both permits via tasks
    let q1 = manager.add_task(TaskType::DownloadVideo, "Task 1".into());
    let q2 = manager.add_task(TaskType::DownloadVideo, "Task 2".into());
    let a1 = q1.start(sem).await;
    let a2 = q2.start(sem).await;

    // Semaphore should be full
    assert_eq!(sem.available_permits(), 0, "Expected 0 available permits");

    // Drop one active task
    drop(a1);

    // Give a tiny bit of time for the drop to complete
    tokio::time::sleep(Duration::from_millis(1)).await;

    // Should have one permit available now
    assert_eq!(
        sem.available_permits(),
        1,
        "Expected 1 available permit after drop"
    );

    // Drop the second
    drop(a2);
    tokio::time::sleep(Duration::from_millis(1)).await;

    // Should have both permits back
    assert_eq!(
        sem.available_permits(),
        2,
        "Expected 2 available permits after both dropped"
    );
}

#[tokio::test]
async fn test_concurrent_task_limits() {
    let manager = test_manager();
    let sem = test_semaphore();

    // Start exactly 2 tasks (our semaphore limit)
    let q1 = manager.add_task(TaskType::DownloadVideo, "Concurrent 1".into());
    let q2 = manager.add_task(TaskType::DownloadVideo, "Concurrent 2".into());

    let a1 = q1.start(sem).await;
    let _a2 = q2.start(sem).await;

    // Both should be InProgress
    let tasks = manager.tasks.lock().unwrap();
    let in_progress_count = tasks
        .values()
        .filter(|t| matches!(t.state, TaskState::InProgress))
        .count();
    assert_eq!(in_progress_count, 2, "Expected 2 InProgress tasks");
    drop(tasks);

    // Try to start a third - should block
    let q3 = manager.add_task(TaskType::DownloadVideo, "Concurrent 3".into());
    let id3 = q3.id().to_string();

    let handle = tokio::spawn(async move { q3.start(sem).await });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Third should still be Queued
    let tasks = manager.tasks.lock().unwrap();
    let task3_status = tasks.get(&id3).unwrap();
    assert!(
        matches!(task3_status.state, TaskState::Queued),
        "Third task should be Queued"
    );
    let in_progress_count = tasks
        .values()
        .filter(|t| matches!(t.state, TaskState::InProgress))
        .count();
    assert_eq!(in_progress_count, 2, "Should never exceed 2 InProgress");
    drop(tasks);

    // Complete one task to free a permit
    a1.complete();

    // Third task should now become active
    let _a3 = handle.await.unwrap();

    let tasks = manager.tasks.lock().unwrap();
    let task3_status = tasks.get(&id3).unwrap();
    assert!(
        matches!(task3_status.state, TaskState::InProgress),
        "Third task should now be InProgress"
    );
}

#[tokio::test]
async fn test_cleanup_timing() {
    let manager = test_manager();
    let sem = test_semaphore();

    // Create and complete a task
    let queued = manager.add_task(TaskType::DownloadVideo, "Cleanup Test".into());
    let id = queued.id().to_string();
    let active = queued.start(sem).await;
    active.complete();

    // Task should exist immediately after completion
    {
        let tasks = manager.tasks.lock().unwrap();
        assert!(
            tasks.contains_key(&id),
            "Task should exist after completion"
        );
    }

    // Should still exist after 3 seconds (cleanup is at 5s for completed)
    tokio::time::sleep(Duration::from_secs(3)).await;
    manager.cleanup_old_tasks();
    {
        let tasks = manager.tasks.lock().unwrap();
        assert!(tasks.contains_key(&id), "Task should still exist after 3s");
    }

    // Should be cleaned up after 6 seconds
    tokio::time::sleep(Duration::from_secs(3)).await;
    manager.cleanup_old_tasks();
    {
        let tasks = manager.tasks.lock().unwrap();
        assert!(
            !tasks.contains_key(&id),
            "Task should be cleaned up after 6s"
        );
    }
}

#[tokio::test]
async fn test_failed_task_cleanup_timing() {
    let manager = test_manager();
    let sem = test_semaphore();

    // Create and fail a task
    let queued = manager.add_task(TaskType::DownloadVideo, "Failed Cleanup Test".into());
    let id = queued.id().to_string();
    let active = queued.start(sem).await;
    active.mark_failed("Test failure".to_string());

    // Should exist after 25 seconds (cleanup is at 30s for failed)
    tokio::time::sleep(Duration::from_secs(25)).await;
    manager.cleanup_old_tasks();
    {
        let tasks = manager.tasks.lock().unwrap();
        assert!(
            tasks.contains_key(&id),
            "Failed task should still exist after 25s"
        );
    }

    // Should be cleaned up after 35 seconds total
    tokio::time::sleep(Duration::from_secs(10)).await;
    manager.cleanup_old_tasks();
    {
        let tasks = manager.tasks.lock().unwrap();
        assert!(
            !tasks.contains_key(&id),
            "Failed task should be cleaned up after 35s"
        );
    }
}
