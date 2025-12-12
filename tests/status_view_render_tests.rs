use std::collections::HashMap;

use axum::body;
use loco_rs::{controller::views::engines, prelude::*};
use tokio::runtime::Runtime;

use localtube::{
    job_tracking::{
        metrics::{
            AllMetrics, TaskMetrics, MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART,
            MIN_SUCCESS_AGE_BEFORE_RESTART,
        },
        task::TaskType,
    },
    views,
};

#[test]
fn renders_status_without_download_metrics() {
    let view_engine = engines::TeraView::build().expect("TeraView build should succeed");
    let metrics = AllMetrics {
        tasks: HashMap::new(),
        gluetun_enabled: false,
    };

    let response = views::status::show(&view_engine, &metrics)
        .expect("Rendering status view without download metrics should succeed")
        .into_response();

    assert!(
        response.status().is_success(),
        "Rendering response should be successful"
    );
}

#[test]
fn renders_status_with_download_metrics() {
    let mut tasks = HashMap::new();
    tasks.insert(
        TaskType::DownloadVideo,
        TaskMetrics {
            success_count: 1,
            failure_count: 0,
            consecutive_failures: 0,
            last_success_seconds_ago: Some(30),
            last_failure_seconds_ago: None,
            restart_count: 0,
            last_restart_seconds_ago: None,
            last_restart_outcome: None,
            last_restart_error: None,
            restart_in_progress: false,
        },
    );

    let metrics = AllMetrics {
        tasks,
        gluetun_enabled: true,
    };

    let view_engine = engines::TeraView::build().expect("TeraView build should succeed");
    let response = views::status::show(&view_engine, &metrics)
        .expect("Rendering status view with download metrics should succeed")
        .into_response();

    let runtime = Runtime::new().expect("tokio runtime should be created");
    let body_bytes = runtime
        .block_on(body::to_bytes(response.into_body(), usize::MAX))
        .expect("Converting response body into bytes should succeed");
    let body = String::from_utf8(body_bytes.to_vec()).expect("Body should be valid UTF-8");

    assert!(
        body.contains("System Status"),
        "Response body should contain the header indicating successful render"
    );
    assert!(
        body.contains("Restart VPN"),
        "Response body should include the manual VPN restart control when enabled"
    );

    let min_success_age_minutes = MIN_SUCCESS_AGE_BEFORE_RESTART.as_secs().div_ceil(60);
    assert!(
        body.contains(&format!(
            "after {} consecutive failures",
            MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART
        )),
        "Response body should include the dynamic restart threshold"
    );
    assert!(
        body.contains(&format!(
            "{min_success_age_minutes} minutes without success"
        )),
        "Response body should include the dynamic restart gate duration"
    );
}
