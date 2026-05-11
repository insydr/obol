pub mod jupiter;
pub mod gmgn;
pub mod twitter;

use std::sync::Arc;

use crate::config::AppConfig;
use crate::db::DbPool;
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

/// Service that coordinates enrichment from multiple sources with
/// database caching to avoid redundant API calls.
pub struct EnrichmentService {
    jupiter: jupiter::JupiterClient,
    gmgn: gmgn::GmgnClient,
    twitter: twitter::TwitterFetcher,
    db: Option<DbPool>,
    cache_ttl_secs: u64,
}

impl EnrichmentService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            jupiter: jupiter::JupiterClient::new(config),
            gmgn: gmgn::GmgnClient::new(config),
            twitter: twitter::TwitterFetcher::new(config),
            db: None,
            cache_ttl_secs: 300, // 5 minutes
        }
    }

    /// Set the database pool for caching.
    pub fn with_db(mut self, pool: DbPool) -> Self {
        self.db = Some(pool);
        self
    }

    /// Enrich a token by fetching data from all sources concurrently.
    /// Checks the cache first and skips API calls if fresh data exists.
    pub async fn enrich(&self, mint: &str) -> Option<EnrichmentData> {
        // Check cache first.
        if let Some(ref db) = self.db {
            if let Ok(Some(cached)) = Self::load_from_cache(db, mint, self.cache_ttl_secs) {
                tracing::debug!(mint = mint, "Using cached enrichment data");
                return Some(cached);
            }
        }

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

        let data = EnrichmentData {
            market_cap_sol,
            market_cap_usd,
            holder_count,
            top_holder_pct,
            ath_distance_pct,
            jupiter: jupiter_data,
            gmgn: gmgn_data,
            twitter: twitter_data,
        };

        // Store to cache.
        if let Some(ref db) = self.db {
            Self::save_to_cache(db, mint, &data);
        }

        Some(data)
    }

    /// Load enrichment data from the database cache if it's fresh enough.
    fn load_from_cache(db: &DbPool, mint: &str, ttl_secs: u64) -> Result<Option<EnrichmentData>> {
        let conn = db.get()?;
        let result = conn.query_row(
            "SELECT jupiter_data, gmgn_data, twitter_data, updated_at FROM enrichment_cache WHERE mint = ?1",
            rusqlite::params![mint],
            |row| {
                let jupiter: Option<String> = row.get(0)?;
                let gmgn: Option<String> = row.get(1)?;
                let twitter: Option<String> = row.get(2)?;
                let updated_at: String = row.get(3)?;
                Ok((jupiter, gmgn, twitter, updated_at))
            },
        ).ok();

        match result {
            Some((jupiter_str, gmgn_str, twitter_str, updated_at)) => {
                // Check if cache is still fresh.
                if let Ok(updated) = chrono::NaiveDateTime::parse_from_str(&updated_at, "%Y-%m-%d %H:%M:%S") {
                    let age = chrono::Utc::now().naive_utc() - updated;
                    if age.num_seconds() > ttl_secs as i64 {
                        return Ok(None); // Cache expired.
                    }
                }

                let jupiter_data = jupiter_str.and_then(|s| serde_json::from_str(&s).ok());
                let gmgn_data = gmgn_str.and_then(|s| serde_json::from_str(&s).ok());
                let twitter_data = twitter_str.and_then(|s| serde_json::from_str(&s).ok());

                let (market_cap_sol, market_cap_usd, holder_count, top_holder_pct, ath_distance_pct) =
                    extract_fields(&jupiter_data, &gmgn_data);

                Ok(Some(EnrichmentData {
                    market_cap_sol,
                    market_cap_usd,
                    holder_count,
                    top_holder_pct,
                    ath_distance_pct,
                    jupiter: jupiter_data,
                    gmgn: gmgn_data,
                    twitter: twitter_data,
                }))
            }
            None => Ok(None),
        }
    }

    /// Save enrichment data to the database cache.
    fn save_to_cache(db: &DbPool, mint: &str, data: &EnrichmentData) {
        if let Ok(conn) = db.get() {
            let jupiter_str = data.jupiter.as_ref().map(|v| v.to_string());
            let gmgn_str = data.gmgn.as_ref().map(|v| v.to_string());
            let twitter_str = data.twitter.as_ref().map(|v| v.to_string());

            let _ = conn.execute(
                "INSERT INTO enrichment_cache (mint, jupiter_data, gmgn_data, twitter_data, updated_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))
                 ON CONFLICT(mint) DO UPDATE SET jupiter_data = ?2, gmgn_data = ?3, twitter_data = ?4, updated_at = datetime('now')",
                rusqlite::params![mint, jupiter_str, gmgn_str, twitter_str],
            );
        }
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
