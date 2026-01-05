use async_trait::async_trait;
use loco_rs::prelude::*;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::fmt;
use tracing::{debug, info, warn};

use super::config::GluetunConfig;

#[derive(Debug, Clone, Deserialize)]
struct VpnStatusResponse {
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct VpnStatusChangeResponse {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
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
            .build()
            .map_err(|err| loco_rs::Error::Any(Box::new(err)))?;

        Ok(Self { client, config })
    }

    async fn send_status_change(
        &self,
        status: &'static str,
    ) -> Result<Option<String>, GluetunError> {
        let url = self.config.status_url();
        debug!(%url, %status, "Sending Gluetun VPN status change");
        let response = self
            .client
            .put(&url)
            .json(&serde_json::json!({ "status": status }))
            .send()
            .await?;

        if !response.status().is_success() {
            let http_status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            warn!(
                %url,
                %status,
                %http_status,
                body = %body,
                "Gluetun status change request failed"
            );
            return Err(GluetunError::UnexpectedStatus(http_status));
        }

        let body = response.json::<VpnStatusChangeResponse>().await?;
        debug!(
            %url,
            %status,
            returned_status = ?body.status,
            outcome = ?body.outcome,
            "Gluetun status change acknowledged"
        );
        Ok(body.outcome)
    }

    async fn poll_until(&self, desired: &'static str) -> Result<(), GluetunError> {
        let url = self.config.status_url();
        for attempt in 1..=self.config.poll_attempts {
            debug!(%url, %desired, attempt, "Polling Gluetun VPN status");
            let response = self.client.get(&url).send().await?;
            if !response.status().is_success() {
                let http_status = response.status();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<unreadable body>".to_string());
                warn!(
                    %url,
                    %desired,
                    attempt,
                    %http_status,
                    body = %body,
                    "Gluetun status poll request failed"
                );
                return Err(GluetunError::UnexpectedStatus(http_status));
            }
            let body = response.json::<VpnStatusResponse>().await?;
            debug!(
                %url,
                %desired,
                attempt,
                status = %body.status,
                "Gluetun VPN status poll response"
            );
            if body.status == desired {
                return Ok(());
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
        warn!(%url, %desired, "Timed out polling Gluetun VPN status");
        Err(GluetunError::PollTimeout)
    }
}

#[async_trait]
impl GluetunController for HttpGluetunController {
    async fn restart(&self) -> std::result::Result<GluetunRestartOutcome, GluetunError> {
        let started_at = std::time::Instant::now();
        info!("Starting Gluetun VPN restart sequence");
        let stop_outcome = self.send_status_change("stopped").await?;
        self.poll_until("stopped").await?;

        let start_outcome = self.send_status_change("running").await?;
        self.poll_until("running").await?;

        info!(
            elapsed_ms = started_at.elapsed().as_millis(),
            stop_outcome = ?stop_outcome,
            start_outcome = ?start_outcome,
            "Gluetun VPN restart sequence finished"
        );
        Ok(GluetunRestartOutcome {
            stop_outcome,
            start_outcome,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpn_status_change_response_deserializes_outcome_only_payload() {
        let body: VpnStatusChangeResponse =
            serde_json::from_str(r#"{"outcome":"set to stopped"}"#).expect("deserialize");

        assert_eq!(body.status.as_deref(), None);
        assert_eq!(body.outcome.as_deref(), Some("set to stopped"));
    }

    #[test]
    fn vpn_status_change_response_deserializes_status_and_outcome_payload() {
        let body: VpnStatusChangeResponse =
            serde_json::from_str(r#"{"status":"stopped","outcome":"ok"}"#).expect("deserialize");

        assert_eq!(body.status.as_deref(), Some("stopped"));
        assert_eq!(body.outcome.as_deref(), Some("ok"));
    }

    #[test]
    fn vpn_status_response_deserializes_status_payload() {
        let body: VpnStatusResponse =
            serde_json::from_str(r#"{"status":"running"}"#).expect("deserialize status response");

        assert_eq!(body.status, "running");
    }
}
