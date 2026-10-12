//! Amazon Bedrock Reranking — implements the `RerankingModel` trait.
//!
//! Aligned with Vercel AI SDK `AmazonBedrockRerankingModel`
//! (`reference/ai/packages/amazon-bedrock/src/reranking/amazon-bedrock-reranking-model.ts`).
//!
//! Uses the Bedrock Agent Runtime `/rerank` endpoint.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingRank, RerankingResponse,
    RerankingResult,
};
use aimux_core::shared::SharedProviderOptions;

use aimux_provider_utils::HttpBody;

use super::RegionFn;
use super::options;
use crate::shared::EndpointConfig;

/// Bedrock provider-specific reranking options.
#[derive(Debug, Clone, Default)]
struct BedrockRerankingOptions {
    next_token: Option<String>,
    additional_model_request_fields: Option<Value>,
}

fn parse_bedrock_reranking_options(
    provider_options: Option<&SharedProviderOptions>,
) -> BedrockRerankingOptions {
    let mut opts = BedrockRerankingOptions::default();
    if let Some(po) = provider_options {
        let bedrock = options::read(Some(po));
        if let Some(bedrock) = bedrock {
            if let Some(token) = bedrock.get("nextToken").and_then(|v| v.as_str()) {
                opts.next_token = Some(token.to_string());
            }
            if let Some(fields) = bedrock.get("additionalModelRequestFields") {
                opts.additional_model_request_fields = Some(fields.clone());
            }
        }
    }
    opts
}

/// The response from the Bedrock `/rerank` endpoint.
#[derive(Debug, Deserialize)]
struct BedrockRerankingResponse {
    results: Vec<BedrockRerankingResult>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct BedrockRerankingResult {
    index: u32,
    relevanceScore: f64,
}

/// An Amazon Bedrock reranking model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct BedrockRerankingModel {
    model_id: String,
    config: EndpointConfig,
    region: RegionFn,
}

impl BedrockRerankingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig, region: RegionFn) -> Self {
        Self {
            model_id,
            config,
            region,
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
        let bedrock_options = parse_bedrock_reranking_options(options.provider_options.as_ref());

        let region = (self.region)()?;
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
                    "numberOfResults": options.top_n
                },
                "type": "BEDROCK_RERANKING_MODEL"
            },
            "sources": sources
        });

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
            super::bedrock_failed_response_handler(),
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
