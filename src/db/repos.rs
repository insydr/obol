use rusqlite::{params, Connection};
use crate::db::models::*;
use crate::db::DbPool;
use crate::error::{CharonError, Result};

// ═══════════════════════════════════════════════════════════════════════════
// CandidateRepo
// ═══════════════════════════════════════════════════════════════════════════

pub struct CandidateRepo;

impl CandidateRepo {
    /// Insert a new signal, updating source_count if the mint already exists
    /// as a candidate.
    pub fn upsert_from_signal(pool: &DbPool, signal: &NewSignal) -> Result<Candidate> {
        let conn = pool.get()?;
        // Check if candidate already exists for this mint.
        let existing: Option<Candidate> = conn
            .query_row(
                "SELECT id, mint, name, symbol, uri, source_count, sources, market_cap_sol,
                        market_cap_usd, holder_count, top_holder_pct, ath_distance_pct,
                        fee_claim_count, first_seen_at, last_updated_at, status
                 FROM candidates WHERE mint = ?1",
                params![signal.mint],
                |row| Ok(map_candidate(row)),
            )
            .ok();

        if let Some(mut cand) = existing {
            // Increment source count and merge sources.
            let current_sources: Vec<String> = cand
                .sources
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();

            let mut merged = current_sources;
            if !merged.contains(&signal.source) {
                merged.push(signal.source.clone());
            }

            cand.source_count = merged.len() as i32;
            cand.sources = Some(serde_json::to_string(&merged).unwrap_or_default());
            cand.last_updated_at = chrono::Utc::now().naive_utc().to_string();

            if signal.source == "fee_claim" {
                cand.fee_claim_count += 1;
            }

            conn.execute(
                "UPDATE candidates SET source_count = ?1, sources = ?2, fee_claim_count = ?3,
                        last_updated_at = ?4
                 WHERE id = ?5",
                params![
                    cand.source_count,
                    cand.sources,
                    cand.fee_claim_count,
                    cand.last_updated_at,
                    cand.id,
                ],
            )?;

            Ok(cand)
        } else {
            let sources = serde_json::to_string(&vec![signal.source.clone()]).unwrap_or_default();
            let fee_claim_count = if signal.source == "fee_claim" { 1 } else { 0 };
            let now = chrono::Utc::now().naive_utc().to_string();

            conn.execute(
                "INSERT INTO candidates (mint, source_count, sources, fee_claim_count, first_seen_at, last_updated_at, status)
                 VALUES (?1, 1, ?2, ?3, ?4, ?4, 'pending')",
                params![signal.mint, sources, fee_claim_count, now],
            )?;

            let id = conn.last_insert_rowid();
            Ok(Candidate {
                id,
                mint: signal.mint.clone(),
                name: None,
                symbol: None,
                uri: None,
                source_count: 1,
                sources: Some(sources),
                market_cap_sol: None,
                market_cap_usd: None,
                holder_count: None,
                top_holder_pct: None,
                ath_distance_pct: None,
                fee_claim_count,
                first_seen_at: now.clone(),
                last_updated_at: now,
                status: "pending".to_string(),
            })
        }
    }

