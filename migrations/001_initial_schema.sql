-- Migration 001: Initial schema
-- This is the canonical schema definition.  The application also
-- creates these tables automatically via the embedded SQL in
-- src/db/schema.rs, so this file is primarily for documentation
-- and manual database setup.

-- Core signal tables
CREATE TABLE IF NOT EXISTS signals (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    source          TEXT NOT NULL,
    mint            TEXT NOT NULL,
    payload         TEXT,
    detected_at     TEXT NOT NULL DEFAULT (datetime('now')),
    processed       INTEGER NOT NULL DEFAULT 0,
    UNIQUE(source, mint, detected_at)
);

CREATE INDEX IF NOT EXISTS idx_signals_mint ON signals(mint);
CREATE INDEX IF NOT EXISTS idx_signals_processed ON signals(processed);

-- Candidate tokens
CREATE TABLE IF NOT EXISTS candidates (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    mint            TEXT NOT NULL UNIQUE,
    name            TEXT,
    symbol          TEXT,
    uri             TEXT,
    source_count    INTEGER NOT NULL DEFAULT 1,
    sources         TEXT,
    market_cap_sol  REAL,
    market_cap_usd  REAL,
    holder_count    INTEGER,
    top_holder_pct  REAL,
    ath_distance_pct REAL,
    fee_claim_count INTEGER DEFAULT 0,
    first_seen_at   TEXT NOT NULL DEFAULT (datetime('now')),
    last_updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    status          TEXT NOT NULL DEFAULT 'pending'
);

CREATE INDEX IF NOT EXISTS idx_candidates_status ON candidates(status);
CREATE INDEX IF NOT EXISTS idx_candidates_mint ON candidates(mint);

-- Positions
CREATE TABLE IF NOT EXISTS positions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    mint            TEXT NOT NULL,
    symbol          TEXT,
    entry_price     REAL,
    current_price   REAL,
    buy_sol         REAL NOT NULL,
    token_amount    REAL,
    pnl_percent     REAL DEFAULT 0.0,
    pnl_sol         REAL DEFAULT 0.0,
    tp_percent      REAL,
    sl_percent      REAL,
    trailing_stop_pct   REAL,
    trailing_activated  INTEGER NOT NULL DEFAULT 0,
    highest_pnl_pct     REAL DEFAULT 0.0,
    status          TEXT NOT NULL DEFAULT 'open',
    opened_at       TEXT NOT NULL DEFAULT (datetime('now')),
    closed_at       TEXT,
    close_reason    TEXT,
    tx_buy_sig      TEXT,
    tx_sell_sig     TEXT
);

CREATE INDEX IF NOT EXISTS idx_positions_status ON positions(status);
CREATE INDEX IF NOT EXISTS idx_positions_mint ON positions(mint);

-- Decisions
CREATE TABLE IF NOT EXISTS decisions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    candidate_id    INTEGER NOT NULL REFERENCES candidates(id),
    decision_type   TEXT NOT NULL,
    action          TEXT NOT NULL,
    confidence      REAL,
    reasoning       TEXT,
    model_name      TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_decisions_candidate ON decisions(candidate_id);

-- Strategies
CREATE TABLE IF NOT EXISTS strategies (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL UNIQUE,
    config_json     TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_strategies_name ON strategies(name);

-- Lessons
CREATE TABLE IF NOT EXISTS lessons (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    position_id     INTEGER REFERENCES positions(id),
    mint            TEXT NOT NULL,
    lesson_type     TEXT NOT NULL,
    summary         TEXT NOT NULL,
    tags            TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_lessons_mint ON lessons(mint);

-- Execution log
CREATE TABLE IF NOT EXISTS execution_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    position_id     INTEGER REFERENCES positions(id),
    mint            TEXT NOT NULL,
    action          TEXT NOT NULL,
    amount_sol      REAL,
    token_amount    REAL,
    tx_signature    TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    error_msg       TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_execution_log_position ON execution_log(position_id);

-- Enrichment cache
CREATE TABLE IF NOT EXISTS enrichment_cache (
    mint            TEXT PRIMARY KEY,
    jupiter_data    TEXT,
    gmgn_data       TEXT,
    twitter_data    TEXT,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Fee claims
CREATE TABLE IF NOT EXISTS fee_claims (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    mint            TEXT NOT NULL,
    claim_type      TEXT,
    slot            INTEGER,
    amount_lamports INTEGER,
    detected_at     TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(mint, slot)
);

CREATE INDEX IF NOT EXISTS idx_fee_claims_mint ON fee_claims(mint);

-- Trending tokens
CREATE TABLE IF NOT EXISTS trending_tokens (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    mint            TEXT NOT NULL,
    source          TEXT NOT NULL,
    rank            INTEGER,
    data_json       TEXT,
    detected_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_trending_mint ON trending_tokens(mint);

-- Price snapshots
CREATE TABLE IF NOT EXISTS price_snapshots (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    mint            TEXT NOT NULL,
    price_sol       REAL NOT NULL,
    price_usd       REAL,
    market_cap_sol  REAL,
    volume_24h_sol  REAL,
    recorded_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_price_snapshots_mint ON price_snapshots(mint);

-- Wallet state
CREATE TABLE IF NOT EXISTS wallet_state (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    sol_balance     REAL NOT NULL,
    total_pnl_sol   REAL DEFAULT 0.0,
    open_positions  INTEGER DEFAULT 0,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Strategy overrides
CREATE TABLE IF NOT EXISTS strategy_overrides (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    strategy_name   TEXT NOT NULL,
    field           TEXT NOT NULL,
    old_value       TEXT,
    new_value       TEXT,
    changed_by      TEXT,
    changed_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

-- System metadata
CREATE TABLE IF NOT EXISTS system_meta (
    key             TEXT PRIMARY KEY,
    value           TEXT NOT NULL,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
