//! Google Gemini embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `GoogleEmbeddingModel`
//! (`reference/ai/packages/google/src/google-embedding-model.ts`).
//!
//! Uses two endpoints depending on the number of values:
//! - Single value: `POST {base_url}/models/{model}:embedContent`
//! - Multiple values: `POST {base_url}/models/{model}:batchEmbedContents`

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use super::options::google_options;
use crate::shared::EndpointConfig;

/// A Google Gemini embedding model (e.g. `"gemini-embedding-001"`).
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct GoogleEmbeddingModel {
    model_id: String,
    config: EndpointConfig,
}

impl GoogleEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl EmbeddingModel for GoogleEmbeddingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        Some(100)
    }

    fn supports_parallel_calls(&self) -> bool {
        true
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        let google_options = parse_google_provider_options(options.provider_options.as_ref());

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // For single embeddings, use the single endpoint.
        if options.values.len() == 1 {
            let value = &options.values[0];
            let mut parts = Map::new();
            parts.insert("text".to_string(), json!(value));

            let mut content = Map::new();
            content.insert(
                "parts".to_string(),
                Value::Array(vec![Value::Object(parts)]),
            );

            let mut body = Map::new();
            body.insert(
                "model".to_string(),
                json!(format!("models/{}", self.model_id)),
            );
            body.insert("content".to_string(), Value::Object(content));
            if let Some(dim) = google_options.output_dimensionality {
                body.insert("outputDimensionality".to_string(), json!(dim));
            }
            if let Some(task_type) = google_options.task_type {
                body.insert("taskType".to_string(), json!(task_type));
            }

            let url = exchange.url(&format!("/models/{}:embedContent", self.model_id));

            let resp = aimux_provider_utils::post_json_to_api(
                exchange.request(url, options),
                exchange.transform_body(Value::Object(body)),
                aimux_provider_utils::create_json_response_handler(),
                super::google_failed_response_handler(),
            )
            .await?;

            let response_headers = resp.response_headers;

            let raw_value: Value = resp.value;

            // Single embedding: response.embedding.values
            let embedding = raw_value
                .get("embedding")
                .and_then(|e| e.get("values"))
                .and_then(|v| v.as_array())
                .map(|vals| {
                    vals.iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect()
                })
                .unwrap_or_default();

            return Ok(EmbeddingResult {
                embeddings: vec![embedding],
                usage: None,
                provider_metadata: None,
                response: Some(EmbeddingResponse {
                    headers: Some(response_headers),
                    body: Some(raw_value),
                }),
                warnings: Vec::new(),
            });
        }

        // For multiple values, use the batch endpoint.
        let requests: Vec<Value> = options
            .values
            .iter()
            .map(|value| {
                let mut parts = Map::new();
                parts.insert("text".to_string(), json!(value));

                let mut content = Map::new();
                content.insert("role".to_string(), json!("user"));
                content.insert(
                    "parts".to_string(),
                    Value::Array(vec![Value::Object(parts)]),
                );

                let mut req = Map::new();
                req.insert(
                    "model".to_string(),
                    json!(format!("models/{}", self.model_id)),
                );
                req.insert("content".to_string(), Value::Object(content));
                if let Some(dim) = google_options.output_dimensionality {
                    req.insert("outputDimensionality".to_string(), json!(dim));
                }
                if let Some(task_type) = google_options.task_type.clone() {
                    req.insert("taskType".to_string(), json!(task_type));
                }
                Value::Object(req)
            })
            .collect();

        let mut body = Map::new();
        body.insert("requests".to_string(), Value::Array(requests));

        let url = exchange.url(&format!("/models/{}:batchEmbedContents", self.model_id));

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(url, options),
            exchange.transform_body(Value::Object(body)),
            aimux_provider_utils::create_json_response_handler(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_value: Value = resp.value;

        // Batch embeddings: response.embeddings[].values
        let embeddings: Vec<Vec<f32>> = raw_value
            .get("embeddings")
            .and_then(|e| e.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|item| {
                        item.get("values")
                            .and_then(|v| v.as_array())
                            .map(|vals| {
                                vals.iter()
                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(EmbeddingResult {
            embeddings,
            usage: None,
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

struct GoogleEmbeddingProviderOptions {
    output_dimensionality: Option<u32>,
    task_type: Option<String>,
}

fn parse_google_provider_options(
    options: Option<&SharedProviderOptions>,
) -> GoogleEmbeddingProviderOptions {
    let provider_opts = google_options(options);
    GoogleEmbeddingProviderOptions {
        output_dimensionality: provider_opts
            .and_then(|o| o.get("outputDimensionality"))
            .and_then(serde_json::Value::as_u64)
            .map(|d| d as u32),
        task_type: provider_opts
            .and_then(|o| o.get("taskType"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
    }
}