    /// Fetch candidates with a given status.
    pub fn list_by_status(pool: &DbPool, status: &str) -> Result<Vec<Candidate>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, mint, name, symbol, uri, source_count, sources, market_cap_sol,
                    market_cap_usd, holder_count, top_holder_pct, ath_distance_pct,
                    fee_claim_count, first_seen_at, last_updated_at, status
             FROM candidates WHERE status = ?1 ORDER BY first_seen_at DESC",
        )?;
        let rows = stmt.query_map(params![status], |row| Ok(map_candidate(row)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Update the status of a candidate.
    pub fn update_status(pool: &DbPool, id: i64, status: &str) -> Result<()> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE candidates SET status = ?1, last_updated_at = datetime('now') WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    /// Update enrichment fields on a candidate.
    pub fn update_enrichment(
        pool: &DbPool,
        id: i64,
        market_cap_sol: Option<f64>,
        market_cap_usd: Option<f64>,
        holder_count: Option<i32>,
        top_holder_pct: Option<f64>,
        ath_distance_pct: Option<f64>,
    ) -> Result<()> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE candidates SET market_cap_sol = ?1, market_cap_usd = ?2,
                    holder_count = ?3, top_holder_pct = ?4, ath_distance_pct = ?5,
                    last_updated_at = datetime('now')
             WHERE id = ?6",
            params![market_cap_sol, market_cap_usd, holder_count, top_holder_pct, ath_distance_pct, id],
        )?;
        Ok(())
    }

    /// Get a candidate by mint.
    pub fn get_by_mint(pool: &DbPool, mint: &str) -> Result<Option<Candidate>> {
        let conn = pool.get()?;
        let result = conn
            .query_row(
                "SELECT id, mint, name, symbol, uri, source_count, sources, market_cap_sol,
                        market_cap_usd, holder_count, top_holder_pct, ath_distance_pct,
                        fee_claim_count, first_seen_at, last_updated_at, status
                 FROM candidates WHERE mint = ?1",
                params![mint],
                |row| Ok(map_candidate(row)),
            )
            .ok();
        Ok(result)
    }
}

fn map_candidate(row: &rusqlite::Row<'_>) -> Candidate {
    Candidate {
        id: row.get(0).unwrap_or(0),
        mint: row.get(1).unwrap_or_default(),
        name: row.get(2).unwrap_or(None),
        symbol: row.get(3).unwrap_or(None),
        uri: row.get(4).unwrap_or(None),
        source_count: row.get(5).unwrap_or(1),
        sources: row.get(6).unwrap_or(None),
        market_cap_sol: row.get(7).unwrap_or(None),
        market_cap_usd: row.get(8).unwrap_or(None),
        holder_count: row.get(9).unwrap_or(None),
        top_holder_pct: row.get(10).unwrap_or(None),
        ath_distance_pct: row.get(11).unwrap_or(None),
        fee_claim_count: row.get(12).unwrap_or(0),
        first_seen_at: row.get(13).unwrap_or_default(),
        last_updated_at: row.get(14).unwrap_or_default(),
        status: row.get(15).unwrap_or_else(|_| "pending".to_string()),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PositionRepo
// ═══════════════════════════════════════════════════════════════════════════

pub struct PositionRepo;

impl PositionRepo {
    /// Open a new position from approved candidate parameters.
    pub fn open(pool: &DbPool, params: &OpenPositionParams) -> Result<Position> {
        let conn = pool.get()?;
        let now = chrono::Utc::now().naive_utc().to_string();
        conn.execute(
            "INSERT INTO positions (mint, symbol, buy_sol, tp_percent, sl_percent,
                    trailing_stop_pct, trailing_activated, highest_pnl_pct, status, opened_at, tx_buy_sig)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0.0, 'open', ?7, ?8)",
            params![
                params.mint,
                params.symbol,
                params.buy_sol,
                params.tp_percent,
                params.sl_percent,
                params.trailing_stop_pct,
                now,
                params.tx_buy_sig,
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(Position {
            id,
            mint: params.mint.clone(),
            symbol: params.symbol.clone(),
            entry_price: None,
            current_price: None,
            buy_sol: params.buy_sol,
            token_amount: None,
            pnl_percent: 0.0,
            pnl_sol: 0.0,
            tp_percent: Some(params.tp_percent),
            sl_percent: Some(params.sl_percent),
            trailing_stop_pct: params.trailing_stop_pct,
            trailing_activated: false,
            highest_pnl_pct: 0.0,
            status: "open".to_string(),
            opened_at: now,
            closed_at: None,
            close_reason: None,
            tx_buy_sig: params.tx_buy_sig.clone(),
            tx_sell_sig: None,
        })
    }

    /// List all open positions.
    pub fn list_open(pool: &DbPool) -> Result<Vec<Position>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, mint, symbol, entry_price, current_price, buy_sol, token_amount,
                    pnl_percent, pnl_sol, tp_percent, sl_percent, trailing_stop_pct,
                    trailing_activated, highest_pnl_pct, status, opened_at, closed_at,
                    close_reason, tx_buy_sig, tx_sell_sig
             FROM positions WHERE status = 'open' ORDER BY opened_at DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok(map_position(row)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Update PnL and price tracking for an open position.
    pub fn update_pnl(
        pool: &DbPool,
        id: i64,
        current_price: f64,
        pnl_percent: f64,
        pnl_sol: f64,
        highest_pnl_pct: f64,
        trailing_activated: bool,
    ) -> Result<()> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE positions SET current_price = ?1, pnl_percent = ?2, pnl_sol = ?3,
                    highest_pnl_pct = ?4, trailing_activated = ?5
             WHERE id = ?6",
            params![current_price, pnl_percent, pnl_sol, highest_pnl_pct, trailing_activated as i32, id],
        )?;
        Ok(())
    }

    /// Close a position with a reason and optional tx signature.
    pub fn close(pool: &DbPool, id: i64, reason: &str, tx_sell_sig: Option<&str>) -> Result<()> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE positions SET status = 'closed', closed_at = datetime('now'),
                    close_reason = ?1, tx_sell_sig = ?2
             WHERE id = ?3",
            params![reason, tx_sell_sig, id],
        )?;
        Ok(())
    }

    /// Get a position by ID.
    pub fn get_by_id(pool: &DbPool, id: i64) -> Result<Option<Position>> {
        let conn = pool.get()?;
        let result = conn
            .query_row(
                "SELECT id, mint, symbol, entry_price, current_price, buy_sol, token_amount,
                        pnl_percent, pnl_sol, tp_percent, sl_percent, trailing_stop_pct,
                        trailing_activated, highest_pnl_pct, status, opened_at, closed_at,
                        close_reason, tx_buy_sig, tx_sell_sig
                 FROM positions WHERE id = ?1",
                params![id],
                |row| Ok(map_position(row)),
            )
            .ok();
        Ok(result)
    }

    /// Count open positions.
    pub fn count_open(pool: &DbPool) -> Result<i64> {
        let conn = pool.get()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM positions WHERE status = 'open'", [], |row| row.get(0))
            .unwrap_or(0);
        Ok(count)
    }
}

