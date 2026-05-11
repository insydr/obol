use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

use crate::config::AppConfig;
use crate::error::{CharonError, Result};
use crate::utils::retry::retry_with_backoff;

/// GMGN API client with rate limiting awareness.
pub struct GmgnClient {
    http: Client,
    base_url: String,
    rate_limit_per_sec: u32,
}

impl GmgnClient {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("Failed to build GMGN HTTP client"),
            base_url: "https://gmgn.ai/defi/quotation/v1".to_string(),
            rate_limit_per_sec: config.gmgn_rate_limit_per_sec,
        }
    }

    /// Fetch token information from GMGN.
    pub async fn fetch_token_info(&self, mint: &str) -> Result<Option<serde_json::Value>> {
        retry_with_backoff(1, || async {
            // Rate-limit: sleep to respect GMGN's rate limits.
            let sleep_ms = 1000 / self.rate_limit_per_sec.max(1) as u64;
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

            let url = format!("{}/tokens/sol/{}", self.base_url, mint);
            let resp = self.http.get(&url).send().await?;

            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await?;
                // Extract the token object from the response wrapper.
                let token = data
                    .get("data")
                    .and_then(|d| d.get("token"))
                    .cloned()
                    .unwrap_or(data);
                Ok(Some(token))
            } else if resp.status().as_u16() == 404 {
                Ok(None)
            } else {
                let status = resp.status();
                Err(CharonError::Enrichment(format!(
                    "GMGN API error for {}: {}",
                    mint, status
                )))
            }
        })
        .await
    }

    /// Fetch holder analysis for a token.
    pub async fn fetch_holders(&self, mint: &str) -> Result<Option<serde_json::Value>> {
        retry_with_backoff(1, || async {
            let sleep_ms = 1000 / self.rate_limit_per_sec.max(1) as u64;
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

            let url = format!("{}/holders/sol/{}", self.base_url, mint);
            let resp = self.http.get(&url).send().await?;

            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await?;
                Ok(Some(data.get("data").cloned().unwrap_or(data)))
            } else {
                Ok(None)
            }
        })
        .await
    }
}
