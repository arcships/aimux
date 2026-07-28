//! Cohere Reranking — implements the `RerankingModel` trait.
//!
//! Aligned with Vercel AI SDK `CohereRerankingModel`
//! (`reference/ai/packages/cohere/src/reranking/cohere-reranking-model.ts`).

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingRank, RerankingResponse,
    RerankingResult,
};
use aimux_core::types::Warning;

use aimux_provider_utils::response::{DEFAULT_ERROR_STRUCTURE, parse_provider_error};

use super::CohereConfig;

/// Cohere provider-specific reranking options.
#[derive(Debug, Clone, Default)]
struct CohereRerankingOptions {
    max_tokens_per_doc: Option<u64>,
    priority: Option<u64>,
}

fn parse_cohere_reranking_options(
    provider_options: Option<&HashMap<String, Value>>,
) -> CohereRerankingOptions {
    let mut opts = CohereRerankingOptions::default();
    if let Some(po) = provider_options
        && let Some(cohere) = po.get("cohere")
    {
        if let Some(v) = cohere.get("maxTokensPerDoc").and_then(|v| v.as_u64()) {
            opts.max_tokens_per_doc = Some(v);
        }
        if let Some(v) = cohere.get("priority").and_then(|v| v.as_u64()) {
            opts.priority = Some(v);
        }
    }
    opts
}

/// The response from the Cohere `/rerank` endpoint.
#[derive(Debug, Deserialize)]
struct CohereRerankingResponse {
    #[serde(default)]
    id: Option<String>,
    results: Vec<CohereRerankingResult>,
}

#[derive(Debug, Deserialize)]
struct CohereRerankingResult {
    index: u32,
    relevance_score: f64,
}

/// A Cohere reranking model.
pub struct CohereRerankingModel {
    model_id: String,
    config: CohereConfig,
    client: Client,
}

impl CohereRerankingModel {
    pub fn new(model_id: String, config: CohereConfig, client: Client) -> Self {
        Self {
            model_id,
            config,
            client,
        }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/rerank", self.config.base_url)
    }
}

#[async_trait]
impl RerankingModel for CohereRerankingModel {
    fn provider(&self) -> &str {
        "cohere"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_rerank(
        &self,
        options: &RerankingCallOptions,
    ) -> Result<RerankingResult, AiMuxError> {
        let cohere_options = parse_cohere_reranking_options(options.provider_options.as_ref());

        let mut warnings = Vec::new();

        // Convert documents to strings.
        let documents: Vec<String> = match &options.documents {
            RerankingDocuments::Text { values } => values.clone(),
            RerankingDocuments::Object { values } => {
                warnings.push(Warning::Compatibility {
                    feature: "object documents".to_string(),
                    details: Some("Object documents are converted to strings.".to_string()),
                });
                values.iter().map(|v| v.to_string()).collect()
            }
        };

        let mut body = json!({
            "model": self.model_id,
            "query": options.query,
            "documents": documents,
            "top_n": options.top_n,
        });

        if let Some(max_tokens) = cohere_options.max_tokens_per_doc {
            body["max_tokens_per_doc"] = json!(max_tokens);
        }
        if let Some(priority) = cohere_options.priority {
            body["priority"] = json!(priority);
        }

        let headers = self.build_headers(options.headers.as_ref());
        let header_map: reqwest::header::HeaderMap = headers
            .iter()
            .filter_map(|(k, v)| {
                reqwest::header::HeaderName::try_from(k)
                    .ok()
                    .zip(reqwest::header::HeaderValue::try_from(v).ok())
            })
            .collect();

        let resp = self
            .client
            .post(self.endpoint())
            .header("Content-Type", "application/json")
            .headers(header_map)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &DEFAULT_ERROR_STRUCTURE,
            ));
        }

        // Capture response headers.
        let response_headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let raw_body: Value = resp
            .json()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let data: CohereRerankingResponse =
            serde_json::from_value(raw_body.clone()).map_err(|e| {
                AiMuxError::Provider(format!("failed to parse reranking response: {e}"))
            })?;

        let ranking: Vec<RerankingRank> = data
            .results
            .into_iter()
            .map(|r| RerankingRank {
                index: r.index,
                relevance_score: r.relevance_score,
            })
            .collect();

        Ok(RerankingResult {
            ranking,
            provider_metadata: None,
            warnings: Some(warnings),
            response: Some(RerankingResponse {
                id: data.id,
                timestamp: None,
                model_id: None,
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
