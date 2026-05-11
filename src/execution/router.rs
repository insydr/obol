use reqwest::Client;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::config::AppConfig;
use crate::error::{CharonError, Result};
use crate::execution::wallet::WalletService;
use crate::utils::retry::retry_with_backoff;

/// Jupiter Ultra swap execution router.  Handles buy and sell operations
/// with slippage protection and transaction signing.
pub struct SwapRouter {
    http: Client,
    jupiter_base_url: String,
    jupiter_api_key: Option<String>,
    slippage_bps: u32,
    sol_mint: String,
    wallet: Arc<WalletService>,
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
        }
    }

    /// Execute a buy swap: SOL → token.
    pub async fn buy(&self, token_mint: &str, sol_amount: f64) -> Result<String> {
        let lamports = sol_to_lamports(sol_amount);

        // Check wallet balance before proceeding.
        self.wallet.check_balance(lamports + sol_to_lamports(0.01)).await?;

        // Step 1: Get quote from Jupiter.
        let quote = self.get_quote(&self.sol_mint, token_mint, lamports).await?;

        // Step 2: Get swap transaction from Jupiter Ultra.
        let swap_tx = self.get_swap_transaction(&quote).await?;

        // Step 3: Sign and send the transaction.
        let signature = self.sign_and_send(&swap_tx).await?;

        tracing::info!(
            mint = token_mint,
            sol = sol_amount,
            sig = %signature,
            "Buy transaction submitted"
        );

        Ok(signature)
    }

    /// Execute a sell swap: token → SOL.
    pub async fn sell(&self, token_mint: &str, token_amount: u64) -> Result<String> {
        // Step 1: Get quote from Jupiter.
        let quote = self.get_quote(token_mint, &self.sol_mint, token_amount).await?;

        // Step 2: Get swap transaction from Jupiter Ultra.
        let swap_tx = self.get_swap_transaction(&quote).await?;

        // Step 3: Sign and send the transaction.
        let signature = self.sign_and_send(&swap_tx).await?;

        tracing::info!(
            mint = token_mint,
            amount = token_amount,
            sig = %signature,
            "Sell transaction submitted"
        );

        Ok(signature)
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

    /// Deserialize, sign, and broadcast the base58-encoded transaction.
    async fn sign_and_send(&self, serialized_tx: &str) -> Result<String> {
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

        // Send the transaction.
        let rpc = self.wallet.rpc_client();
        let signature = rpc
            .send_transaction_with_config(
                &tx,
                CommitmentConfig::confirmed(),
            )
            .await
            .map_err(|e| CharonError::Execution(format!("Failed to send tx: {}", e)))?;

        Ok(signature.to_string())
    }
}

fn sol_to_lamports(sol: f64) -> u64 {
    (sol * 1_000_000_000.0) as u64
}
