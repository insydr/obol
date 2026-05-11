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

use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use config::AppConfig;
use db::{CandidateRepo, DbPool, ExecutionLogRepo, PositionRepo, StrategyRepo};
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

    // Initialize tracing with structured spans.
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
    let enrichment = Arc::new(EnrichmentService::new(&config).with_db(pool.clone()));
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

    // ── Cancellation token for graceful shutdown ────────────────────────
    let cancel_token = CancellationToken::new();
    let cancel_token_clone = cancel_token.clone();

    // ── Signal channels ─────────────────────────────────────────────────
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

    // Price monitor — feed with mints from open positions.
    let price_monitor_mints = PositionRepo::get_open_mints(&pool).unwrap_or_default();
    let price_monitor = Arc::new(PriceMonitor::new(&config));
    price_monitor.spawn(signal_tx, price_monitor_mints);

    tracing::info!("All signal pollers started");

    // ── Pipeline concurrency limiter ────────────────────────────────────
    let pipeline_semaphore = Arc::new(Semaphore::new(
        config.pipeline_concurrency_limit as usize,
    ));

    // ── Main processing loop ────────────────────────────────────────────
    let processing_pool = pool.clone();
    let processing_orchestrator = orchestrator.clone();
    let processing_telegram = telegram_bot.clone();
    let processing_semaphore = pipeline_semaphore.clone();

    let processing_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                event = signal_rx.recv() => {
                    let event = match event {
                        Some(e) => e,
                        None => {
                            tracing::info!("Signal channel closed, stopping processing loop");
                            return;
                        }
                    };

                    tracing::debug!(source = %event.source, mint = %event.mint, "Received signal");

                    // Acquire a semaphore permit to limit concurrency.
                    let permit = match processing_semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => {
                            tracing::warn!("Pipeline semaphore closed, dropping signal");
                            continue;
                        }
                    };

                    let orch = processing_orchestrator.clone();
                    let tg = processing_telegram.clone();
                    let db = processing_pool.clone();

                    tokio::spawn(async move {
                        let _permit = permit; // Held until task completes.

                        match orch.process_signal(event).await {
                            Ok(outcome) => {
                                match outcome {
                                    pipeline::orchestrator::PipelineOutcome::DryRun(position) => {
                                        tg.send_message(&format!(
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
                                        tg.send_candidate_alert(
                                            enriched.candidate.id,
                                            &enriched.candidate.mint,
                                            enriched.candidate.symbol.as_deref().unwrap_or("???"),
                                            enriched.candidate.buy_sol,
                                            &sources,
                                            enriched.candidate.market_cap_sol,
                                        ).await;
                                    }
                                    pipeline::orchestrator::PipelineOutcome::Executed(position, tx_sig) => {
                                        tg.send_message(&format!(
                                            "✅ LIVE: Bought {} ({}) — {:.3} SOL | TX: {}...",
                                            position.symbol.as_deref().unwrap_or("???"),
                                            &position.mint[..8],
                                            position.buy_sol,
                                            &tx_sig[..16],
                                        )).await;
                                    }
                                    _ => {
                                        // Rejected, already processed, etc.
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Pipeline processing error");
                            }
                        }

                        // Drop db reference explicitly.
                        drop(db);
                    });
                }
                _ = cancel_token_clone.cancelled() => {
                    tracing::info!("Processing loop received shutdown signal");
                    return;
                }
            }
        }
    });

    // ── Position monitor loop ───────────────────────────────────────────
    let monitor_pool = pool.clone();
    let monitor_jupiter = jupiter.clone();
    let monitor_router = router.clone();
    let monitor_analyzer = analyzer.clone();
    let monitor_config = config.clone();
    let monitor_telegram = telegram_bot.clone();
    let monitor_cancel = cancel_token.clone();

    let monitor_handle = tokio::spawn(async move {
        let position_mgr = PositionManager::new(
            monitor_pool.clone(),
            monitor_jupiter,
            monitor_config.clone(),
        );

        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            monitor_config.position_monitor_ms,
        ));

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = monitor_cancel.cancelled() => {
                    tracing::info!("Position monitor received shutdown signal");
                    return;
                }
            }

            match position_mgr.monitor_cycle().await {
                Ok(actions) => {
                    for action in actions {
                        let position_id = action.position_id();

                        match action {
                            execution::positions::PositionAction::TakeProfit {
                                ref mint,
                                pnl_percent,
                                ..
                            } => {
                                // Fetch position for symbol and SOL PnL.
                                let (symbol, pnl_sol) = PositionRepo::get_by_id(&monitor_pool, position_id)
                                    .ok()
                                    .flatten()
                                    .map(|p| (
                                        p.symbol.unwrap_or_else(|| "???".to_string()),
                                        p.pnl_sol,
                                    ))
                                    .unwrap_or_else(|| ("???".to_string(), 0.0));

                                #[cfg(feature = "live-trading")]
                                {
                                    if let Ok(Some(pos)) = PositionRepo::get_by_id(&monitor_pool, position_id) {
                                        let token_amount = pos.token_amount.unwrap_or(0.0) as u64;
                                        if token_amount > 0 {
                                            // Log execution.
                                            let log_id = ExecutionLogRepo::insert(
                                                &monitor_pool, Some(position_id), mint, "sell_tp",
                                                None, Some(token_amount as f64), None, "pending", None,
                                            ).ok();

                                            match monitor_router.sell(mint, token_amount).await {
                                                Ok(sig) => {
                                                    position_mgr.close_position(position_id, "tp", Some(&sig)).await.ok();
                                                    if let Some(id) = log_id {
                                                        ExecutionLogRepo::mark_success(&monitor_pool, id, Some(&sig)).ok();
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::error!(id = position_id, error = %e, "TP sell failed");
                                                    if let Some(id) = log_id {
                                                        ExecutionLogRepo::mark_failed(&monitor_pool, id, &e.to_string()).ok();
                                                    }
                                                }
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
                                    &symbol,
                                    pnl_percent,
                                    pnl_sol,
                                    "Take Profit",
                                ).await;

                                // Generate lesson.
                                if let Ok(Some(pos)) = PositionRepo::get_by_id(&monitor_pool, position_id) {
                                    if pos.status == "closed" {
                                        monitor_analyzer.analyze(&pos).ok();
                                    }
                                }
                            }
                            execution::positions::PositionAction::StopLoss {
                                ref mint,
                                pnl_percent,
                                ..
                            } => {
                                let (symbol, pnl_sol) = PositionRepo::get_by_id(&monitor_pool, position_id)
                                    .ok()
                                    .flatten()
                                    .map(|p| (
                                        p.symbol.unwrap_or_else(|| "???".to_string()),
                                        p.pnl_sol,
                                    ))
                                    .unwrap_or_else(|| ("???".to_string(), 0.0));

                                #[cfg(feature = "live-trading")]
                                {
                                    if let Ok(Some(pos)) = PositionRepo::get_by_id(&monitor_pool, position_id) {
                                        let token_amount = pos.token_amount.unwrap_or(0.0) as u64;
                                        if token_amount > 0 {
                                            let log_id = ExecutionLogRepo::insert(
                                                &monitor_pool, Some(position_id), mint, "sell_sl",
                                                None, Some(token_amount as f64), None, "pending", None,
                                            ).ok();

                                            match monitor_router.sell(mint, token_amount).await {
                                                Ok(sig) => {
                                                    position_mgr.close_position(position_id, "sl", Some(&sig)).await.ok();
                                                    if let Some(id) = log_id {
                                                        ExecutionLogRepo::mark_success(&monitor_pool, id, Some(&sig)).ok();
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::error!(id = position_id, error = %e, "SL sell failed");
                                                    if let Some(id) = log_id {
                                                        ExecutionLogRepo::mark_failed(&monitor_pool, id, &e.to_string()).ok();
                                                    }
                                                }
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
                                    &symbol,
                                    pnl_percent,
                                    pnl_sol,
                                    "Stop Loss",
                                ).await;

                                if let Ok(Some(pos)) = PositionRepo::get_by_id(&monitor_pool, position_id) {
                                    if pos.status == "closed" {
                                        monitor_analyzer.analyze(&pos).ok();
                                    }
                                }
                            }
                            execution::positions::PositionAction::TrailingStop {
                                ref mint,
                                highest_pnl,
                                current_pnl,
                                ..
                            } => {
                                let (symbol, pnl_sol) = PositionRepo::get_by_id(&monitor_pool, position_id)
                                    .ok()
                                    .flatten()
                                    .map(|p| (
                                        p.symbol.unwrap_or_else(|| "???".to_string()),
                                        p.pnl_sol,
                                    ))
                                    .unwrap_or_else(|| ("???".to_string(), 0.0));

                                #[cfg(feature = "live-trading")]
                                {
                                    if let Ok(Some(pos)) = PositionRepo::get_by_id(&monitor_pool, position_id) {
                                        let token_amount = pos.token_amount.unwrap_or(0.0) as u64;
                                        if token_amount > 0 {
                                            let log_id = ExecutionLogRepo::insert(
                                                &monitor_pool, Some(position_id), mint, "sell_trailing",
                                                None, Some(token_amount as f64), None, "pending", None,
                                            ).ok();

                                            match monitor_router.sell(mint, token_amount).await {
                                                Ok(sig) => {
                                                    position_mgr.close_position(position_id, "trailing", Some(&sig)).await.ok();
                                                    if let Some(id) = log_id {
                                                        ExecutionLogRepo::mark_success(&monitor_pool, id, Some(&sig)).ok();
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::error!(id = position_id, error = %e, "Trailing sell failed");
                                                    if let Some(id) = log_id {
                                                        ExecutionLogRepo::mark_failed(&monitor_pool, id, &e.to_string()).ok();
                                                    }
                                                }
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
                                    &symbol,
                                    current_pnl,
                                    pnl_sol,
                                    "Trailing Stop",
                                ).await;

                                if let Ok(Some(pos)) = PositionRepo::get_by_id(&monitor_pool, position_id) {
                                    if pos.status == "closed" {
                                        monitor_analyzer.analyze(&pos).ok();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Position monitor cycle failed");
                }
            }
        }
    });

    // ── Graceful shutdown handler ───────────────────────────────────────
    let shutdown_token = cancel_token.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
        tracing::info!("Shutdown signal received (Ctrl+C), cleaning up...");
        shutdown_token.cancel();
    });

    // ── Start Telegram bot (blocking) ──────────────────────────────────
    // Run the Telegram bot in a separate task so we can wait for shutdown.
    let bot_token = cancel_token.clone();
    let bot_handle = tokio::spawn(async move {
        tokio::select! {
            _ = telegram_bot.run() => {}
            _ = bot_token.cancelled() => {
                tracing::info!("Telegram bot shutting down");
            }
        }
    });

    // Wait for shutdown to propagate.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Second Ctrl+C received, forcing exit");
        }
        _ = cancel_token.cancelled() => {
            // Give background tasks a moment to finish.
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        _ = processing_handle => {}
        _ = monitor_handle => {}
        _ = bot_handle => {}
    }

    tracing::info!("Charon-RS shutdown complete");
    Ok(())
}
