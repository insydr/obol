use crate::db::models::{Candidate, StrategyConfig};
use crate::error::{CharonError, Result};

/// Applies the strategy gate filters to a candidate.  Returns `Ok(())` if
/// the candidate passes all gates, or `Err` with the rejection reason.
pub struct CandidateBuilder;

impl CandidateBuilder {
    /// Run all strategy filters against a candidate.
    pub fn apply_filters(candidate: &Candidate, strategy: &StrategyConfig) -> Result<()> {
        // Source count gate.
        if candidate.source_count < strategy.min_source_count {
            return Err(CharonError::StrategyRejected(format!(
                "Source count {} < minimum {}",
                candidate.source_count, strategy.min_source_count
            )));
        }

        // Fee claim gate.
        if candidate.fee_claim_count < strategy.min_fee_claims && strategy.min_fee_claims > 0 {
            return Err(CharonError::StrategyRejected(format!(
                "Fee claims {} < minimum {}",
                candidate.fee_claim_count, strategy.min_fee_claims
            )));
        }

        // Market cap gate (lower bound).
        if let Some(mc) = candidate.market_cap_sol {
            if mc < strategy.min_market_cap_sol {
                return Err(CharonError::StrategyRejected(format!(
                    "Market cap {:.2} SOL < minimum {:.2} SOL",
                    mc, strategy.min_market_cap_sol
                )));
            }
            if mc > strategy.max_market_cap_sol {
                return Err(CharonError::StrategyRejected(format!(
                    "Market cap {:.2} SOL > maximum {:.2} SOL",
                    mc, strategy.max_market_cap_sol
                )));
            }
        }

        // Holder count gate.
        if let Some(holders) = candidate.holder_count {
            if holders < strategy.min_holders {
                return Err(CharonError::StrategyRejected(format!(
                    "Holders {} < minimum {}",
                    holders, strategy.min_holders
                )));
            }
        }

        // Top holder concentration gate.
        if let (Some(top_pct), Some(holders)) = (candidate.top_holder_pct, candidate.holder_count) {
            if holders >= strategy.min_holders && top_pct > strategy.max_top_holder_pct {
                return Err(CharonError::StrategyRejected(format!(
                    "Top holder {:.1}% > max {:.1}%",
                    top_pct, strategy.max_top_holder_pct
                )));
            }
        }

        // ATH distance gate.
        if let (Some(ath_dist), Some(max_ath)) = (candidate.ath_distance_pct, strategy.max_ath_distance_pct) {
            if ath_dist > max_ath {
                return Err(CharonError::StrategyRejected(format!(
                    "ATH distance {:.1}% > max {:.1}%",
                    ath_dist, max_ath
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candidate() -> Candidate {
        Candidate {
            id: 1,
            mint: "So11111111111111111111111111111111111111112".to_string(),
            name: Some("Test Token".to_string()),
            symbol: Some("TEST".to_string()),
            uri: None,
            source_count: 2,
            sources: Some(r#"["fee_claim","graduated"]"#.to_string()),
            market_cap_sol: Some(50.0),
            market_cap_usd: Some(5000.0),
            holder_count: Some(200),
            top_holder_pct: Some(15.0),
            ath_distance_pct: None,
            fee_claim_count: 2,
            first_seen_at: "2024-01-01".to_string(),
            last_updated_at: "2024-01-01".to_string(),
            status: "pending".to_string(),
        }
    }

    #[test]
    fn test_candidate_passes_all_filters() {
        let candidate = make_candidate();
        let strategy = StrategyConfig::default();
        assert!(CandidateBuilder::apply_filters(&candidate, &strategy).is_ok());
    }

    #[test]
    fn test_candidate_rejected_low_source_count() {
        let mut candidate = make_candidate();
        candidate.source_count = 1;
        let strategy = StrategyConfig::default();
        assert!(CandidateBuilder::apply_filters(&candidate, &strategy).is_err());
    }

    #[test]
    fn test_candidate_rejected_low_market_cap() {
        let mut candidate = make_candidate();
        candidate.market_cap_sol = Some(2.0);
        let strategy = StrategyConfig::default();
        assert!(CandidateBuilder::apply_filters(&candidate, &strategy).is_err());
    }

    #[test]
    fn test_candidate_rejected_high_top_holder() {
        let mut candidate = make_candidate();
        candidate.top_holder_pct = Some(50.0);
        let strategy = StrategyConfig::default();
        assert!(CandidateBuilder::apply_filters(&candidate, &strategy).is_err());
    }
}
