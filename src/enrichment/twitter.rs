use reqwest::Client;
use std::time::Duration;

use crate::config::AppConfig;
use crate::error::Result;
use crate::utils::retry::retry_with_backoff;

/// Fetches Twitter/X narrative data via fxtwitter or similar proxy.
pub struct TwitterFetcher {
    http: Client,
    fxtwitter_base: String,
}

impl TwitterFetcher {
    pub fn new(_config: &AppConfig) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("Failed to build Twitter HTTP client"),
            fxtwitter_base: "https://api.fxtwitter.com".to_string(),
        }
    }

    /// Fetch social narrative data for a token.  This is a best-effort
    /// enrichment — failures are logged but not propagated as errors.
    pub async fn fetch_narrative(&self, _mint: &str) -> Result<Option<serde_json::Value>> {
        // In the original Node.js code, this searches for tweets mentioning
        // the token's ticker or CA.  The actual implementation depends on
        // the available Twitter/X API access.
        //
        // For now, return None — this can be extended with Twitter API v2,
        // fxtwitter, or a custom social-sentiment service.
        Ok(None)
    }

    /// Search for recent tweets about a token symbol.
    pub async fn search_tweets(&self, query: &str) -> Result<Vec<TweetData>> {
        retry_with_backoff(1, || async {
            let url = format!("{}/search/{}", self.fxtwitter_base, query);
            let resp = self.http.get(&url).send().await?;

            if resp.status().is_success() {
                let data: serde_json::Value = resp.json().await.unwrap_or_default();
                let tweets = parse_tweets(&data);
                Ok(tweets)
            } else {
                Ok(vec![])
            }
        })
        .await
    }
}

fn parse_tweets(_data: &serde_json::Value) -> Vec<TweetData> {
    // Placeholder — parse tweet objects from the API response.
    vec![]
}

/// Simplified tweet data.
#[derive(Debug, Clone)]
pub struct TweetData {
    pub text: String,
    pub author: String,
    pub likes: i64,
    pub retweets: i64,
    pub created_at: String,
}
