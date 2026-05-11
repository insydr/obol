use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::config::AppConfig;
use crate::error::Result;
use crate::signals::SignalEvent;

/// Fee claim event from the Pump.fun WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeClaimEvent {
    pub mint: String,
    pub claim_type: Option<String>,
    pub slot: Option<i64>,
    pub amount_lamports: Option<i64>,
}

/// Connects to the Pump.fun fee-claim WebSocket and streams fee claim
/// events as [`SignalEvent`]s.  Reconnects automatically on disconnect
/// with exponential backoff.
pub struct FeeClaimListener {
    ws_url: String,
}

impl FeeClaimListener {
    pub fn new(config: &AppConfig) -> Self {
        // The signal server may provide a WS endpoint, or we use a
        // well-known Pump.fun WS URL.
        let ws_url = format!("{}/ws/fee-claims", config.signal_server_url)
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        Self { ws_url }
    }

    /// Connect and stream events.  Reconnects with exponential backoff on
    /// errors, starting at 1s and capping at 60s.
    pub async fn listen(&self, tx: mpsc::Sender<SignalEvent>) -> Result<()> {
        let mut backoff_secs: u64 = 1;

        loop {
            match connect_async(&self.ws_url).await {
                Ok((ws_stream, _)) => {
                    tracing::info!("Connected to fee-claim WebSocket");
                    // Reset backoff on successful connection.
                    backoff_secs = 1;

                    let (_, mut read) = ws_stream.split();

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                if let Ok(claim) = serde_json::from_str::<FeeClaimEvent>(&text) {
                                    let event = SignalEvent {
                                        source: "fee_claim".to_string(),
                                        mint: claim.mint.clone(),
                                        payload: Some(serde_json::to_value(&claim).unwrap_or_default()),
                                    };
                                    if tx.send(event).await.is_err() {
                                        return Ok(());
                                    }
                                }
                            }
                            Ok(Message::Ping(_data)) => {
                                tracing::trace!("WS ping received");
                            }
                            Ok(Message::Close(_)) => {
                                tracing::warn!("Fee-claim WebSocket closed, reconnecting...");
                                break;
                            }
                            Err(e) => {
                                tracing::error!("Fee-claim WS read error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to connect fee-claim WS: {}", e);
                }
            }

            // Exponential backoff before reconnect, capped at 60s.
            tracing::info!(
                backoff_secs = backoff_secs,
                "Reconnecting fee-claim WebSocket after backoff"
            );
            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(60);
        }
    }

    pub fn spawn(self: Arc<Self>, tx: mpsc::Sender<SignalEvent>) {
        tokio::spawn(async move {
            if let Err(e) = self.listen(tx).await {
                tracing::error!("Fee-claim listener exited with error: {}", e);
            }
        });
    }
}
