use axum::async_trait;
use loco_rs::{
    app::{AppContext, Initializer},
    Error, Result,
};

use crate::ytdlp;

pub struct DownloadDeps;

#[async_trait]
impl Initializer for DownloadDeps {
    fn name(&self) -> String {
        "download-deps".to_string()
    }

    async fn before_run(&self, _app_context: &AppContext) -> Result<()> {
        ytdlp::download_deps().await.map_err(Error::msg)?;
        Ok(())
    }
}
