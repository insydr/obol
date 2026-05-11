use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;

use crate::error::{CharonError, Result};

/// Trading execution mode — mirrors the original Node.js three-mode system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingMode {
    DryRun,
    Confirm,
    Live,
}

impl std::fmt::Display for TradingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradingMode::DryRun => write!(f, "dry_run"),
            TradingMode::Confirm => write!(f, "confirm"),
            TradingMode::Live => write!(f, "live"),
        }
    }
}

impl std::str::FromStr for TradingMode {
    type Err = CharonError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dry_run" | "dryrun" => Ok(TradingMode::DryRun),
            "confirm" => Ok(TradingMode::Confirm),
            "live" => Ok(TradingMode::Live),
            _ => Err(CharonError::Config(format!(
                "Invalid trading mode: '{}'. Expected dry_run, confirm, or live",
                s
            ))),
        }
    }
}

/// Top-level application configuration, loaded from environment variables and
/// validated at startup. Strategy-specific thresholds are stored in SQLite and
/// hot-reloaded at runtime.
#[derive(Debug, Clone)]
pub struct AppConfig {
    // ── Signal sources ──────────────────────────────────────────────────
    pub signal_server_url: String,
    pub signal_server_key: String,
    pub signal_poll_ms: u64,

    // ── Execution ───────────────────────────────────────────────────────
    pub trading_mode: TradingMode,
    pub solana_private_key_bs58: String,
    pub solana_rpc_url: String,
    pub jupiter_api_key: Option<String>,
    pub jupiter_base_url: String,
    pub live_min_sol_reserve: f64,
    pub default_buy_sol: f64,
    pub slippage_bps: u32,

    // ── LLM ─────────────────────────────────────────────────────────────
    pub enable_llm: bool,
    pub llm_base_url: String,
    pub llm_api_key: Option<String>,
    pub llm_model: String,
    pub llm_candidate_pick_count: usize,
    pub llm_timeout_secs: u64,

    // ── Telegram ────────────────────────────────────────────────────────
    pub telegram_bot_token: String,
    pub telegram_chat_id: i64,

    // ── Database ────────────────────────────────────────────────────────
    pub database_path: String,

    // ── Position management ─────────────────────────────────────────────
    pub position_monitor_ms: u64,
    pub default_tp_percent: f64,
    pub default_sl_percent: f64,
    pub trailing_stop_percent: Option<f64>,
    pub trailing_stop_activated_at_percent: Option<f64>,

    // ── Enrichment / rate-limits ────────────────────────────────────────
    pub gmgn_rate_limit_per_sec: u32,
    pub jupiter_rate_limit_per_sec: u32,

    // ── Misc ────────────────────────────────────────────────────────────
    pub log_level: String,
}

impl AppConfig {
    /// Load configuration from environment variables, falling back to
    /// sensible defaults where appropriate.  Calls [`Self::validate`]
    /// before returning.
    pub fn from_env() -> Result<Self> {
        let trading_mode_str = env_or("TRADING_MODE", "dry_run");
        let trading_mode = trading_mode_str.parse::<TradingMode>()?;

        let config = Self {
            signal_server_url: env_or("SIGNAL_SERVER_URL", "https://api.thecharon.xyz/api".into()),
            signal_server_key: env_required("SIGNAL_SERVER_KEY")?,
            signal_poll_ms: env_or_parse("SIGNAL_POLL_MS", 30_000)?,

            trading_mode,
            solana_private_key_bs58: env_required("SOLANA_PRIVATE_KEY")?,
            solana_rpc_url: env_or("SOLANA_RPC_URL", "https://api.mainnet-beta.solana.com".into()),
            jupiter_api_key: env_optional("JUPITER_API_KEY"),
            jupiter_base_url: env_or(
                "JUPITER_BASE_URL",
                "https://ultra-api.jup.ag".into(),
            ),
            live_min_sol_reserve: env_or_parse("LIVE_MIN_SOL_RESERVE", 0.02)?,
            default_buy_sol: env_or_parse("DEFAULT_BUY_SOL", 0.1)?,
            slippage_bps: env_or_parse("SLIPPAGE_BPS", 500)?,

            enable_llm: env_or_parse("ENABLE_LLM", false)?,
            llm_base_url: env_or("LLM_BASE_URL", "https://api.minimax.io/v1".into()),
            llm_api_key: env_optional("LLM_API_KEY"),
            llm_model: env_or("LLM_MODEL", "MiniMax-M2.7".into()),
            llm_candidate_pick_count: env_or_parse("LLM_CANDIDATE_PICK_COUNT", 10)?,
            llm_timeout_secs: env_or_parse("LLM_TIMEOUT_SECS", 30)?,

            telegram_bot_token: env_required("TELEGRAM_BOT_TOKEN")?,
            telegram_chat_id: env_required("TELEGRAM_CHAT_ID")?,

            database_path: env_or("DATABASE_PATH", "charon.db".into()),

            position_monitor_ms: env_or_parse("POSITION_MONITOR_MS", 10_000)?,
            default_tp_percent: env_or_parse("DEFAULT_TP_PERCENT", 100.0)?,
            default_sl_percent: env_or_parse("DEFAULT_SL_PERCENT", 30.0)?,
            trailing_stop_percent: env_or_parse_opt("TRAILING_STOP_PERCENT"),
            trailing_stop_activated_at_percent: env_or_parse_opt("TRAILING_STOP_ACTIVATED_AT_PERCENT"),

            gmgn_rate_limit_per_sec: env_or_parse("GMGN_RATE_LIMIT_PER_SEC", 2)?,
            jupiter_rate_limit_per_sec: env_or_parse("JUPITER_RATE_LIMIT_PER_SEC", 5)?,

            log_level: env_or("LOG_LEVEL", "info".into()),
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration consistency and fail fast on impossible
    /// combinations.
    fn validate(&self) -> Result<()> {
        if self.trading_mode == TradingMode::Live && self.jupiter_api_key.is_none() {
            tracing::warn!(
                "Running in LIVE mode without JUPITER_API_KEY — swaps may be rate-limited"
            );
        }
        if self.enable_llm && self.llm_api_key.is_none() {
            tracing::warn!("LLM is enabled but LLM_API_KEY is not set — requests may fail");
        }
        if self.live_min_sol_reserve < 0.0 {
            return Err(CharonError::Config(
                "LIVE_MIN_SOL_RESERVE must be >= 0".into(),
            ));
        }
        if self.default_buy_sol <= 0.0 {
            return Err(CharonError::Config(
                "DEFAULT_BUY_SOL must be > 0".into(),
            ));
        }
        if self.slippage_bps > 10_000 {
            return Err(CharonError::Config(
                "SLIPPAGE_BPS must be <= 10000 (100%)".into(),
            ));
        }
        Ok(())
    }
}

// ── Env helpers ────────────────────────────────────────────────────────────

fn env_required(key: &str) -> Result<String> {
    env::var(key).map_err(|_| CharonError::Config(format!("Required env var {} is not set", key)))
}

fn env_optional(key: &str) -> Option<String> {
    env::var(key).ok()
}

fn env_or(key: &str, default: String) -> String {
    env::var(key).unwrap_or(default)
}

fn env_or_parse<T: std::str::FromStr>(key: &str, default: T) -> Result<T> {
    match env::var(key) {
        Ok(val) => val
            .parse::<T>()
            .map_err(|_| CharonError::Config(format!("Cannot parse env var {}", key))),
        Err(_) => Ok(default),
    }
}

fn env_or_parse_opt<T: std::str::FromStr>(key: &str) -> Option<T> {
    env::var(key).ok().and_then(|v| v.parse().ok())
}
