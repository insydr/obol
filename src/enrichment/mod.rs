pub mod jupiter;
pub mod gmgn;
pub mod twitter;

use std::sync::Arc;

use crate::config::AppConfig;
use crate::error::Result;

/// Aggregated enrichment data for a token.
#[derive(Debug, Clone)]
pub struct EnrichmentData {
    pub market_cap_sol: Option<f64>,
    pub market_cap_usd: Option<f64>,
    pub holder_count: Option<i32>,
    pub top_holder_pct: Option<f64>,
    pub ath_distance_pct: Option<f64>,
    pub jupiter: Option<serde_json::Value>,
    pub gmgn: Option<serde_json::Value>,
    pub twitter: Option<serde_json::Value>,
}

/// Service that coordinates enrichment from multiple sources.
pub struct EnrichmentService {
    jupiter: jupiter::JupiterClient,
    gmgn: gmgn::GmgnClient,
    twitter: twitter::TwitterFetcher,
}

impl EnrichmentService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            jupiter: jupiter::JupiterClient::new(config),
            gmgn: gmgn::GmgnClient::new(config),
            twitter: twitter::TwitterFetcher::new(config),
        }
    }

    /// Enrich a token by fetching data from all sources concurrently.
    pub async fn enrich(&self, mint: &str) -> Option<EnrichmentData> {
        let (jup_res, gmgn_res, tw_res) = tokio::join!(
            self.jupiter.fetch_token_info(mint),
            self.gmgn.fetch_token_info(mint),
            self.twitter.fetch_narrative(mint),
        );

        let jupiter_data = jup_res.ok().flatten();
        let gmgn_data = gmgn_res.ok().flatten();
        let twitter_data = tw_res.ok().flatten();

        // Extract common fields from whichever source provides them.
        let (market_cap_sol, market_cap_usd, holder_count, top_holder_pct, ath_distance_pct) =
            extract_fields(&jupiter_data, &gmgn_data);

        Some(EnrichmentData {
            market_cap_sol,
            market_cap_usd,
            holder_count,
            top_holder_pct,
            ath_distance_pct,
            jupiter: jupiter_data,
            gmgn: gmgn_data,
            twitter: twitter_data,
        })
    }
}

fn extract_fields(
    jupiter: &Option<serde_json::Value>,
    gmgn: &Option<serde_json::Value>,
) -> (Option<f64>, Option<f64>, Option<i32>, Option<f64>, Option<f64>) {
    let mc_sol = gmgn
        .as_ref()
        .and_then(|g| g.get("market_cap_sol"))
        .and_then(|v| v.as_f64())
        .or_else(|| {
            jupiter
                .as_ref()
                .and_then(|j| j.get("market_cap_sol"))
                .and_then(|v| v.as_f64())
        });

    let mc_usd = gmgn
        .as_ref()
        .and_then(|g| g.get("market_cap_usd"))
        .and_then(|v| v.as_f64());

    let holders = gmgn
        .as_ref()
        .and_then(|g| g.get("holder_count"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    let top_pct = gmgn
        .as_ref()
        .and_then(|g| g.get("top_holder_pct"))
        .and_then(|v| v.as_f64());

    let ath_dist = gmgn
        .as_ref()
        .and_then(|g| g.get("ath_dip_percent"))
        .and_then(|v| v.as_f64());

    (mc_sol, mc_usd, holders, top_pct, ath_dist)
}
