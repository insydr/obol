mod config;
mod db;
mod enrichment;
mod error;
mod execution;
mod learning;
mod pipeline;
mod signals;
mod telegram;
mod utils;

use std::sync::Arc;

use tokio::sync::mpsc;

use config::AppConfig;
use db::{CandidateRepo, DbPool, PositionRepo, StrategyRepo};
use enrichment::EnrichmentService;
use execution::positions::PositionManager;
use execution::router::SwapRouter;
use execution::wallet::WalletService;
use learning::analyzer::TradeAnalyzer;
use pipeline::llm::LlmClient;
use pipeline::orchestrator::PipelineOrchestrator;
use signals::fee_claim::FeeClaimListener;
use signals::graduated::GraduatedPoller;
use signals::price_monitor::PriceMonitor;
use signals::server_client::SignalServerClient;
use signals::trending::TrendingPoller;
use telegram::bot::CharonBot;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present.
    dotenvy::dotenv().ok();

    // Load and validate configuration.
    let config = AppConfig::from_env()?;

    // Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level)),
        )
        .init();

    tracing::info!("Charon-RS starting in {} mode", config.trading_mode);

    // Initialize database.
    let pool = db::create_pool(&config.database_path)?;
    db::run_migrations(&pool)?;

    // Ensure a default strategy exists.
    let default_strategy = db::models::StrategyConfig::default();
    StrategyRepo::upsert(&pool, "sniper", &default_strategy)?;

    // Build shared services.
    let config = Arc::new(config);
    let wallet = Arc::new(WalletService::new(&config)?);
    let enrichment = Arc::new(EnrichmentService::new(&config));
    let llm = Arc::new(LlmClient::new(&config));
    let jupiter = Arc::new(enrichment::jupiter::JupiterClient::new(&config));
    let router = Arc::new(SwapRouter::new(&config, wallet.clone()));
    let analyzer = Arc::new(TradeAnalyzer::new(pool.clone()));

    // Build the pipeline orchestrator.
    let orchestrator = Arc::new(PipelineOrchestrator::new(
        config.clone(),
        pool.clone(),
        enrichment.clone(),
        llm.clone(),
        router.clone(),
    )?);

    // Build the Telegram bot.
    let telegram_bot = Arc::new(CharonBot::new(
        config.clone(),
        pool.clone(),
        orchestrator.clone(),
    ));

    // ── Signal channels ──────────────────────────────────────────────────
    let (signal_tx, mut signal_rx) = mpsc::channel::<signals::SignalEvent>(1024);

    // Spawn signal pollers.
    let signal_client = Arc::new(SignalServerClient::new(&config));
    signal_client.spawn_polling_loop(signal_tx.clone());

    let graduated = Arc::new(GraduatedPoller::new(&config));
    graduated.spawn(signal_tx.clone());

    let trending = Arc::new(TrendingPoller::new(&config));
    trending.spawn(signal_tx.clone());

    let fee_claim = Arc::new(FeeClaimListener::new(&config));
    fee_claim.spawn(signal_tx.clone());

    // Price monitor — would need tracked mints from DB in production.
    // For now, spawn with empty list.
    let price_monitor = Arc::new(PriceMonitor::new(&config));
    price_monitor.spawn(signal_tx.clone(), vec![]);

    tracing::info!("All signal pollers started");

    // ── Main processing loop ─────────────────────────────────────────────
    let processing_db = pool.clone();
    let processing_orchestrator = orchestrator.clone();
    let processing_telegram = telegram_bot.clone();

    let processing_handle = tokio::spawn(async move {
        while let Some(event) = signal_rx.recv().await {
            tracing::debug!(source = %event.source, mint = %event.mint, "Received signal");

            match processing_orchestrator.process_signal(event).await {
                Ok(outcome) => {
                    match outcome {
                        pipeline::orchestrator::PipelineOutcome::DryRun(position) => {
                            processing_telegram.send_message(&format!(
                                "🧪 DRY RUN: Simulated buy on {} ({}) — {:.3} SOL",
                                position.symbol.as_deref().unwrap_or("???"),
                                &position.mint[..8],
                                position.buy_sol,
                            )).await;
                        }
                        pipeline::orchestrator::PipelineOutcome::AwaitingConfirmation(enriched) => {
                            let sources: Vec<String> = enriched
                                .candidate
                                .sources
                                .as_ref()
                                .and_then(|s| serde_json::from_str(s).ok())
                                .unwrap_or_default();
                            processing_telegram.send_candidate_alert(
                                &enriched.candidate.mint,
                                enriched.candidate.symbol.as_deref().unwrap_or("???"),
                                enriched.candidate.buy_sol,
                                &sources,
                                enriched.candidate.market_cap_sol,
                            ).await;
                        }
                        pipeline::orchestrator::PipelineOutcome::Executed(position, tx_sig) => {
                            processing_telegram.send_message(&format!(
                                "✅ LIVE: Bought {} ({}) — {:.3} SOL | TX: {}",
                                position.symbol.as_deref().unwrap_or("???"),
                                &position.mint[..8],
                                position.buy_sol,
                                &tx_sig[..16],
                            )).await;
                        }
                        _ => {
                            // Rejected, already processed, etc. — no notification needed.
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Pipeline processing error");
                }
            }
        }
    });

    // ── Position monitor loop ────────────────────────────────────────────
    let monitor_db = pool.clone();
    let monitor_config = config.clone();
    let monitor_telegram = telegram_bot.clone();

    let monitor_handle = tokio::spawn(async move {
        let position_mgr = PositionManager::new(
            monitor_db,
            jupiter.clone(),
            monitor_config,
        );

        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            monitor_config.position_monitor_ms,
        ));

        loop {
            interval.tick().await;
            match position_mgr.monitor_cycle().await {
                Ok(actions) => {
                    for action in actions {
                        match action {
                            execution::positions::PositionAction::TakeProfit {
                                position_id,
                                ref mint,
                                pnl_percent,
                            } => {
                                // In live mode, execute the sell.  Otherwise just close in DB.
                                #[cfg(feature = "live-trading")]
                                {
                                    if let Ok(Some(pos)) = PositionRepo::get_by_id(position_mgr.db(), position_id) {
                                        match router.sell(mint, pos.token_amount.unwrap_or(0) as u64).await {
                                            Ok(sig) => {
                                                position_mgr.close_position(position_id, "tp", Some(&sig)).await.ok();
                                            }
                                            Err(e) => {
                                                tracing::error!(id = position_id, error = %e, "TP sell failed");
                                            }
                                        }
                                    }
                                }
                                #[cfg(not(feature = "live-trading"))]
                                {
                                    position_mgr.close_position(position_id, "tp", None).await.ok();
                                }

                                monitor_telegram.send_position_update(
                                    mint,
                                    "???",
                                    pnl_percent,
                                    0.0,
                                    "Take Profit",
                                ).await;
                            }
                            execution::positions::PositionAction::StopLoss {
                                position_id,
                                ref mint,
                                pnl_percent,
                            } => {
                                #[cfg(feature = "live-trading")]
                                {
                                    if let Ok(Some(pos)) = PositionRepo::get_by_id(position_mgr.db(), position_id) {
                                        match router.sell(mint, pos.token_amount.unwrap_or(0) as u64).await {
                                            Ok(sig) => {
                                                position_mgr.close_position(position_id, "sl", Some(&sig)).await.ok();
                                            }
                                            Err(e) => {
                                                tracing::error!(id = position_id, error = %e, "SL sell failed");
                                            }
                                        }
                                    }
                                }
                                #[cfg(not(feature = "live-trading"))]
                                {
                                    position_mgr.close_position(position_id, "sl", None).await.ok();
                                }

                                monitor_telegram.send_position_update(
                                    mint,
                                    "???",
                                    pnl_percent,
                                    0.0,
                                    "Stop Loss",
                                ).await;
                            }
                            execution::positions::PositionAction::TrailingStop {
                                position_id,
                                ref mint,
                                highest_pnl,
                                current_pnl,
                            } => {
                                #[cfg(feature = "live-trading")]
                                {
                                    if let Ok(Some(pos)) = PositionRepo::get_by_id(position_mgr.db(), position_id) {
                                        match router.sell(mint, pos.token_amount.unwrap_or(0) as u64).await {
                                            Ok(sig) => {
                                                position_mgr.close_position(position_id, "trailing", Some(&sig)).await.ok();
                                            }
                                            Err(e) => {
                                                tracing::error!(id = position_id, error = %e, "Trailing sell failed");
                                            }
                                        }
                                    }
                                }
                                #[cfg(not(feature = "live-trading"))]
                                {
                                    position_mgr.close_position(position_id, "trailing", None).await.ok();
                                }

                                monitor_telegram.send_position_update(
                                    mint,
                                    "???",
                                    current_pnl,
                                    0.0,
                                    "Trailing Stop",
                                ).await;
                            }
                            _ => {}
                        }

                        // Generate lesson from closed positions.
                        if let Ok(Some(pos)) = PositionRepo::get_by_id(&pool, action.position_id()) {
                            if pos.status == "closed" {
                                analyzer.analyze(&pos).ok();
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Position monitor cycle failed");
                }
            }
        }
    });

    // ── Start Telegram bot ───────────────────────────────────────────────
    telegram_bot.run().await;

    // Wait for background tasks (these run indefinitely, so they'll
    // only complete on error or shutdown).
    let _ = tokio::try_join!(processing_handle, monitor_handle);

    Ok(())
}

/// Helper trait to extract position_id from PositionAction.
trait PositionActionExt {
    fn position_id(&self) -> i64;
}

impl PositionActionExt for execution::positions::PositionAction {
    fn position_id(&self) -> i64 {
        match self {
            execution::positions::PositionAction::TakeProfit { position_id, .. } => *position_id,
            execution::positions::PositionAction::StopLoss { position_id, .. } => *position_id,
            execution::positions::PositionAction::TrailingStop { position_id, .. } => *position_id,
            execution::positions::PositionAction::PartialTakeProfit { position_id, .. } => *position_id,
        }
    }
}
