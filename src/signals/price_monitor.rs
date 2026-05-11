use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::error::Result;
use crate::signals::SignalEvent;
use crate::utils::retry::retry_with_backoff;

/// Price dip alert configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DipAlert {
    pub mint: String,
    pub current_price_sol: f64,
    pub dip_percent: f64,
    pub from_ath_percent: f64,
}

/// Monitors token prices and emits dip-buy signals when a token
/// drops significantly from its ATH — a potential buy opportunity.
pub struct PriceMonitor {
    http: Client,
    gmgn_base_url: String,
    poll_ms: u64,
    dip_threshold_pct: f64,
}

impl PriceMonitor {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to build HTTP client"),
            gmgn_base_url: "https://gmgn.ai/defi/quotation/v1".to_string(),
            poll_ms: config.signal_poll_ms * 3,
            dip_threshold_pct: 40.0, // Default: 40% dip from ATH triggers alert
        }
    }

    /// Check tracked tokens for price dips.
    pub async fn check_dips(&self, tracked_mints: &[String]) -> Result<Vec<SignalEvent>> {
        let mut events = Vec::new();
        for mint in tracked_mints {
            match self.fetch_token_price(mint).await {
                Ok(Some(price_data)) => {
                    if let Some(dip_pct) = price_data.ath_dip_percent {
                        if dip_pct >= self.dip_threshold_pct {
                            events.push(SignalEvent {
                                source: "price_dip".to_string(),
                                mint: mint.clone(),
                                payload: Some(serde_json::to_value(&price_data).unwrap_or_default()),
                            });
                        }
                    }
                }
                Ok(None) => {
                    tracing::debug!("No price data for mint: {}", mint);
                }
                Err(e) => {
                    tracing::warn!("Price check failed for {}: {}", mint, e);
                }
            }
        }
        Ok(events)
    }

    async fn fetch_token_price(&self, mint: &str) -> Result<Option<TokenPriceData>> {
        retry_with_backoff(1, || async {
            let url = format!("{}/tokens/sol/{}", self.gmgn_base_url, mint);
            let resp = self.http.get(&url).send().await?;

            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await?;
                let token = data.get("data").and_then(|d| d.get("token"));
                if let Some(t) = token {
                    Ok(Some(TokenPriceData {
                        price_sol: t.get("price").and_then(|p| p.as_f64()).unwrap_or(0.0),
                        market_cap_sol: t.get("market_cap").and_then(|p| p.as_f64()),
                        ath_dip_percent: t.get("ath_dip_percent").and_then(|p| p.as_f64()),
                    }))
                } else {
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        })
        .await
    }

    pub fn spawn(self: Arc<Self>, tx: mpsc::Sender<SignalEvent>, tracked_mints: Vec<String>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(self.poll_ms));
            loop {
                interval.tick().await;
                match self.check_dips(&tracked_mints).await {
                    Ok(events) => {
                        for event in events {
                            if tx.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Price monitor check failed: {}", e);
                    }
                }
            }
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPriceData {
    pub price_sol: f64,
    pub market_cap_sol: Option<f64>,
    pub ath_dip_percent: Option<f64>,
}
