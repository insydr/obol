use reqwest::Client;
use solana_client::rpc_config::RpcSendTransactionConfig;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    signature::Signature,
    transaction::Transaction,
};
use std::sync::Arc;
use std::time::Duration;

use crate::config::AppConfig;
use crate::error::{CharonError, Result};
use crate::execution::wallet::WalletService;
use crate::utils::retry::retry_with_backoff;

/// Jupiter Ultra swap execution router.  Handles buy and sell operations
/// with slippage protection, transaction signing, and on-chain confirmation.
pub struct SwapRouter {
    http: Client,
    jupiter_base_url: String,
    jupiter_api_key: Option<String>,
    slippage_bps: u32,
    sol_mint: String,
    wallet: Arc<WalletService>,
    confirm_timeout: Duration,
}

impl SwapRouter {
    pub fn new(config: &AppConfig, wallet: Arc<WalletService>) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build swap router HTTP client"),
            jupiter_base_url: config.jupiter_base_url.clone(),
            jupiter_api_key: config.jupiter_api_key.clone(),
            slippage_bps: config.slippage_bps,
            sol_mint: "So11111111111111111111111111111111111111112".to_string(),
            wallet,
            confirm_timeout: Duration::from_secs(45),
        }
    }

    /// Execute a buy swap: SOL → token.  Returns the transaction signature
    /// and the amount of tokens received.
    pub async fn buy(&self, token_mint: &str, sol_amount: f64) -> Result<SwapResult> {
        let lamports = sol_to_lamports(sol_amount);

        // Check wallet balance before proceeding.
        self.wallet.check_balance(lamports + sol_to_lamports(0.01)).await?;

        // Step 1: Get quote from Jupiter.
        let quote = self.get_quote(&self.sol_mint, token_mint, lamports).await?;

        // Step 2: Get swap transaction from Jupiter Ultra.
        let swap_tx = self.get_swap_transaction(&quote).await?;

        // Step 3: Sign, send, and confirm the transaction.
        let signature = self.sign_send_and_confirm(&swap_tx).await?;

        // Step 4: Parse token amount from the quote's outAmount.
        let token_amount = quote
            .get("outAmount")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok());

        tracing::info!(
            mint = token_mint,
            sol = sol_amount,
            sig = %signature,
            tokens = ?token_amount,
            "Buy transaction confirmed"
        );

        Ok(SwapResult {
            signature: signature.to_string(),
            token_amount,
        })
    }

    /// Execute a sell swap: token → SOL.
    pub async fn sell(&self, token_mint: &str, token_amount: u64) -> Result<String> {
        // Step 1: Get quote from Jupiter.
        let quote = self.get_quote(token_mint, &self.sol_mint, token_amount).await?;

        // Step 2: Get swap transaction from Jupiter Ultra.
        let swap_tx = self.get_swap_transaction(&quote).await?;

        // Step 3: Sign, send, and confirm the transaction.
        let signature = self.sign_send_and_confirm(&swap_tx).await?;

        tracing::info!(
            mint = token_mint,
            amount = token_amount,
            sig = %signature,
            "Sell transaction confirmed"
        );

        Ok(signature.to_string())
    }

    /// Get a swap quote from Jupiter.
    async fn get_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
    ) -> Result<serde_json::Value> {
        retry_with_backoff(3, || async {
            let url = format!("{}/quote", self.jupiter_base_url);
            let mut req = self.http.get(&url).query(&[
                ("inputMint", input_mint),
                ("outputMint", output_mint),
                ("amount", &amount.to_string()),
                ("slippageBps", &self.slippage_bps.to_string()),
            ]);
            if let Some(ref key) = self.jupiter_api_key {
                req = req.header("Authorization", format!("Bearer {}", key));
            }

            let resp = req.send().await?;
            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await?;
                Ok(data)
            } else {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                Err(CharonError::Execution(format!(
                    "Jupiter quote failed: {} - {}",
                    status, text
                )))
            }
        })
        .await
    }

    /// Get a signed swap transaction from Jupiter Ultra.
    async fn get_swap_transaction(&self, quote: &serde_json::Value) -> Result<String> {
        retry_with_backoff(2, || async {
            let url = format!("{}/swap", self.jupiter_base_url);
            let mut req = self.http.post(&url);
            if let Some(ref key) = self.jupiter_api_key {
                req = req.header("Authorization", format!("Bearer {}", key));
            }

            let body = serde_json::json!({
                "quoteResponse": quote,
                "userPublicKey": self.wallet.pubkey().to_string(),
                "wrapAndUnwrapSol": true,
                "dynamicComputeUnitLimit": true,
                "prioritizationFeeLamports": "auto",
            });

            let resp = req.json(&body).send().await?;
            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await?;
                let swap_transaction = data["swapTransaction"]
                    .as_str()
                    .ok_or_else(|| CharonError::Execution("No swapTransaction in response".into()))?
                    .to_string();
                Ok(swap_transaction)
            } else {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                Err(CharonError::Execution(format!(
                    "Jupiter swap failed: {} - {}",
                    status, text
                )))
            }
        })
        .await
    }

    /// Deserialize, sign, send, and confirm the base58-encoded transaction.
    async fn sign_send_and_confirm(&self, serialized_tx: &str) -> Result<Signature> {
        let tx_bytes = bs58::decode(serialized_tx)
            .into_vec()
            .map_err(|e| CharonError::Execution(format!("Failed to decode tx: {}", e)))?;

        let mut tx: Transaction =
            bincode::deserialize(&tx_bytes)
                .map_err(|e| CharonError::Execution(format!("Failed to deserialize tx: {}", e)))?;

        // Sign with our keypair.
        let keypair = self.wallet.keypair();
        tx.try_sign(&[keypair], *tx.message.recent_blockhash())
            .map_err(|e| CharonError::Execution(format!("Failed to sign tx: {}", e)))?;

        // Send the transaction via the async RPC client.
        let rpc = self.wallet.rpc_client();
        let config = RpcSendTransactionConfig {
            skip_preflight: false,
            preflight_commitment: Some(CommitmentConfig::confirmed().commitment),
            ..Default::default()
        };

        let signature = rpc
            .send_transaction_with_config(&tx, config)
            .await
            .map_err(|e| CharonError::Execution(format!("Failed to send tx: {}", e)))?;

        // Wait for on-chain confirmation with timeout.
        match tokio::time::timeout(self.confirm_timeout, async {
            // Poll for confirmation.
            loop {
                match rpc.get_signature_statuses(&[signature]).await {
                    Ok(response) => {
                        if let Some(status) = response.value.first() {
                            match status {
                                Some(s) if s.err.is_none() => return Ok(()),
                                Some(s) => {
                                    return Err(CharonError::Execution(format!(
                                        "Transaction failed on-chain: {:?}",
                                        s.err
                                    )));
                                }
                                None => {
                                    // Not yet confirmed, keep polling.
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Confirmation poll error: {}", e);
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        {
            Ok(Ok(())) => Ok(signature),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                tracing::warn!(
                    sig = %signature,
                    "Transaction sent but confirmation timed out — it may still succeed on-chain"
                );
                Ok(signature)
            }
        }
    }
}

/// Result of a swap execution containing the transaction signature and
/// optional token amount received.
#[derive(Debug, Clone)]
pub struct SwapResult {
    pub signature: String,
    pub token_amount: Option<u64>,
}

fn sol_to_lamports(sol: f64) -> u64 {
    (sol * 1_000_000_000.0) as u64
}
