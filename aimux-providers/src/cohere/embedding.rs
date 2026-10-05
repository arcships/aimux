//! Cohere embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `CohereEmbeddingModel`
//! (`reference/ai/packages/cohere/src/cohere-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/embed`

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use crate::shared::EndpointConfig;

/// A Cohere embedding model (e.g. `"embed-english-v3.0"`).
pub struct CohereEmbeddingModel {
    model_id: String,
    config: EndpointConfig,
}

impl CohereEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl EmbeddingModel for CohereEmbeddingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        Some(96)
    }

    fn supports_parallel_calls(&self) -> bool {
        true
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        let cohere_options = parse_cohere_provider_options(options.provider_options.as_ref());

        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model_id));
        body.insert("embedding_types".to_string(), json!(["float"]));
        body.insert("texts".to_string(), json!(options.values));
        // Default input_type is "search_query" when not provided.
        body.insert(
            "input_type".to_string(),
            json!(
                cohere_options
                    .input_type
                    .unwrap_or_else(|| "search_query".to_string())
            ),
        );
        if let Some(truncate) = cohere_options.truncate {
            body.insert("truncate".to_string(), json!(truncate));
        }
        if let Some(output_dimension) = cohere_options.output_dimension {
            body.insert("output_dimension".to_string(), json!(output_dimension));
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/embed"), options),
            exchange.transform_body(Value::Object(body)),
            aimux_provider_utils::create_json_response_handler(),
            super::cohere_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_value: Value = resp.value;

        // Extract embeddings: response.embeddings.float
        let embeddings: Vec<Vec<f32>> = raw_value
            .get("embeddings")
            .and_then(|e| e.get("float"))
            .and_then(|f| f.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|row| {
                        row.as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Extract usage: response.meta.billed_units.input_tokens
        let usage = raw_value
            .get("meta")
            .and_then(|m| m.get("billed_units"))
            .and_then(|b| b.get("input_tokens"))
            .and_then(serde_json::Value::as_u64)
            .map(|tokens| EmbeddingUsage {
                tokens: tokens as u32,
            })
            .unwrap_or_default();

        Ok(EmbeddingResult {
            embeddings,
            usage: Some(usage),
            provider_metadata: None,
            response: Some(EmbeddingResponse {
                headers: Some(response_headers),
                body: Some(raw_value),
            }),
            warnings: Vec::new(),
        })
    }
}

// ── Provider options parsing ─────────────────────────────────────────────────

struct CohereEmbeddingProviderOptions {
    input_type: Option<String>,
    truncate: Option<String>,
    output_dimension: Option<u32>,
}

fn parse_cohere_provider_options(
    options: Option<&SharedProviderOptions>,
) -> CohereEmbeddingProviderOptions {
    let provider_opts = super::options::cohere_options(options);
    CohereEmbeddingProviderOptions {
        input_type: provider_opts
            .and_then(|o| o.get("inputType"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        truncate: provider_opts
            .and_then(|o| o.get("truncate"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        output_dimension: provider_opts
            .and_then(|o| o.get("outputDimension"))
            .and_then(serde_json::Value::as_u64)
            .map(|d| d as u32),
    }
}
