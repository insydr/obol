/// Telegram message formatting utilities.
pub struct TelegramFormatter;

impl TelegramFormatter {
    /// Format a candidate alert for manual confirmation.
    pub fn format_candidate_alert(
        &self,
        mint: &str,
        symbol: &str,
        buy_sol: f64,
        sources: &[String],
        market_cap_sol: Option<f64>,
    ) -> String {
        let sources_str = sources.join(", ");
        let mc_str = market_cap_sol
            .map(|mc| format!("{:.1} SOL", mc))
            .unwrap_or_else(|| "N/A".to_string());
        format!(
            "🎯 **New Candidate**\n\n\
             Symbol: {}\n\
             Mint: {}\n\
             Sources: {}\n\
             Market cap: {}\n\
             Proposed buy: {:.3} SOL\n\n\
             Use /confirm {} or /reject {}",
            symbol, mint, sources_str, mc_str, buy_sol, &mint[..8], &mint[..8]
        )
    }

    /// Format a position update notification.
    pub fn format_position_update(
        &self,
        mint: &str,
        symbol: &str,
        pnl_percent: f64,
        pnl_sol: f64,
        action: &str,
    ) -> String {
        let emoji = if pnl_percent >= 0.0 { "📈" } else { "📉" };
        format!(
            "{} **Position {}**\n\n\
             Symbol: {}\n\
             Mint: {}\n\
             PnL: {:.1}% ({:.4} SOL)\n\
             Action: {}",
            emoji, action, symbol, mint, pnl_percent, pnl_sol, action
        )
    }

    /// Format a trade execution confirmation.
    pub fn format_trade_exec(
        &self,
        mint: &str,
        action: &str,
        amount_sol: f64,
        tx_sig: &str,
    ) -> String {
        format!(
            "✅ **Trade Executed**\n\n\
             Action: {}\n\
             Mint: {}\n\
             Amount: {:.3} SOL\n\
             TX: {}",
            action, mint, amount_sol, tx_sig
        )
    }
}
