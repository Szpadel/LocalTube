#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unused_async)]

use axum::debug_handler;
use loco_rs::prelude::*;
use tracing::{error, info};

use crate::{job_tracking::manager::TaskManager, views};

#[debug_handler]
pub async fn show(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    let metrics = TaskManager::global().get_metrics();
    views::status::show(&v, &metrics)
}

#[debug_handler]
pub async fn restart_gluetun(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    let task_manager = TaskManager::global();

    if !task_manager.gluetun_enabled() {
        info!("Manual Gluetun VPN restart requested but integration is disabled");
        return views::status::restart_result(
            &v,
            "error",
            "Gluetun integration is disabled. Set LOCALTUBE_GLUETUN_CONTROL_ADDR to enable it.",
        );
    }

    let Some(controller) = crate::gluetun::supervisor::controller() else {
        error!("Manual Gluetun VPN restart requested but controller is missing");
        return views::status::restart_result(
            &v,
            "error",
            "Gluetun controller is not available. Check server logs.",
        );
    };

    if !task_manager.begin_gluetun_restart(None) {
        info!("Manual Gluetun VPN restart requested but a restart is already in progress");
        return views::status::restart_result(&v, "warning", "VPN restart is already in progress.");
    }

    info!("Manual Gluetun VPN restart accepted");
    let task_manager = task_manager.clone();
    tokio::spawn(async move {
        let outcome = controller.restart().await;
        match &outcome {
            Ok(result) => info!("Manual Gluetun VPN restart succeeded: {result}"),
            Err(err) => error!("Manual Gluetun VPN restart failed: {err}"),
        }
        task_manager.finish_gluetun_restart(None, &outcome);
    });

    views::status::restart_result(
        &v,
        "success",
        "VPN restart triggered. Check server logs for progress.",
    )
}

pub fn routes() -> Routes {
    Routes::new()
        .add("/status", get(show))
        .add("/status/gluetun/restart", post(restart_gluetun))
}
