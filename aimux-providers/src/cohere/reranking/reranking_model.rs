//! Cohere Reranking — implements the `RerankingModel` trait.
//!
//! Aligned with Vercel AI SDK `CohereRerankingModel`
//! (`reference/ai/packages/cohere/src/reranking/cohere-reranking-model.ts`).

use async_trait::async_trait;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingRank, RerankingResponse,
    RerankingResult,
};
use aimux_core::types::Warning;

use crate::shared::EndpointConfig;

use super::reranking_api::CohereRerankingResponse;
use super::reranking_model_options::parse_cohere_reranking_options;

/// A Cohere reranking model.
pub struct CohereRerankingModel {
    model_id: String,
    config: EndpointConfig,
}

impl CohereRerankingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl RerankingModel for CohereRerankingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_rerank(
        &self,
        options: &RerankingCallOptions,
    ) -> Result<RerankingResult, AiMuxError> {
        let cohere_options = parse_cohere_reranking_options(options.provider_options.as_ref())?;

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
            "model": self.model_id,
            "query": options.query,
            "documents": documents,
        });

        if let Some(top_n) = options.top_n {
            body["top_n"] = json!(top_n);
        }
        if let Some(max_tokens) = cohere_options.max_tokens_per_doc {
            body["max_tokens_per_doc"] = json!(max_tokens);
        }
        if let Some(priority) = cohere_options.priority {
            body["priority"] = json!(priority);
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body);

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/rerank"), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<CohereRerankingResponse>(),
            super::super::error::cohere_failed_response_handler(),
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

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
