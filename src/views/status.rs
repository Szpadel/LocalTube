use loco_rs::prelude::*;
use serde::Serialize;

use crate::job_tracking::{
    metrics::{AllMetrics, TaskMetrics},
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

    format::render().view(
        v,
        "status/index.html",
        data!({
            "gluetun_enabled": metrics.gluetun_enabled,
            "tasks": tasks,
            "download_metrics": download_metrics,
        }),
    )
}
