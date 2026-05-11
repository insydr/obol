use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::error::{CharonError, Result};
use crate::utils::retry::retry_with_backoff;

/// Signal emitted by any signal source.  Carries enough information for
/// the pipeline to decide whether to create / update a candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEvent {
    pub source: String,
    pub mint: String,
    pub payload: Option<serde_json::Value>,
}

/// API response shape from the Charon signal server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalServerResponse {
    pub signals: Vec<SignalServerItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalServerItem {
    pub mint: String,
    pub source: String,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// HTTP client for the Charon signal server.  Polls on a configurable
/// interval and emits [`SignalEvent`]s through a channel.
pub struct SignalServerClient {
    http: Client,
    base_url: String,
    api_key: String,
    poll_ms: u64,
}

impl SignalServerClient {
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

    /// Poll the signal server once and return a batch of signal events.
    pub async fn poll(&self) -> Result<Vec<SignalEvent>> {
        retry_with_backoff(3, || async {
            let url = format!("{}/signals", self.base_url);
            let resp = self
                .http
                .get(&url)
                .header("X-API-Key", &self.api_key)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(CharonError::SignalFetchFailed {
                    status: status.as_u16(),
                    body,
                });
            }

            let data: SignalServerResponse = resp.json().await?;
            let events: Vec<SignalEvent> = data
                .signals
                .into_iter()
                .map(|item| SignalEvent {
                    source: item.source,
                    mint: item.mint,
                    payload: Some(item.extra),
                })
                .collect();
            Ok(events)
        })
        .await
    }

    /// Spawn a long-running polling loop that emits events to `tx`.
    pub fn spawn_polling_loop(self: Arc<Self>, tx: mpsc::Sender<SignalEvent>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(self.poll_ms));
            loop {
                interval.tick().await;
                match self.poll().await {
                    Ok(events) => {
                        for event in events {
                            if tx.send(event).await.is_err() {
                                tracing::warn!("Signal channel closed, stopping poll loop");
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Signal server poll failed: {}", e);
                    }
                }
            }
        });
    }
}
