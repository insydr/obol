use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::config::AppConfig;
use crate::db::models::*;
use crate::db::{CandidateRepo, DbPool, DecisionRepo, ExecutionLogRepo, PositionRepo, StrategyRepo};
use crate::enrichment::{EnrichmentData, EnrichmentService};
use crate::error::{CharonError, Result};
use crate::execution::router::SwapRouter;
use crate::pipeline::candidate::CandidateBuilder;
use crate::pipeline::llm::LlmClient;
use crate::signals::SignalEvent;
use crate::telegram::formatting::TelegramFormatter;

/// The main processing pipeline.  Receives raw signal events, filters and
/// enriches them, optionally runs LLM screening, and then either executes
/// trades or queues them for manual confirmation.
pub struct PipelineOrchestrator {
    config: Arc<AppConfig>,
    db: DbPool,
    enrichment: Arc<EnrichmentService>,
    llm: Arc<LlmClient>,
    router: Arc<SwapRouter>,
    strategy: Arc<RwLock<StrategyConfig>>,
    formatter: TelegramFormatter,
}

impl PipelineOrchestrator {
    pub fn new(
        config: Arc<AppConfig>,
        db: DbPool,
        enrichment: Arc<EnrichmentService>,
        llm: Arc<LlmClient>,
        router: Arc<SwapRouter>,
    ) -> Result<Self> {
        // Load initial strategy from DB or use defaults.
        let strategy_config = StrategyRepo::get_config(&db, "sniper").unwrap_or_default();
        Ok(Self {
            config,
            db,
            enrichment,
            llm,
            router,
            strategy: Arc::new(RwLock::new(strategy_config)),
            formatter: TelegramFormatter,
        })
    }

    /// Hot-reload the strategy from the database.
    pub async fn reload_strategy(&self) {
        let new_config = StrategyRepo::get_config(&self.db, "sniper").unwrap_or_default();
        let mut guard = self.strategy.write().await;
        *guard = new_config;
        tracing::info!("Strategy reloaded");
    }

    /// Process a single signal event through the full pipeline.
    pub async fn process_signal(&self, event: SignalEvent) -> Result<PipelineOutcome> {
        tracing::info!(
            source = %event.source,
            mint = %event.mint,
            "Processing signal"
        );

        // Step 1: Upsert candidate from signal.
        let new_signal = crate::db::models::NewSignal {
            source: event.source.clone(),
            mint: event.mint.clone(),
            payload: event.payload.map(|v| v.to_string()),
        };
        let candidate = CandidateRepo::upsert_from_signal(&self.db, &new_signal)?;

        // Step 2: Check if candidate already processed.
        if candidate.status != "pending" && candidate.status != "screened" {
            tracing::debug!(mint = %candidate.mint, status = %candidate.status, "Skipping already processed candidate");
            return Ok(PipelineOutcome::AlreadyProcessed(candidate.status));
        }

        // Step 3: Apply strategy gate filters.
        let strategy = self.strategy.read().await.clone();
        if let Err(reason) = CandidateBuilder::apply_filters(&candidate, &strategy) {
            tracing::info!(mint = %candidate.mint, reason = %reason, "Candidate rejected by strategy filters");
            CandidateRepo::update_status(&self.db, candidate.id, "rejected")?;
            DecisionRepo::insert(
                &self.db,
                candidate.id,
                "auto",
                "reject",
                None,
                Some(&reason),
                None,
            )?;
            return Ok(PipelineOutcome::Rejected(reason));
        }

        // Step 4: Enrich candidate.
        let enrichment_data = self.enrichment.enrich(&candidate.mint).await;
        if let Some(ref data) = enrichment_data {
            CandidateRepo::update_enrichment(
                &self.db,
                candidate.id,
                data.market_cap_sol,
                data.market_cap_usd,
                data.holder_count,
                data.top_holder_pct,
                data.ath_distance_pct,
            )?;
        }

        let mut candidate = candidate;
        // Populate buy_sol from current strategy for confirm mode display.
        candidate.buy_sol = strategy.buy_sol;

        let enriched = EnrichedCandidate {
            candidate: candidate.clone(),
            jupiter_data: enrichment_data.as_ref().and_then(|d| d.jupiter.clone()),
            gmgn_data: enrichment_data.as_ref().and_then(|d| d.gmgn.clone()),
            twitter_data: enrichment_data.as_ref().and_then(|d| d.twitter.clone()),
        };

        // Step 5: LLM screening (if enabled).
        if strategy.require_llm_approval && self.config.enable_llm {
            let batch = vec![enriched.clone()];
            let decisions = self.llm.screen_candidates(&batch).await?;
            if let Some(decision) = decisions.first() {
                DecisionRepo::insert(
                    &self.db,
                    candidate.id,
                    "llm",
                    &decision.action,
                    decision.confidence,
                    decision.reasoning.as_deref(),
                    Some(&self.config.llm_model),
                )?;
                if decision.action != "approve" {
                    CandidateRepo::update_status(&self.db, candidate.id, "rejected")?;
                    return Ok(PipelineOutcome::RejectedByLlm(decision.reasoning.unwrap_or_default()));
                }
            }
        }

        // Step 6: Approve candidate.
        CandidateRepo::update_status(&self.db, candidate.id, "approved")?;

        // Step 7: Execute based on trading mode.
        let buy_sol = strategy.buy_sol;
        let tp_percent = strategy.tp_percent;
        let sl_percent = strategy.sl_percent;

        match self.config.trading_mode {
            crate::config::TradingMode::DryRun => {
                tracing::info!(
                    mint = %candidate.mint,
                    buy_sol = buy_sol,
                    "DRY RUN: Simulating buy"
                );
                let position = PositionRepo::open(
                    &self.db,
                    &OpenPositionParams {
                        mint: candidate.mint.clone(),
                        symbol: candidate.symbol.clone(),
                        buy_sol,
                        tp_percent,
                        sl_percent,
                        trailing_stop_pct: strategy.trailing_stop_pct,
                        tx_buy_sig: Some("dry_run".to_string()),
                        token_amount: None,
                    },
                )?;
                Ok(PipelineOutcome::DryRun(position))
            }
            crate::config::TradingMode::Confirm => {
                tracing::info!(
                    mint = %candidate.mint,
                    "CONFIRM mode: Queuing for Telegram approval"
                );
                Ok(PipelineOutcome::AwaitingConfirmation(enriched))
            }
            crate::config::TradingMode::Live => {
                self.execute_live_buy(&candidate, buy_sol, tp_percent, sl_percent, &strategy).await
            }
        }
    }

