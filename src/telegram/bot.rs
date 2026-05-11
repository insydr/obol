use std::sync::Arc;

use teloxide::{
    prelude::*,
    utils::command::BotCommands,
};
use tokio::sync::RwLock;

use crate::config::AppConfig;
use crate::db::{CandidateRepo, DbPool, PositionRepo, StrategyRepo};
use crate::pipeline::orchestrator::PipelineOrchestrator;
use crate::telegram::commands::{Command, handle_command};
use crate::telegram::formatting::TelegramFormatter;

/// The Telegram bot wrapper.  Manages command routing and message delivery.
pub struct CharonBot {
    bot: Bot,
    chat_id: ChatId,
    db: DbPool,
    config: Arc<AppConfig>,
    orchestrator: Arc<PipelineOrchestrator>,
    formatter: TelegramFormatter,
}

impl CharonBot {
    pub fn new(
        config: Arc<AppConfig>,
        db: DbPool,
        orchestrator: Arc<PipelineOrchestrator>,
    ) -> Self {
        let bot = Bot::new(&config.telegram_bot_token);
        let chat_id = ChatId(config.telegram_chat_id);
        Self {
            bot,
            chat_id,
            db,
            config,
            orchestrator,
            formatter: TelegramFormatter,
        }
    }

    /// Start the bot's command listener.
    pub async fn run(self: Arc<Self>) {
        tracing::info!("Starting Telegram bot...");

        let handler = Update::filter_message()
            .branch(
                dptree::entry()
                    .filter_command::<Command>()
                    .endpoint(move |bot: Bot, msg: Message, cmd: Command| {
                        let this = self.clone();
                        async move {
                            handle_command(bot, msg, cmd, &this).await;
                        }
                    }),
            );

        let bot_clone = self.bot.clone();
        Dispatcher::builder(bot_clone, handler)
            .build()
            .dispatch()
            .await;
    }

    /// Send a message to the configured chat.
    pub async fn send_message(&self, text: &str) {
        if let Err(e) = self.bot.send_message(self.chat_id, text).await {
            tracing::error!("Failed to send Telegram message: {}", e);
        }
    }

    /// Send a candidate alert for manual confirmation.
    pub async fn send_candidate_alert(
        &self,
        mint: &str,
        symbol: &str,
        buy_sol: f64,
        sources: &[String],
        market_cap_sol: Option<f64>,
    ) {
        let msg = self.formatter.format_candidate_alert(
            mint,
            symbol,
            buy_sol,
            sources,
            market_cap_sol,
        );
        self.send_message(&msg).await;
    }

    /// Send a position update notification.
    pub async fn send_position_update(
        &self,
        mint: &str,
        symbol: &str,
        pnl_percent: f64,
        pnl_sol: f64,
        action: &str,
    ) {
        let msg = self.formatter.format_position_update(
            mint, symbol, pnl_percent, pnl_sol, action,
        );
        self.send_message(&msg).await;
    }

    /// Access the database pool.
    pub fn db(&self) -> &DbPool {
        &self.db
    }

    /// Access the orchestrator.
    pub fn orchestrator(&self) -> &Arc<PipelineOrchestrator> {
        &self.orchestrator
    }

    /// Access the config.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }
}
