//! Google Vertex AI embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `GoogleVertexEmbeddingModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-embedding-model.ts`).
//!
//! Uses two endpoints depending on the model:
//! - `gemini-embedding-2` / `gemini-embedding-2-preview`:
//!   `POST {base_url}/models/{model}:embedContent` (single value only)
//! - Others: `POST {base_url}/models/{model}:predict` (batch)

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::google::google_failed_response_handler;
use crate::google::options::Namespace;
use crate::shared::EndpointConfig;

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

/// A Google Vertex AI embedding model (e.g. `"textembedding-gecko@001"`).
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct VertexEmbeddingModel {
    model_id: String,
    config: EndpointConfig,
}

impl VertexEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

/// Returns `true` for models that only support the `:embedContent` endpoint
/// (single value per call), not the `:predict` batch endpoint.
fn uses_embed_content_endpoint(model_id: &str) -> bool {
    model_id == "gemini-embedding-2" || model_id == "gemini-embedding-2-preview"
}

#[async_trait]
impl EmbeddingModel for VertexEmbeddingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        if uses_embed_content_endpoint(&self.model_id) {
            Some(1)
        } else {
            Some(2048)
        }
    }

    fn supports_parallel_calls(&self) -> bool {
        true
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        // Parse provider options: `googleVertex`, then `google`.
        let vertex_options = parse_vertex_provider_options(options.provider_options.as_ref());

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        if uses_embed_content_endpoint(&self.model_id) {
            // gemini-embedding-2: use :embedContent endpoint (single value).
            let mut parts = Map::new();
            parts.insert(
                "text".to_string(),
                json!(options.values.first().unwrap_or(&String::new())),
            );

            let mut content = Map::new();
            content.insert(
                "parts".to_string(),
                Value::Array(vec![Value::Object(parts)]),
            );

            let mut embed_config = Map::new();
            if let Some(dim) = vertex_options.output_dimensionality {
                embed_config.insert("outputDimensionality".to_string(), json!(dim));
            }
            if let Some(task_type) = &vertex_options.task_type {
                embed_config.insert("taskType".to_string(), json!(task_type));
            }
            if let Some(title) = &vertex_options.title {
                embed_config.insert("title".to_string(), json!(title));
            }
            if let Some(auto_truncate) = vertex_options.auto_truncate {
                embed_config.insert("autoTruncate".to_string(), json!(auto_truncate));
            }

            let mut body = Map::new();
            body.insert("content".to_string(), Value::Object(content));
            body.insert(
                "embedContentConfig".to_string(),
                Value::Object(embed_config),
            );

            let url = exchange.url(&format!("/models/{}:embedContent", self.model_id));

            let resp = aimux_provider_utils::post_json_to_api(
                exchange.request(url, options),
                exchange.transform_body(Value::Object(body)),
                aimux_provider_utils::create_json_response_handler(),
                google_failed_response_handler(),
            )
            .await?;

            let response_headers = resp.response_headers;

            let raw_value: Value = resp.value;

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

            let usage = raw_value
                .get("usageMetadata")
                .and_then(|u| u.get("promptTokenCount"))
                .and_then(serde_json::Value::as_u64)
                .map(|tokens| EmbeddingUsage {
                    tokens: tokens as u32,
                });

            return Ok(EmbeddingResult {
                embeddings: vec![embedding],
                usage,
                provider_metadata: None,
                response: Some(EmbeddingResponse {
                    headers: Some(response_headers),
                    body: Some(raw_value),
                }),
                warnings: Vec::new(),
            });
        }

        // Other models: use :predict endpoint (batch).
        let instances: Vec<Value> = options
            .values
            .iter()
            .map(|value| {
                let mut instance = Map::new();
                instance.insert("content".to_string(), json!(value));
                if let Some(task_type) = &vertex_options.task_type {
                    instance.insert("task_type".to_string(), json!(task_type));
                }
                if let Some(title) = &vertex_options.title {
                    instance.insert("title".to_string(), json!(title));
                }
                Value::Object(instance)
            })
            .collect();

        let mut parameters = Map::new();
        if let Some(dim) = vertex_options.output_dimensionality {
            parameters.insert("outputDimensionality".to_string(), json!(dim));
        }
        if let Some(auto_truncate) = vertex_options.auto_truncate {
            parameters.insert("autoTruncate".to_string(), json!(auto_truncate));
        }

        let mut body = Map::new();
        body.insert("instances".to_string(), Value::Array(instances));
        body.insert("parameters".to_string(), Value::Object(parameters));

        let url = exchange.url(&format!("/models/{}:predict", self.model_id));

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(url, options),
            exchange.transform_body(Value::Object(body)),
            aimux_provider_utils::create_json_response_handler(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_value: Value = resp.value;

        // Batch: response.predictions[].embeddings.values
        let (embeddings, total_tokens): (Vec<Vec<f32>>, u32) = raw_value
            .get("predictions")
            .and_then(|p| p.as_array())
            .map(|arr| {
                let embs: Vec<Vec<f32>> = arr
                    .iter()
                    .map(|pred| {
                        pred.get("embeddings")
                            .and_then(|e| e.get("values"))
                            .and_then(|v| v.as_array())
                            .map(|vals| {
                                vals.iter()
                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
                    .collect();
                let tokens: u32 = arr
                    .iter()
                    .filter_map(|pred| {
                        pred.get("embeddings")
                            .and_then(|e| e.get("statistics"))
                            .and_then(|s| s.get("token_count"))
                            .and_then(serde_json::Value::as_u64)
                    })
                    .map(|t| t as u32)
                    .sum();
                (embs, tokens)
            })
            .unwrap_or_default();

        Ok(EmbeddingResult {
            embeddings,
            usage: Some(EmbeddingUsage {
                tokens: total_tokens,
            }),
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

struct VertexEmbeddingProviderOptions {
    output_dimensionality: Option<u32>,
    task_type: Option<String>,
    title: Option<String>,
    auto_truncate: Option<bool>,
}

/// Parse Vertex embedding provider options.
///
/// Tries the `googleVertex` key first, then `google` (the shared Gemini
/// model's key).
fn parse_vertex_provider_options(
    options: Option<&SharedProviderOptions>,
) -> VertexEmbeddingProviderOptions {
    let provider_opts = Namespace::Vertex.read_in(options);

    VertexEmbeddingProviderOptions {
        output_dimensionality: provider_opts
            .and_then(|o| o.get("outputDimensionality"))
            .and_then(serde_json::Value::as_u64)
            .map(|d| d as u32),
        task_type: provider_opts
            .and_then(|o| o.get("taskType"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        title: provider_opts
            .and_then(|o| o.get("title"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        auto_truncate: provider_opts
            .and_then(|o| o.get("autoTruncate"))
            .and_then(serde_json::Value::as_bool),
    }
}
