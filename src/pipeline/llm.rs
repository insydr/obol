use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use crate::config::AppConfig;
use crate::db::models::EnrichedCandidate;
use crate::error::{CharonError, Result};
use crate::utils::retry::retry_with_backoff;

/// LLM decision output for a single candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmDecision {
    pub mint: String,
    pub action: String,        // "approve", "reject", "skip"
    pub confidence: Option<f64>,
    pub reasoning: Option<String>,
}

/// OpenAI-compatible chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    content: String,
}

/// Trait for LLM providers — allows swapping between OpenAI, MiniMax,
/// local models, or mock implementations.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn screen_candidates(&self, batch: &[EnrichedCandidate]) -> Result<Vec<LlmDecision>>;
}

/// Concrete LLM client using an OpenAI-compatible endpoint.
pub struct LlmClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    pick_count: usize,
    timeout: Duration,
}

impl LlmClient {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(config.llm_timeout_secs + 5))
                .build()
                .expect("Failed to build LLM HTTP client"),
            base_url: config.llm_base_url.clone(),
            api_key: config.llm_api_key.clone(),
            model: config.llm_model.clone(),
            pick_count: config.llm_candidate_pick_count,
            timeout: Duration::from_secs(config.llm_timeout_secs),
        }
    }

    fn build_prompt(&self, batch: &[EnrichedCandidate]) -> String {
        let mut prompt = String::from(
            "You are a Solana meme-token analyst. For each candidate token below, \
             decide whether to APPROVE or REJECT it for trading. Consider: \
             market cap, holder distribution, social signals, fee claims, \
             and overall risk/reward. Return a JSON array of decisions.\n\n\
             Candidates:\n",
        );

        for (i, c) in batch.iter().enumerate() {
            prompt.push_str(&format!(
                "--- Candidate {} ---\n\
                 Mint: {}\n\
                 Symbol: {}\n\
                 Source count: {}\n\
                 Market cap (SOL): {:?}\n\
                 Holders: {:?}\n\
                 Top holder %: {:?}\n\
                 Fee claims: {}\n\
                 GMGN data: {}\n\
                 Twitter data: {}\n\n",
                i + 1,
                c.candidate.mint,
                c.candidate.symbol.as_deref().unwrap_or("unknown"),
                c.candidate.source_count,
                c.candidate.market_cap_sol,
                c.candidate.holder_count,
                c.candidate.top_holder_pct,
                c.candidate.fee_claim_count,
                c.gmgn_data
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "N/A".to_string()),
                c.twitter_data
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "N/A".to_string()),
            ));
        }

        prompt.push_str(&format!(
            "\nRespond with a JSON array of objects: \
             [{{\"mint\": \"...\", \"action\": \"approve\"|\"reject\", \
             \"confidence\": 0.0-1.0, \"reasoning\": \"...\"}}]\n\
             Pick at most {} candidates to approve.\n",
            self.pick_count
        ));

        prompt
    }
}

#[async_trait]
impl LlmProvider for LlmClient {
    async fn screen_candidates(&self, batch: &[EnrichedCandidate]) -> Result<Vec<LlmDecision>> {
        if batch.is_empty() {
            return Ok(vec![]);
        }

        let prompt = self.build_prompt(batch);

        retry_with_backoff(2, || async {
            let mut request = self.http.post(format!("{}/chat/completions", self.base_url));

            if let Some(ref key) = self.api_key {
                request = request.header("Authorization", format!("Bearer {}", key));
            }

            let body = serde_json::json!({
                "model": self.model,
                "messages": [
                    {"role": "system", "content": "You are a Solana meme-token analyst. Return only valid JSON."},
                    {"role": "user", "content": prompt}
                ],
                "temperature": 0.3,
                "max_tokens": 2048,
            });

            let resp = request.json(&body).send().await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(CharonError::Llm(format!(
                    "LLM API error {}: {}",
                    status, text
                )));
            }

            let chat_resp: ChatCompletionResponse = resp.json().await?;
            let content = chat_resp
                .choices
                .first()
                .map(|c| c.message.content.clone())
                .unwrap_or_default();

            // Parse the JSON decisions from the LLM response.
            let decisions: Vec<LlmDecision> = parse_llm_decisions(&content);
            Ok(decisions)
        })
        .await
    }
}

/// Attempt to parse LLM decisions from the response text.  The LLM may
/// wrap the JSON in markdown code blocks or add extra text.
fn parse_llm_decisions(content: &str) -> Vec<LlmDecision> {
    // Try to extract JSON from markdown code blocks first.
    let json_str = if let Some(start) = content.find("```json") {
        let start = start + 7;
        if let Some(end) = content[start..].find("```") {
            &content[start..start + end]
        } else {
            content
        }
    } else if let Some(start) = content.find('[') {
        if let Some(end) = content.rfind(']') {
            &content[start..=end]
        } else {
            content
        }
    } else {
        content
    };

    serde_json::from_str(json_str).unwrap_or_default()
}

/// A mock LLM provider that approves everything — useful for testing.
pub struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn screen_candidates(&self, batch: &[EnrichedCandidate]) -> Result<Vec<LlmDecision>> {
        Ok(batch
            .iter()
            .map(|c| LlmDecision {
                mint: c.candidate.mint.clone(),
                action: "approve".to_string(),
                confidence: Some(0.8),
                reasoning: Some("Mock approval".to_string()),
            })
            .collect())
    }
}
