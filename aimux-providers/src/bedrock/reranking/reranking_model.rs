//! Amazon Bedrock Reranking — implements the `RerankingModel` trait.
//!
//! Aligned with Vercel AI SDK `AmazonBedrockRerankingModel`
//! (`reference/ai/packages/amazon-bedrock/src/reranking/amazon-bedrock-reranking-model.ts`).
//!
//! Uses the Bedrock Agent Runtime `/rerank` endpoint.

use super::super::sigv4_fetch::Auth;
use super::reranking_api::BedrockRerankingResponse;
use super::reranking_model_options::parse_bedrock_reranking_options;
use crate::shared::EndpointConfig;
use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingRank, RerankingResponse,
    RerankingResult,
};
use aimux_provider_utils::HttpBody;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

/// An Amazon Bedrock reranking model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct BedrockRerankingModel {
    model_id: String,
    config: EndpointConfig,
    /// The region of a request (the model ARN names it), resolved per request.
    auth: Arc<Auth>,
}

impl BedrockRerankingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig, auth: Arc<Auth>) -> Self {
        Self {
            model_id,
            config,
            auth,
        }
    }
}

#[async_trait]
impl RerankingModel for BedrockRerankingModel {
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
        let bedrock_options = parse_bedrock_reranking_options(options.provider_options.as_ref())?;

        let region = self.auth.region()?;
        let model_arn = format!(
            "arn:aws:bedrock:{region}::foundation-model/{}",
            self.model_id
        );

        // Build sources array.
        let sources: Vec<Value> = match &options.documents {
            RerankingDocuments::Text { values } => values
                .iter()
                .map(|v| {
                    json!({
                        "type": "INLINE",
                        "inlineDocumentSource": {
                            "type": "TEXT",
                            "textDocument": { "text": v }
                        }
                    })
                })
                .collect(),
            RerankingDocuments::Object { values } => values
                .iter()
                .map(|v| {
                    json!({
                        "type": "INLINE",
                        "inlineDocumentSource": {
                            "type": "JSON",
                            "jsonDocument": v
                        }
                    })
                })
                .collect(),
        };

        let mut body = json!({
            "queries": [
                {
                    "textQuery": { "text": options.query },
                    "type": "TEXT"
                }
            ],
            "rerankingConfiguration": {
                "bedrockRerankingConfiguration": {
                    "modelConfiguration": {
                        "modelArn": model_arn
                    },
                },
                "type": "BEDROCK_RERANKING_MODEL"
            },
            "sources": sources
        });

        if let Some(top_n) = options.top_n {
            body["rerankingConfiguration"]["bedrockRerankingConfiguration"]["numberOfResults"] =
                json!(top_n);
        }

        if let Some(ref token) = bedrock_options.next_token {
            body["nextToken"] = json!(token);
        }
        if let Some(ref fields) = bedrock_options.additional_model_request_fields {
            body["rerankingConfiguration"]["bedrockRerankingConfiguration"]["modelConfiguration"]
                ["additionalModelRequestFields"] = fields.clone();
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body);
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = exchange.url("/rerank");

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(url, options),
            HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            aimux_provider_utils::create_json_response_handler::<BedrockRerankingResponse>(),
            super::super::error::bedrock_failed_response_handler(),
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
                relevance_score: r.relevanceScore,
            })
            .collect();

        Ok(RerankingResult {
            ranking,
            provider_metadata: None,
            warnings: None,
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
