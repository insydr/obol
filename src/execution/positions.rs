use std::sync::Arc;

use crate::config::AppConfig;
use crate::db::models::{Position, StrategyConfig};
use crate::db::{PositionRepo, DbPool};
use crate::enrichment::jupiter::JupiterClient;
use crate::error::Result;

/// Position lifecycle manager.  Monitors open positions for TP/SL/trailing
/// stop conditions and triggers sell orders when thresholds are hit.
pub struct PositionManager {
    db: DbPool,
    jupiter: Arc<JupiterClient>,
    config: Arc<AppConfig>,
}

impl PositionManager {
    pub fn new(db: DbPool, jupiter: Arc<JupiterClient>, config: Arc<AppConfig>) -> Self {
        Self { db, jupiter, config }
    }

    /// Run one monitoring cycle: check all open positions.
    pub async fn monitor_cycle(&self) -> Result<Vec<PositionAction>> {
        let positions = PositionRepo::list_open(&self.db)?;
        let mut actions = Vec::new();

        for position in positions {
            match self.check_position(&position).await {
                Ok(Some(action)) => {
                    tracing::info!(
                        mint = %position.mint,
                        id = position.id,
                        action = ?action,
                        "Position threshold triggered"
                    );
                    actions.push(action);
                }
                Ok(None) => {
                    // Position is within normal range.
                }
                Err(e) => {
                    tracing::error!(
                        mint = %position.mint,
                        id = position.id,
                        error = %e,
                        "Position check failed"
                    );
                }
            }
        }

        Ok(actions)
    }

    /// Check a single position against TP/SL/trailing stop thresholds.
    async fn check_position(&self, position: &Position) -> Result<Option<PositionAction>> {
        // Fetch current price from Jupiter.
        let price_data = self
            .jupiter
            .fetch_token_info(&position.mint)
            .await
            .ok()
            .flatten();

        let current_price = match price_data {
            Some(ref data) => data
                .get("price")
                .and_then(|p| p.as_f64())
                .unwrap_or(0.0),
            None => {
                tracing::debug!(mint = %position.mint, "No price data available");
                return Ok(None);
            }
        };

        // Update position PnL.
        let entry_price = position.entry_price.unwrap_or(0.0);
        let pnl_percent = if entry_price > 0.0 {
            ((current_price - entry_price) / entry_price) * 100.0
        } else {
            0.0
        };

        let pnl_sol = position.buy_sol * (pnl_percent / 100.0);
        let highest_pnl = position.highest_pnl_pct.max(pnl_percent);
        let trailing_activated = if let Some(act_pct) = position.trailing_stop_pct {
            pnl_percent >= act_pct || position.trailing_activated
        } else {
            false
        };

        // Persist updated PnL.
        PositionRepo::update_pnl(
            &self.db,
            position.id,
            current_price,
            pnl_percent,
            pnl_sol,
            highest_pnl,
            trailing_activated,
        )?;

        // Check take-profit.
        if let Some(tp) = position.tp_percent {
            if pnl_percent >= tp {
                return Ok(Some(PositionAction::TakeProfit {
                    position_id: position.id,
                    mint: position.mint.clone(),
                    pnl_percent,
                }));
            }
        }

        // Check stop-loss.
        if let Some(sl) = position.sl_percent {
            if pnl_percent <= -sl {
                return Ok(Some(PositionAction::StopLoss {
                    position_id: position.id,
                    mint: position.mint.clone(),
                    pnl_percent,
                }));
            }
        }

        // Check trailing stop.
        if let (Some(trailing_pct), true) = (position.trailing_stop_pct, trailing_activated) {
            let drawdown_from_peak = highest_pnl - pnl_percent;
            if drawdown_from_peak >= trailing_pct && pnl_percent > 0.0 {
                return Ok(Some(PositionAction::TrailingStop {
                    position_id: position.id,
                    mint: position.mint.clone(),
                    highest_pnl,
                    current_pnl: pnl_percent,
                }));
            }
        }

        Ok(None)
    }

    /// Close a position in the database and optionally execute a sell.
    pub async fn close_position(
        &self,
        position_id: i64,
        reason: &str,
        tx_sig: Option<&str>,
    ) -> Result<()> {
        PositionRepo::close(&self.db, position_id, reason, tx_sig)
    }
}

/// Actions that can be triggered on a position.
#[derive(Debug)]
pub enum PositionAction {
    TakeProfit {
        position_id: i64,
        mint: String,
        pnl_percent: f64,
    },
    StopLoss {
        position_id: i64,
        mint: String,
        pnl_percent: f64,
    },
    TrailingStop {
        position_id: i64,
        mint: String,
        highest_pnl: f64,
        current_pnl: f64,
    },
    PartialTakeProfit {
        position_id: i64,
        mint: String,
        sell_percent: f64,
        pnl_percent: f64,
    },
}
