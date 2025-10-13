use loco_rs::prelude::*;
use std::{env, time::Duration};

pub(crate) const CONTROL_ADDR_ENV: &str = "LOCALTUBE_GLUETUN_CONTROL_ADDR";

#[derive(Debug, Clone)]
pub struct GluetunConfig {
    pub(crate) base_url: String,
    pub(crate) poll_attempts: u8,
    pub(crate) poll_interval: Duration,
}

impl GluetunConfig {
    #[allow(clippy::result_large_err)]
    ///
    /// # Errors
    ///
    /// Returns an error if reading the `LOCALTUBE_GLUETUN_CONTROL_ADDR` environment variable fails.
    pub fn from_env() -> Result<Option<Self>> {
        let addr = match env::var(CONTROL_ADDR_ENV) {
            Ok(val) if !val.trim().is_empty() => val.trim().to_owned(),
            _ => return Ok(None),
        };

        let base_url = if addr.starts_with("http://") || addr.starts_with("https://") {
            addr
        } else {
            format!("http://{addr}")
        };

        Ok(Some(Self {
            base_url,
            poll_attempts: 5,
            poll_interval: Duration::from_secs(1),
        }))
    }

    pub(crate) fn status_url(&self) -> String {
        format!("{}/v1/vpn/status", self.base_url)
    }
}
