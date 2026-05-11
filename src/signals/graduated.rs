use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::error::Result;
use crate::signals::SignalEvent;
use crate::utils::retry::retry_with_backoff;

/// Response shape from the graduated-token endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduatedToken {
    pub mint: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub market_cap_sol: Option<f64>,
    pub graduated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduatedResponse {
    pub tokens: Vec<GraduatedToken>,
}

/// Polls for Pump.fun tokens that have graduated (bonding curve complete)
/// and emits them as signal events.
pub struct GraduatedPoller {
    http: Client,
    base_url: String,
    api_key: String,
    poll_ms: u64,
}

impl GraduatedPoller {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("Failed to build HTTP client"),
            base_url: config.signal_server_url.clone(),
            api_key: config.signal_server_key.clone(),
            poll_ms: config.signal_poll_ms,
        }
    }

    pub async fn poll(&self) -> Result<Vec<SignalEvent>> {
        retry_with_backoff(3, || async {
            let url = format!("{}/graduated", self.base_url);
            let resp = self
                .http
                .get(&url)
                .header("X-API-Key", &self.api_key)
                .send()
                .await?;

            let data: GraduatedResponse = resp.json().await?;
            Ok(data
                .tokens
                .into_iter()
                .map(|t| SignalEvent {
                    source: "graduated".to_string(),
                    mint: t.mint,
                    payload: Some(serde_json::to_value(&t).unwrap_or_default()),
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
                match self.poll().await {
                    Ok(events) => {
                        for event in events {
                            if tx.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Graduated poll failed: {}", e);
                    }
                }
            }
        });
    }
}
