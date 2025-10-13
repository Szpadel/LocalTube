use async_trait::async_trait;
use axum::{http::StatusCode, response::IntoResponse, routing::get, Router as AxumRouter};
use loco_rs::{
    app::{AppContext, Initializer},
    Result,
};

use crate::job_tracking::manager::{start_cleanup_task, TaskManager};
use crate::ws::ws_handler;

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "Status API is working")
}

// Removed unused fallback function

pub struct WebSocketStatusInitializer;

#[async_trait]
impl Initializer for WebSocketStatusInitializer {
    fn name(&self) -> String {
        "websocket-status".to_string()
    }

    async fn after_routes(&self, router: AxumRouter, _ctx: &AppContext) -> Result<AxumRouter> {
        // Start the cleanup task now that the Tokio runtime is fully initialized
        start_cleanup_task(TaskManager::global().clone());

        let router = router.route("/ws/status", get(ws_handler));
        let router = router.route("/ws/health", get(health_check));

        Ok(router)
    }
}
