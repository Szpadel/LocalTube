use serde::Serialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crate::job_tracking::task::TaskType;

pub const MAX_CONSECUTIVE_FAILURES_BEFORE_RESTART: u64 = 3;
pub const MIN_SUCCESS_AGE_BEFORE_RESTART: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub(crate) struct RestartMetrics {
    pub(crate) count: u64,
    pub(crate) last_started: Option<Instant>,
    pub(crate) last_completed: Option<Instant>,
    pub(crate) last_outcome: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) in_progress: bool,
}

#[derive(Default)]
pub(crate) struct TaskMetricData {
    pub(crate) success: u64,
    pub(crate) failure: u64,
    pub(crate) consecutive_failures: u64,
    pub(crate) last_success: Option<Instant>,
    pub(crate) last_failure: Option<Instant>,
    pub(crate) restart: RestartMetrics,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMetrics {
    pub success_count: u64,
    pub failure_count: u64,
    pub consecutive_failures: u64,
    pub last_success_seconds_ago: Option<u64>,
    pub last_failure_seconds_ago: Option<u64>,
    pub restart_count: u64,
    pub last_restart_seconds_ago: Option<u64>,
    pub last_restart_outcome: Option<String>,
    pub last_restart_error: Option<String>,
    pub restart_in_progress: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllMetrics {
    pub tasks: HashMap<TaskType, TaskMetrics>,
    pub gluetun_enabled: bool,
}
