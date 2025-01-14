use localtube::app::App;
use loco_rs::{task, testing::prelude::*};

use loco_rs::boot::run_task;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_can_run_refresh_indexes() {
    let boot = boot_test::<App>().await.unwrap();

    assert!(run_task::<App>(
        &boot.app_context,
        Some(&"refresh_indexes".to_string()),
        &task::Vars::default()
    )
    .await
    .is_ok());
}
