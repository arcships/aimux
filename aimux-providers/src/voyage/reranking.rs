//! Voyage Reranking — implements the `RerankingModel` trait.
//!
//! Aligned with Vercel AI SDK `VoyageRerankingModel`
//! (`reference/ai/packages/voyage/src/reranking/voyage-reranking-model.ts`).

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingRank, RerankingResponse,
    RerankingResult,
};
use aimux_core::types::Warning;

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send};

use super::VoyageConfig;

/// Voyage provider-specific reranking options.
#[derive(Debug, Clone, Default)]
struct VoyageRerankingOptions {
    return_documents: Option<bool>,
    truncation: Option<bool>,
}

fn parse_voyage_reranking_options(
    provider_options: Option<&HashMap<String, Value>>,
) -> VoyageRerankingOptions {
    let mut opts = VoyageRerankingOptions::default();
    if let Some(po) = provider_options
        && let Some(voyage) = po.get("voyage")
    {
        if let Some(v) = voyage
            .get("returnDocuments")
            .and_then(serde_json::Value::as_bool)
        {
            opts.return_documents = Some(v);
        }
        if let Some(v) = voyage
            .get("truncation")
            .and_then(serde_json::Value::as_bool)
        {
            opts.truncation = Some(v);
        }
    }
    opts
}

/// The response from the Voyage `/rerank` endpoint.
#[derive(Debug, Deserialize)]
struct VoyageRerankingResponse {
    data: Vec<VoyageRerankingResult>,
}

#[derive(Debug, Deserialize)]
struct VoyageRerankingResult {
    index: u32,
    relevance_score: f64,
}

/// A Voyage reranking model.
pub struct VoyageRerankingModel {
    model_id: String,
    config: VoyageConfig,
}

impl VoyageRerankingModel {
    #[must_use]
    pub fn new(model_id: String, config: VoyageConfig) -> Self {
        Self { model_id, config }
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
impl RerankingModel for VoyageRerankingModel {
    fn provider(&self) -> &str {
        "voyage"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_rerank(
        &self,
        options: &RerankingCallOptions,
    ) -> Result<RerankingResult, AiMuxError> {
        let voyage_options = parse_voyage_reranking_options(options.provider_options.as_ref());

        let mut warnings = Vec::new();

        // Convert documents to strings.
        let documents: Vec<String> = match &options.documents {
            RerankingDocuments::Text { values } => values.clone(),
            RerankingDocuments::Object { values } => {
                warnings.push(Warning::Compatibility {
                    feature: "object documents".to_string(),
                    details: Some("Object documents are converted to strings.".to_string()),
                });
                values
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect()
            }
        };

        let mut body = json!({
            "query": options.query,
            "documents": documents,
            "model": self.model_id,
            "top_k": options.top_n,
        });

        if let Some(return_documents) = voyage_options.return_documents {
            body["return_documents"] = json!(return_documents);
        }
        if let Some(truncation) = voyage_options.truncation {
            body["truncation"] = json!(truncation);
        }

        let headers = self.build_headers(options.headers.as_ref());
        let mut header_list: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        header_list.push(("Content-Type".to_string(), "application/json".to_string()));

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: header_list,
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.headers;

        let raw_body: Value = serde_json::from_slice(&resp.body)?;

        let data: VoyageRerankingResponse = serde_json::from_value(raw_body.clone())?;

        let ranking: Vec<RerankingRank> = data
            .data
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
                id: None,
                timestamp: None,
                model_id: None,
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