fn map_position(row: &rusqlite::Row<'_>) -> Position {
    Position {
        id: row.get(0).unwrap_or(0),
        mint: row.get(1).unwrap_or_default(),
        symbol: row.get(2).unwrap_or(None),
        entry_price: row.get(3).unwrap_or(None),
        current_price: row.get(4).unwrap_or(None),
        buy_sol: row.get(5).unwrap_or(0.0),
        token_amount: row.get(6).unwrap_or(None),
        pnl_percent: row.get(7).unwrap_or(0.0),
        pnl_sol: row.get(8).unwrap_or(0.0),
        tp_percent: row.get(9).unwrap_or(None),
        sl_percent: row.get(10).unwrap_or(None),
        trailing_stop_pct: row.get(11).unwrap_or(None),
        trailing_activated: row.get::<_, i32>(12).unwrap_or(0) == 1,
        highest_pnl_pct: row.get(13).unwrap_or(0.0),
        status: row.get(14).unwrap_or_else(|_| "open".to_string()),
        opened_at: row.get(15).unwrap_or_default(),
        closed_at: row.get(16).unwrap_or(None),
        close_reason: row.get(17).unwrap_or(None),
        tx_buy_sig: row.get(18).unwrap_or(None),
        tx_sell_sig: row.get(19).unwrap_or(None),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// StrategyRepo
// ═══════════════════════════════════════════════════════════════════════════

pub struct StrategyRepo;

impl StrategyRepo {
    /// Get the active strategy by name.
    pub fn get(pool: &DbPool, name: &str) -> Result<Option<Strategy>> {
        let conn = pool.get()?;
        let result = conn
            .query_row(
                "SELECT id, name, config_json, enabled, updated_at FROM strategies WHERE name = ?1 AND enabled = 1",
                params![name],
                |row| {
                    Ok(Strategy {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        config_json: row.get(2)?,
                        enabled: row.get::<_, i32>(3)? == 1,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .ok();
        Ok(result)
    }

    /// Parse the strategy config JSON into a typed struct.
    pub fn get_config(pool: &DbPool, name: &str) -> Result<StrategyConfig> {
        match Self::get(pool, name)? {
            Some(strategy) => {
                let config: StrategyConfig = serde_json::from_str(&strategy.config_json)
                    .unwrap_or_else(|_| StrategyConfig::default());
                Ok(config)
            }
            None => Ok(StrategyConfig::default()),
        }
    }

    /// Upsert a strategy.
    pub fn upsert(pool: &DbPool, name: &str, config: &StrategyConfig) -> Result<()> {
        let conn = pool.get()?;
        let json = serde_json::to_string(config)
            .map_err(|e| CharonError::Internal(format!("Failed to serialize strategy: {}", e)))?;
        conn.execute(
            "INSERT INTO strategies (name, config_json, enabled, updated_at)
             VALUES (?1, ?2, 1, datetime('now'))
             ON CONFLICT(name) DO UPDATE SET config_json = ?2, updated_at = datetime('now')",
            params![name, json],
        )?;
        Ok(())
    }

    /// Update a single field on a strategy.
    pub fn set_field(pool: &DbPool, name: &str, field: &str, value: &str) -> Result<()> {
        let mut config = Self::get_config(pool, name)?;
        match field {
            "min_source_count" => config.min_source_count = value.parse().unwrap_or(config.min_source_count),
            "min_fee_claims" => config.min_fee_claims = value.parse().unwrap_or(config.min_fee_claims),
            "min_market_cap_sol" => config.min_market_cap_sol = value.parse().unwrap_or(config.min_market_cap_sol),
            "max_market_cap_sol" => config.max_market_cap_sol = value.parse().unwrap_or(config.max_market_cap_sol),
            "min_holders" => config.min_holders = value.parse().unwrap_or(config.min_holders),
            "max_top_holder_pct" => config.max_top_holder_pct = value.parse().unwrap_or(config.max_top_holder_pct),
            "buy_sol" => config.buy_sol = value.parse().unwrap_or(config.buy_sol),
            "tp_percent" => config.tp_percent = value.parse().unwrap_or(config.tp_percent),
            "sl_percent" => config.sl_percent = value.parse().unwrap_or(config.sl_percent),
            _ => {
                return Err(CharonError::Config(format!(
                    "Unknown strategy field: '{}'",
                    field
                )))
            }
        }
        Self::upsert(pool, name, &config)
    }

    /// List all strategies.
    pub fn list_all(pool: &DbPool) -> Result<Vec<Strategy>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, config_json, enabled, updated_at FROM strategies ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Strategy {
                id: row.get(0)?,
                name: row.get(1)?,
                config_json: row.get(2)?,
                enabled: row.get::<_, i32>(3)? == 1,
                updated_at: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DecisionRepo
// ═══════════════════════════════════════════════════════════════════════════

pub struct DecisionRepo;

impl DecisionRepo {
    pub fn insert(
        pool: &DbPool,
        candidate_id: i64,
        decision_type: &str,
        action: &str,
        confidence: Option<f64>,
        reasoning: Option<&str>,
        model_name: Option<&str>,
    ) -> Result<i64> {
        let conn = pool.get()?;
        conn.execute(
            "INSERT INTO decisions (candidate_id, decision_type, action, confidence, reasoning, model_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![candidate_id, decision_type, action, confidence, reasoning, model_name],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// LessonRepo
// ═══════════════════════════════════════════════════════════════════════════

pub struct LessonRepo;

impl LessonRepo {
    pub fn insert(
        pool: &DbPool,
        position_id: Option<i64>,
        mint: &str,
        lesson_type: &str,
        summary: &str,
        tags: Option<&str>,
    ) -> Result<i64> {
        let conn = pool.get()?;
        conn.execute(
            "INSERT INTO lessons (position_id, mint, lesson_type, summary, tags)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![position_id, mint, lesson_type, summary, tags],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_recent(pool: &DbPool, limit: i64) -> Result<Vec<Lesson>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, position_id, mint, lesson_type, summary, tags, created_at
             FROM lessons ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(Lesson {
                id: row.get(0)?,
                position_id: row.get(1)?,
                mint: row.get(2)?,
                lesson_type: row.get(3)?,
                summary: row.get(4)?,
                tags: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
