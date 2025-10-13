use async_trait::async_trait;
use loco_rs::prelude::*;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::fmt;

use super::config::GluetunConfig;

#[derive(Debug, Clone, Deserialize)]
struct StatusResponse {
    status: String,
    outcome: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GluetunRestartOutcome {
    pub stop_outcome: Option<String>,
    pub start_outcome: Option<String>,
}

impl fmt::Display for GluetunRestartOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stop_outcome={:?}, start_outcome={:?}",
            self.stop_outcome, self.start_outcome
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GluetunError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unexpected status code: {0}")]
    UnexpectedStatus(StatusCode),
    #[error("gluetun returned unexpected state: expected {expected}, got {actual}")]
    UnexpectedState {
        expected: &'static str,
        actual: String,
    },
    #[error("gluetun did not report desired state after polling")]
    PollTimeout,
}

#[async_trait]
pub trait GluetunController: Send + Sync {
    async fn restart(&self) -> std::result::Result<GluetunRestartOutcome, GluetunError>;
}

#[derive(Debug, Clone)]
pub struct HttpGluetunController {
    client: Client,
    config: GluetunConfig,
}

impl HttpGluetunController {
    #[allow(clippy::result_large_err)]
    ///
    /// # Errors
    ///
    /// Returns an error if constructing the HTTP client fails.
    pub fn new(config: GluetunConfig) -> Result<Self> {
        let client = Client::builder()
            .user_agent("localtube-gluetun-integration")
            .build()?;

        Ok(Self { client, config })
    }

    async fn send_status_change(
        &self,
        status: &'static str,
    ) -> Result<StatusResponse, GluetunError> {
        let response = self
            .client
            .put(self.config.status_url())
            .json(&serde_json::json!({ "status": status }))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(GluetunError::UnexpectedStatus(response.status()));
        }

        let body = response.json::<StatusResponse>().await?;
        if body.status != status {
            return Err(GluetunError::UnexpectedState {
                expected: status,
                actual: body.status,
            });
        }

        Ok(body)
    }

    async fn poll_until(&self, desired: &'static str) -> Result<(), GluetunError> {
        for _ in 0..self.config.poll_attempts {
            let response = self.client.get(self.config.status_url()).send().await?;
            if !response.status().is_success() {
                return Err(GluetunError::UnexpectedStatus(response.status()));
            }
            let body = response.json::<StatusResponse>().await?;
            if body.status == desired {
                return Ok(());
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
        Err(GluetunError::PollTimeout)
    }
}

#[async_trait]
impl GluetunController for HttpGluetunController {
    async fn restart(&self) -> std::result::Result<GluetunRestartOutcome, GluetunError> {
        let stop_body = self.send_status_change("stopped").await?;
        self.poll_until("stopped").await?;

        let start_body = self.send_status_change("running").await?;
        self.poll_until("running").await?;

        Ok(GluetunRestartOutcome {
            stop_outcome: stop_body.outcome,
            start_outcome: start_body.outcome,
        })
    }
}
