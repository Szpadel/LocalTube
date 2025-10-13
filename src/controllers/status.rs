#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unused_async)]

use axum::debug_handler;
use loco_rs::prelude::*;

use crate::{job_tracking::manager::TaskManager, views};

#[debug_handler]
pub async fn show(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    let metrics = TaskManager::global().get_metrics();
    views::status::show(&v, &metrics)
}

pub fn routes() -> Routes {
    Routes::new().add("/status", get(show))
}
