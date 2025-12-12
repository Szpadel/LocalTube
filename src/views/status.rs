use loco_rs::prelude::*;
use serde::Serialize;

use crate::job_tracking::{
    metrics::{
        AllMetrics, TaskMetrics, MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART,
        MIN_SUCCESS_AGE_BEFORE_RESTART,
    },
    task::TaskType,
};

#[derive(Serialize)]
struct TaskEntry {
    name: String,
    metrics: TaskMetrics,
}

#[allow(clippy::result_large_err)]
///
/// # Errors
///
/// Returns an error if rendering the status template fails.
pub fn show(v: &impl ViewRenderer, metrics: &AllMetrics) -> Result<Response> {
    let mut tasks: Vec<TaskEntry> = metrics
        .tasks
        .iter()
        .map(|(task_type, data)| TaskEntry {
            name: task_type.as_str().to_string(),
            metrics: data.clone(),
        })
        .collect();
    tasks.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));

    let download_metrics = metrics.tasks.get(&TaskType::DownloadVideo).cloned();

    let min_success_age_minutes = MIN_SUCCESS_AGE_BEFORE_RESTART.as_secs().div_ceil(60);

    format::render().view(
        v,
        "status/index.html",
        data!({
            "gluetun_enabled": metrics.gluetun_enabled,
            "gluetun_restart_failure_threshold": MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART,
            "gluetun_restart_min_success_age_minutes": min_success_age_minutes,
            "tasks": tasks,
            "download_metrics": download_metrics,
        }),
    )
}

#[allow(clippy::result_large_err)]
///
/// # Errors
///
/// Returns an error if rendering the restart result template fails.
pub fn restart_result(v: &impl ViewRenderer, kind: &str, message: &str) -> Result<Response> {
    format::render().view(
        v,
        "status/restart_result.html",
        data!({
            "kind": kind,
            "message": message,
        }),
    )
}
