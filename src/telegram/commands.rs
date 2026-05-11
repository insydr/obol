use teloxide::{
    prelude::*,
    utils::command::BotCommands,
};

use crate::db::{CandidateRepo, PositionRepo, StrategyRepo};
use crate::telegram::bot::CharonBot;

/// Telegram bot commands — mirrors the original Node.js command set.
#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "Charon Trading Bot Commands"
)]
pub enum Command {
    #[command(description = "Show the main menu")]
    Menu,
    #[command(description = "Show current strategy settings")]
    Strategy,
    #[command(description = "Set a strategy parameter: /stratset <field> <value>")]
    StratSet(String),
    #[command(description = "Show open positions")]
    Positions,
    #[command(description = "Show recent candidates")]
    Candidates,
    #[command(description = "Show bot status and wallet info")]
    Status,
    #[command(description = "Show recent lessons")]
    Lessons,
    #[command(description = "Reload strategy from database")]
    Reload,
    #[command(description = "Show help")]
    Help,
}

/// Handle a parsed command.
pub async fn handle_command(bot: Bot, msg: Message, cmd: Command, charon: &CharonBot) {
    let chat_id = msg.chat.id;

    let response = match cmd {
        Command::Menu => format_menu(),
        Command::Strategy => format_strategy(charon.db()),
        Command::StratSet(args) => handle_strat_set(args, charon),
        Command::Positions => format_positions(charon.db()),
        Command::Candidates => format_candidates(charon.db()),
        Command::Status => format_status(charon),
        Command::Lessons => format_lessons(charon.db()),
        Command::Reload => {
            charon.orchestrator().reload_strategy().await;
            "Strategy reloaded from database.".to_string()
        }
        Command::Help => format_help(),
    };

    if let Err(e) = bot.send_message(ChatId(chat_id), response).await {
        tracing::error!("Failed to send command response: {}", e);
    }
}

fn format_menu() -> String {
    String::from(
        "🚀 **Charon Trading Bot**\n\n\
         /strategy - View strategy settings\n\
         /stratset <field> <value> - Update strategy\n\
         /positions - Open positions\n\
         /candidates - Recent candidates\n\
         /status - Bot status\n\
         /lessons - Trade lessons\n\
         /reload - Reload strategy\n\
         /help - Show help",
    )
}

fn format_strategy(db: &crate::db::DbPool) -> String {
    match StrategyRepo::get_config(db, "sniper") {
        Ok(config) => {
            format!(
                "📊 **Strategy: Sniper**\n\n\
                 Min source count: {}\n\
                 Min fee claims: {}\n\
                 Market cap: {:.1} - {:.1} SOL\n\
                 Min holders: {}\n\
                 Max top holder: {:.1}%\n\
                 Buy SOL: {:.3}\n\
                 TP: {:.1}%  |  SL: {:.1}%\n\
                 Trailing stop: {}\n\
                 LLM approval: {}",
                config.min_source_count,
                config.min_fee_claims,
                config.min_market_cap_sol,
                config.max_market_cap_sol,
                config.min_holders,
                config.max_top_holder_pct,
                config.buy_sol,
                config.tp_percent,
                config.sl_percent,
                config
                    .trailing_stop_pct
                    .map(|t| format!("{:.1}%", t))
                    .unwrap_or_else(|| "disabled".to_string()),
                config.require_llm_approval,
            )
        }
        Err(e) => format!("Error loading strategy: {}", e),
    }
}

fn handle_strat_set(args: String, charon: &CharonBot) -> String {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        return "Usage: /stratset <field> <value>".to_string();
    }
    let field = parts[0];
    let value = parts[1..].join(" ");

    match StrategyRepo::set_field(charon.db(), "sniper", field, &value) {
        Ok(()) => {
            // Trigger hot-reload.
            format!("Strategy field '{}' updated to '{}'. Use /reload to apply.", field, value)
        }
        Err(e) => format!("Failed to update strategy: {}", e),
    }
}

fn format_positions(db: &crate::db::DbPool) -> String {
    match PositionRepo::list_open(db) {
        Ok(positions) => {
            if positions.is_empty() {
                return "No open positions.".to_string();
            }
            let mut msg = String::from("📈 **Open Positions**\n\n");
            for p in &positions {
                let symbol = p.symbol.as_deref().unwrap_or("???");
                msg.push_str(&format!(
                    "• {} ({}) — {:.3} SOL | PnL: {:.1}% ({:.4} SOL)\n",
                    symbol, &p.mint[..8], p.buy_sol, p.pnl_percent, p.pnl_sol
                ));
            }
            msg
        }
        Err(e) => format!("Error loading positions: {}", e),
    }
}

fn format_candidates(db: &crate::db::DbPool) -> String {
    match CandidateRepo::list_by_status(db, "approved") {
        Ok(candidates) => {
            if candidates.is_empty() {
                return "No approved candidates.".to_string();
            }
            let mut msg = String::from("🎯 **Approved Candidates**\n\n");
            for c in candidates.iter().take(10) {
                let symbol = c.symbol.as_deref().unwrap_or("???");
                msg.push_str(&format!(
                    "• {} ({}) — Sources: {} | MC: {:?} SOL\n",
                    symbol,
                    &c.mint[..8],
                    c.source_count,
                    c.market_cap_sol,
                ));
            }
            msg
        }
        Err(e) => format!("Error loading candidates: {}", e),
    }
}

fn format_status(charon: &CharonBot) -> String {
    let config = charon.config();
    let open_count = PositionRepo::count_open(charon.db()).unwrap_or(0);
    format!(
        "🤖 **Charon Status**\n\n\
         Mode: {}\n\
         LLM: {} ({})\n\
         Open positions: {}\n\
         Signal poll: {}ms\n\
         Position monitor: {}ms",
        config.trading_mode,
        if config.enable_llm { "ON" } else { "OFF" },
        config.llm_model,
        open_count,
        config.signal_poll_ms,
        config.position_monitor_ms,
    )
}

fn format_lessons(db: &crate::db::DbPool) -> String {
    match crate::db::LessonRepo::list_recent(db, 5) {
        Ok(lessons) => {
            if lessons.is_empty() {
                return "No lessons yet.".to_string();
            }
            let mut msg = String::from("📚 **Recent Lessons**\n\n");
            for l in &lessons {
                msg.push_str(&format!("• [{}] {} — {}\n", l.lesson_type, &l.mint[..8], l.summary));
            }
            msg
        }
        Err(e) => format!("Error loading lessons: {}", e),
    }
}

fn format_help() -> String {
    String::from(
        "📖 **Charon Help**\n\n\
         Charon is a Solana/Pump.fun trading bot that:\n\
         1. Monitors fee claims, graduated & trending tokens\n\
         2. Filters candidates using strategy gates\n\
         3. Optionally screens with LLM\n\
         4. Executes trades via Jupiter Ultra\n\n\
         Modes: dry_run → confirm → live\n\n\
         Strategy fields you can set:\n\
         min_source_count, min_fee_claims, min_market_cap_sol,\n\
         max_market_cap_sol, min_holders, max_top_holder_pct,\n\
         buy_sol, tp_percent, sl_percent",
    )
}
