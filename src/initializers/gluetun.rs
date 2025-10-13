use async_trait::async_trait;
use axum::Router as AxumRouter;
use loco_rs::{
    app::{AppContext, Initializer},
    Result,
};
use std::sync::Arc;
use tracing::info;

use crate::gluetun::{
    config::GluetunConfig,
    controller::{GluetunController, HttpGluetunController},
    supervisor,
};
use crate::job_tracking::manager::TaskManager;

pub struct GluetunInitializer;

#[async_trait]
impl Initializer for GluetunInitializer {
    fn name(&self) -> String {
        "gluetun-integration".to_string()
    }

    async fn after_routes(&self, router: AxumRouter, _ctx: &AppContext) -> Result<AxumRouter> {
        if let Some(config) = GluetunConfig::from_env()? {
            let controller: Arc<dyn GluetunController> =
                Arc::new(HttpGluetunController::new(config)?);
            supervisor::activate(&controller, TaskManager::global());
            info!("Gluetun integration enabled");
        } else {
            supervisor::deactivate(TaskManager::global());
            info!("Gluetun integration disabled");
        }

        Ok(router)
    }
}
