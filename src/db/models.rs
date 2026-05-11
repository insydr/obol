use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// Newtype wrapper for Solana mint addresses — prevents accidental
/// confusion with other string data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Mint(pub String);

impl std::fmt::Display for Mint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Mint {
    fn from(s: String) -> Self {
        Mint(s)
    }
}

impl AsRef<str> for Mint {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ── Signal ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub id: i64,
    pub source: String,
    pub mint: String,
    pub payload: Option<String>,
    pub detected_at: String,
    pub processed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSignal {
    pub source: String,
    pub mint: String,
    pub payload: Option<String>,
}

// ── Candidate ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CandidateStatus {
    Pending,
    Screened,
    Approved,
    Rejected,
    Expired,
}

impl std::fmt::Display for CandidateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CandidateStatus::Pending => write!(f, "pending"),
            CandidateStatus::Screened => write!(f, "screened"),
            CandidateStatus::Approved => write!(f, "approved"),
            CandidateStatus::Rejected => write!(f, "rejected"),
            CandidateStatus::Expired => write!(f, "expired"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: i64,
    pub mint: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub uri: Option<String>,
    pub source_count: i32,
    pub sources: Option<String>,
    pub market_cap_sol: Option<f64>,
    pub market_cap_usd: Option<f64>,
    pub holder_count: Option<i32>,
    pub top_holder_pct: Option<f64>,
    pub ath_distance_pct: Option<f64>,
    pub fee_claim_count: i32,
    pub first_seen_at: String,
    pub last_updated_at: String,
    pub status: String,
    /// Buy amount from strategy at time of approval (for confirm mode).
    pub buy_sol: f64,
}

/// A candidate enriched with additional data from Jupiter/GMGN/Twitter,
/// ready for LLM screening or direct approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedCandidate {
    pub candidate: Candidate,
    pub jupiter_data: Option<serde_json::Value>,
    pub gmgn_data: Option<serde_json::Value>,
    pub twitter_data: Option<serde_json::Value>,
}

// ── Position ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PositionStatus {
    Open,
    Closed,
    Liquidated,
}

impl std::fmt::Display for PositionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PositionStatus::Open => write!(f, "open"),
            PositionStatus::Closed => write!(f, "closed"),
            PositionStatus::Liquidated => write!(f, "liquidated"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: i64,
    pub mint: String,
    pub symbol: Option<String>,
    pub entry_price: Option<f64>,
    pub current_price: Option<f64>,
    pub buy_sol: f64,
    pub token_amount: Option<f64>,
    pub pnl_percent: f64,
    pub pnl_sol: f64,
    pub tp_percent: Option<f64>,
    pub sl_percent: Option<f64>,
    pub trailing_stop_pct: Option<f64>,
    pub trailing_activated: bool,
    pub highest_pnl_pct: f64,
    pub status: String,
    pub opened_at: String,
    pub closed_at: Option<String>,
    pub close_reason: Option<String>,
    pub tx_buy_sig: Option<String>,
    pub tx_sell_sig: Option<String>,
}

/// Parameters for opening a new position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenPositionParams {
    pub mint: String,
    pub symbol: Option<String>,
    pub buy_sol: f64,
    pub tp_percent: f64,
    pub sl_percent: f64,
    pub trailing_stop_pct: Option<f64>,
    pub tx_buy_sig: Option<String>,
    /// Token amount received from swap (set after buy confirmation).
    pub token_amount: Option<f64>,
}

// ── Decision ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: i64,
    pub candidate_id: i64,
    pub decision_type: String,
    pub action: String,
    pub confidence: Option<f64>,
    pub reasoning: Option<String>,
    pub model_name: Option<String>,
    pub created_at: String,
}

// ── Strategy ──────────────────────────────────────────────────────────────

/// Runtime-configurable strategy stored as JSON in the database.
/// Hot-reloadable without restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub id: i64,
    pub name: String,
    pub config_json: String,
    pub enabled: bool,
    pub updated_at: String,
}

/// The parsed strategy configuration.  This is the typed representation
/// of the `config_json` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    // Source overlap filters
    pub min_source_count: i32,

    // Fee claim filters
    pub min_fee_claims: i32,
    pub max_fee_claim_age_minutes: i64,

    // Market cap filters
    pub min_market_cap_sol: f64,
    pub max_market_cap_sol: f64,

    // Holder filters
    pub min_holders: i32,
    pub max_top_holder_pct: f64,

    // ATH distance filter
    pub max_ath_distance_pct: Option<f64>,

    // Position sizing
    pub buy_sol: f64,
    pub tp_percent: f64,
    pub sl_percent: f64,
    pub trailing_stop_pct: Option<f64>,
    pub trailing_stop_activated_at_pct: Option<f64>,

    // LLM
    pub require_llm_approval: bool,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            min_source_count: 2,
            min_fee_claims: 1,
            max_fee_claim_age_minutes: 60,
            min_market_cap_sol: 5.0,
            max_market_cap_sol: 500.0,
            min_holders: 50,
            max_top_holder_pct: 30.0,
            max_ath_distance_pct: None,
            buy_sol: 0.1,
            tp_percent: 100.0,
            sl_percent: 30.0,
            trailing_stop_pct: None,
            trailing_stop_activated_at_pct: None,
            require_llm_approval: false,
        }
    }
}

// ── Lesson ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    pub id: i64,
    pub position_id: Option<i64>,
    pub mint: String,
    pub lesson_type: String,
    pub summary: String,
    pub tags: Option<String>,
    pub created_at: String,
}

// ── Execution Log ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLog {
    pub id: i64,
    pub position_id: Option<i64>,
    pub mint: String,
    pub action: String,
    pub amount_sol: Option<f64>,
    pub token_amount: Option<f64>,
    pub tx_signature: Option<String>,
    pub status: String,
    pub error_msg: Option<String>,
    pub created_at: String,
}
