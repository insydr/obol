use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};
use std::str::FromStr;
use std::sync::Arc;

use crate::config::AppConfig;
use crate::error::{CharonError, Result};

/// Wallet service wrapping Solana keypair and RPC client.
/// Handles balance checks and transaction signing.
pub struct WalletService {
    keypair: Keypair,
    rpc: Arc<RpcClient>,
    min_reserve_lamports: u64,
}

impl WalletService {
    pub fn new(config: &AppConfig) -> Result<Self> {
        let keypair_bytes = bs58::decode(&config.solana_private_key_bs58)
            .into_vec()
            .map_err(|e| CharonError::Config(format!("Invalid SOLANA_PRIVATE_KEY: {}", e)))?;

        let keypair = Keypair::from_bytes(&keypair_bytes)
            .map_err(|e| CharonError::Config(format!("Failed to create keypair: {}", e)))?;

        let rpc = RpcClient::new(&config.solana_rpc_url);

        Ok(Self {
            keypair,
            rpc: Arc::new(rpc),
            min_reserve_lamports: (config.live_min_sol_reserve * 1_000_000_000.0) as u64,
        })
    }

    /// Get the wallet's public key.
    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    /// Get a reference to the keypair for signing.
    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    /// Get a reference to the RPC client.
    pub fn rpc_client(&self) -> &RpcClient {
        &self.rpc
    }

    /// Check that the wallet has sufficient SOL balance for a transaction.
    pub async fn check_balance(&self, needed_lamports: u64) -> Result<()> {
        let balance = self
            .rpc
            .get_balance(&self.keypair.pubkey())
            .await
            .map_err(|e| CharonError::SolanaClient(format!("Balance check failed: {}", e)))?;

        if balance < needed_lamports + self.min_reserve_lamports {
            return Err(CharonError::InsufficientBalance {
                needed: needed_lamports + self.min_reserve_lamports,
                had: balance,
            });
        }
        Ok(())
    }

    /// Get the current SOL balance in lamports.
    pub async fn get_balance(&self) -> Result<u64> {
        self.rpc
            .get_balance(&self.keypair.pubkey())
            .await
            .map_err(|e| CharonError::SolanaClient(format!("Balance check failed: {}", e)))
    }
}
