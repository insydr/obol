use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::error::Result;
use crate::signals::SignalEvent;
use crate::utils::retry::retry_with_backoff;

/// GMGN trending token entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmgnTrendingToken {
    pub mint: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub rank: Option<i32>,
    pub market_cap: Option<f64>,
    pub price_change_24h: Option<f64>,
}

/// Jupiter trending token entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JupiterTrendingToken {
    pub mint: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub rank: Option<i32>,
}

/// Polls GMGN and Jupiter trending endpoints for token discovery.
pub struct TrendingPoller {
    http: Client,
    gmgn_url: String,
    jupiter_url: String,
    poll_ms: u64,
}

impl TrendingPoller {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to build HTTP client"),
            gmgn_url: "https://gmgn.ai/defi/quotation/v1/trending/sol".to_string(),
            jupiter_url: "https://api.jup.ag/trending".to_string(),
            poll_ms: config.signal_poll_ms * 2, // Trending changes less frequently
        }
    }

    pub async fn poll_gmgn(&self) -> Result<Vec<SignalEvent>> {
        retry_with_backoff(2, || async {
            let resp = self.http.get(&self.gmgn_url).send().await?;
            let data: serde_json::Value = resp.json().await?;

            let tokens = data
                .get("data")
                .and_then(|d| d.get("tokens"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            let mut events = Vec::new();
            if let serde_json::Value::Array(arr) = tokens {
                for (i, token) in arr.iter().enumerate() {
                    if let Some(mint) = token.get("address").and_then(|a| a.as_str()) {
                        events.push(SignalEvent {
                            source: "trending_gmgn".to_string(),
                            mint: mint.to_string(),
                            payload: Some(token.clone()),
                        });
                    }
                }
            }
            Ok(events)
        })
        .await
    }

    pub async fn poll_jupiter(&self) -> Result<Vec<SignalEvent>> {
        retry_with_backoff(2, || async {
            let resp = self.http.get(&self.jupiter_url).send().await?;
            let data: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
            Ok(data
                .into_iter()
                .filter_map(|token| {
                    token.get("address").and_then(|a| a.as_str()).map(|mint| {
                        SignalEvent {
                            source: "trending_jupiter".to_string(),
                            mint: mint.to_string(),
                            payload: Some(token.clone()),
                        }
                    })
                })
                .collect())
        })
        .await
    }

    pub fn spawn(self: Arc<Self>, tx: mpsc::Sender<SignalEvent>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(self.poll_ms));
            loop {
                interval.tick().await;

                // Poll both sources concurrently.
                let (gmgn_res, jup_res) = tokio::join!(self.poll_gmgn(), self.poll_jupiter());

                if let Ok(events) = gmgn_res {
                    for event in events {
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                }
                if let Ok(events) = jup_res {
                    for event in events {
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }
}
