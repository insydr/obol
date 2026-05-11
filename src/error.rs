use thiserror::Error;

/// Unified error type for the entire Charon application.
#[derive(Error, Debug)]
pub enum CharonError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Signal fetch failed: {0}")]
    SignalFetch(#[from] reqwest::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("Connection pool error: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Solana client error: {0}")]
    SolanaClient(String),

    #[error("Execution failed: {0}")]
    Execution(String),

    #[error("Insufficient balance: needed {needed} lamports, had {had} lamports")]
    InsufficientBalance { needed: u64, had: u64 },

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Telegram error: {0}")]
    Telegram(String),

    #[error("Strategy filter rejected: {0}")]
    StrategyRejected(String),

    #[error("Token enrichment failed: {0}")]
    Enrichment(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, CharonError>;