    /// Execute a buy for an already-approved candidate (called from
    /// Telegram /confirm command or inline keyboard).
    pub async fn execute_approved(&self, candidate_id: i64) -> Result<String> {
        let candidate = CandidateRepo::get_by_id(&self.db, candidate_id)?
            .ok_or_else(|| CharonError::Execution(format!("Candidate #{} not found", candidate_id)))?;

        let strategy = self.strategy.read().await.clone();

        match self.config.trading_mode {
            crate::config::TradingMode::DryRun => {
                let position = PositionRepo::open(
                    &self.db,
                    &OpenPositionParams {
                        mint: candidate.mint.clone(),
                        symbol: candidate.symbol.clone(),
                        buy_sol: strategy.buy_sol,
                        tp_percent: strategy.tp_percent,
                        sl_percent: strategy.sl_percent,
                        trailing_stop_pct: strategy.trailing_stop_pct,
                        tx_buy_sig: Some("dry_run".to_string()),
                        token_amount: None,
                    },
                )?;
                Ok(format!(
                    "🧪 DRY RUN: Simulated buy #{} — {} ({}) — {:.3} SOL",
                    position.id,
                    position.symbol.as_deref().unwrap_or("???"),
                    &position.mint[..8],
                    position.buy_sol,
                ))
            }
            crate::config::TradingMode::Confirm | crate::config::TradingMode::Live => {
                match self.execute_live_buy(
                    &candidate,
                    strategy.buy_sol,
                    strategy.tp_percent,
                    strategy.sl_percent,
                    &strategy,
                ).await {
                    Ok(PipelineOutcome::Executed(position, tx_sig)) => {
                        Ok(format!(
                            "✅ Trade executed #{} — {} ({}) — {:.3} SOL | TX: {}...",
                            position.id,
                            position.symbol.as_deref().unwrap_or("???"),
                            &position.mint[..8],
                            position.buy_sol,
                            &tx_sig[..16],
                        ))
                    }
                    Ok(other) => Ok(format!("Unexpected outcome: {:?}", other)),
                    Err(e) => Err(e),
                }
            }
        }
    }

    /// Execute a live buy via Jupiter, tracking token amounts and logging
    /// the execution.
    async fn execute_live_buy(
        &self,
        candidate: &Candidate,
        buy_sol: f64,
        tp_percent: f64,
        sl_percent: f64,
        strategy: &StrategyConfig,
    ) -> Result<PipelineOutcome> {
        #[cfg(feature = "live-trading")]
        {
            tracing::info!(mint = %candidate.mint, buy_sol = buy_sol, "LIVE: Executing buy");

            // Log the execution attempt.
            let log_id = ExecutionLogRepo::insert(
                &self.db,
                None,
                &candidate.mint,
                "buy",
                Some(buy_sol),
                None,
                None,
                "pending",
                None,
            )?;

            match self.router.buy(&candidate.mint, buy_sol).await {
                Ok(swap_result) => {
                    // Open position with token amount from swap.
                    let token_amount = swap_result.token_amount.map(|a| a as f64);
                    let position = PositionRepo::open(
                        &self.db,
                        &OpenPositionParams {
                            mint: candidate.mint.clone(),
                            symbol: candidate.symbol.clone(),
                            buy_sol,
                            tp_percent,
                            sl_percent,
                            trailing_stop_pct: strategy.trailing_stop_pct,
                            tx_buy_sig: Some(swap_result.signature.clone()),
                            token_amount,
                        },
                    )?;

                    // Mark execution log as successful.
                    ExecutionLogRepo::mark_success(
                        &self.db,
                        log_id,
                        Some(&swap_result.signature),
                    )?;

                    Ok(PipelineOutcome::Executed(position, swap_result.signature))
                }
                Err(e) => {
                    tracing::error!(mint = %candidate.mint, error = %e, "Live buy failed");
                    ExecutionLogRepo::mark_failed(&self.db, log_id, &e.to_string())?;
                    Err(e)
                }
            }
        }
        #[cfg(not(feature = "live-trading"))]
        {
            tracing::warn!("Live trading is disabled (compile with --features live-trading)");
            Ok(PipelineOutcome::Rejected("live_trading_disabled".to_string()))
        }
    }
}

/// Outcome of processing a signal through the pipeline.
#[derive(Debug)]
pub enum PipelineOutcome {
    AlreadyProcessed(String),
    Rejected(String),
    RejectedByLlm(String),
    DryRun(Position),
    AwaitingConfirmation(EnrichedCandidate),
    Executed(Position, String),
}
