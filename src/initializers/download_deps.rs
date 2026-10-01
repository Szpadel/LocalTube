use std::time::Duration;

use async_trait::async_trait;
use loco_rs::{
    app::{AppContext, Initializer},
    environment::Environment,
    Error, Result,
};
use tracing::{info, warn};

use crate::ytdlp;

// YouTube changes often break older yt-dlp releases, so the app checks for a new release every day.
const YT_DLP_UPDATE_INTERVAL: Duration = Duration::from_hours(24);

/// Installs a missing yt-dlp binary and keeps it at the latest release.
pub struct DownloadDeps;

#[async_trait]
impl Initializer for DownloadDeps {
    fn name(&self) -> String {
        "download-deps".to_string()
    }

    async fn before_run(&self, app_context: &AppContext) -> Result<()> {
        // The loco test boot also runs `before_run`. Tests must not change
        // `libs/` or need network access, and no test runs yt-dlp.
        if app_context.environment == Environment::Test {
            return Ok(());
        }

        ytdlp::install_yt_dlp().await.map_err(Error::msg)?;

        // Update before the first source refresh starts, so that it uses the new release.
        update_yt_dlp_best_effort().await;
        tokio::spawn(async {
            loop {
                tokio::time::sleep(YT_DLP_UPDATE_INTERVAL).await;
                update_yt_dlp_best_effort().await;
            }
        });

        Ok(())
    }
}

/// Updates yt-dlp and logs the result.
///
/// A failed update is not fatal: the installed release stays in use until the next attempt.
async fn update_yt_dlp_best_effort() {
    match ytdlp::update_yt_dlp().await {
        Ok(result) => info!(%result, "yt-dlp update check finished"),
        Err(error) => warn!(%error, "yt-dlp update failed, so the installed release stays in use"),
    }
}
