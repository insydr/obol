use reqwest::Client;
use std::time::Duration;

use crate::config::AppConfig;
use crate::error::{CharonError, Result};
use crate::utils::retry::retry_with_backoff;

/// Jupiter API client for token info and swap pricing.
pub struct JupiterClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl JupiterClient {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("Failed to build Jupiter HTTP client"),
            base_url: config.jupiter_base_url.clone(),
            api_key: config.jupiter_api_key.clone(),
        }
    }

    /// Fetch token information from Jupiter.
    pub async fn fetch_token_info(&self, mint: &str) -> Result<Option<serde_json::Value>> {
        retry_with_backoff(2, || async {
            let url = format!("{}/token/{}", self.base_url, mint);
            let mut req = self.http.get(&url);
            if let Some(ref key) = self.api_key {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            let resp = req.send().await?;

            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await?;
                Ok(Some(data))
            } else if resp.status().as_u16() == 404 {
                Ok(None)
            } else {
                let status = resp.status();
                Err(CharonError::Enrichment(format!(
                    "Jupiter API error for {}: {}",
                    mint, status
                )))
            }
        })
        .await
    }

    /// Get a swap quote from Jupiter Ultra.
    pub async fn get_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount_lamports: u64,
        slippage_bps: u32,
    ) -> Result<serde_json::Value> {
        retry_with_backoff(2, || async {
            let url = format!("{}/quote", self.base_url);
            let mut req = self.http.get(&url).query(&[
                ("inputMint", input_mint),
                ("outputMint", output_mint),
                ("amount", &amount_lamports.to_string()),
                ("slippageBps", &slippage_bps.to_string()),
            ]);
            if let Some(ref key) = self.api_key {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            let resp = req.send().await?;

            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await?;
                Ok(data)
            } else {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                Err(CharonError::Enrichment(format!(
                    "Jupiter quote error: {} - {}",
                    status, text
                )))
            }
        })
        .await
    }
}
