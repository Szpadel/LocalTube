#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unnecessary_struct_initialization)]
#![allow(clippy::unused_async)]

use axum::debug_handler;
use loco_rs::prelude::*;

use crate::ws::TaskManager;

/// GET /metrics/ - Returns current task metrics in JSON format
#[debug_handler]
pub async fn list(State(_ctx): State<AppContext>) -> Result<Response> {
    format::json(TaskManager::global().get_metrics())
}

pub fn routes() -> Routes {
    Routes::new().prefix("metrics/").add("/", get(list))
}
