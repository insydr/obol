use crate::db::models::Position;
use crate::db::{DbPool, LessonRepo};
use crate::error::Result;

/// Post-trade analyzer that generates lessons from closed positions.
/// Lessons feed back into strategy refinement and are stored for
/// future reference.
pub struct TradeAnalyzer {
    db: DbPool,
}

impl TradeAnalyzer {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// Analyze a closed position and generate a lesson.
    pub fn analyze(&self, position: &Position) -> Result<i64> {
        let lesson_type = if position.pnl_sol > 0.0 {
            "win"
        } else if position.pnl_sol < 0.0 {
            "loss"
        } else {
            "neutral"
        };

        let summary = self.generate_summary(position, lesson_type);
        let tags = self.generate_tags(position, lesson_type);

        LessonRepo::insert(
            &self.db,
            Some(position.id),
            &position.mint,
            lesson_type,
            &summary,
            Some(&tags),
        )
    }

    fn generate_summary(&self, position: &Position, lesson_type: &str) -> String {
        let symbol = position.symbol.as_deref().unwrap_or("???");
        let close_reason = position.close_reason.as_deref().unwrap_or("unknown");

        match lesson_type {
            "win" => format!(
                "Win on {} ({}) — PnL: {:.1}% ({:.4} SOL). \
                 Entry: {:?} SOL, Exit reason: {}. \
                 Consider whether strategy filters correctly identified this opportunity.",
                symbol,
                &position.mint[..8],
                position.pnl_percent,
                position.pnl_sol,
                position.entry_price,
                close_reason,
            ),
            "loss" => format!(
                "Loss on {} ({}) — PnL: {:.1}% ({:.4} SOL). \
                 Entry: {:?} SOL, Exit reason: {}. \
                 Review: Were the initial signals too weak? Was SL appropriate?",
                symbol,
                &position.mint[..8],
                position.pnl_percent,
                position.pnl_sol,
                position.entry_price,
                close_reason,
            ),
            _ => format!(
                "Neutral outcome on {} ({}) — PnL: {:.1}% ({:.4} SOL). \
                 Exit reason: {}.",
                symbol,
                &position.mint[..8],
                position.pnl_percent,
                position.pnl_sol,
                close_reason,
            ),
        }
    }

    fn generate_tags(&self, position: &Position, lesson_type: &str) -> String {
        let mut tags = vec![lesson_type.to_string()];

        if let Some(reason) = &position.close_reason {
            tags.push(reason.clone());
        }

        if position.pnl_percent > 100.0 {
            tags.push("big_win".to_string());
        } else if position.pnl_percent < -30.0 {
            tags.push("big_loss".to_string());
        }

        if position.buy_sol > 0.5 {
            tags.push("large_position".to_string());
        }

        if position.trailing_activated {
            tags.push("trailing_stop".to_string());
        }

        serde_json::to_string(&tags).unwrap_or_default()
    }
}
