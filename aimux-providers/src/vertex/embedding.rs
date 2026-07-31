//! Google Vertex AI embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `GoogleVertexEmbeddingModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-embedding-model.ts`).
//!
//! Uses two endpoints depending on the model:
//! - `gemini-embedding-2` / `gemini-embedding-2-preview`:
//!   `POST {base_url}/models/{model}:embedContent` (single value only)
//! - Others: `POST {base_url}/models/{model}:predict` (batch)

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send};

use super::VertexAuth;
use super::model::VertexConfig;

/// Google-specific error structure: `{ "error": { "message": "..." } }`.
const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

/// A Google Vertex AI embedding model (e.g. `"textembedding-gecko@001"`).
///
/// Does **not** hold an HTTP client — `http::send` uses the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct VertexEmbeddingModel {
    model_id: String,
    config: VertexConfig,
}

impl VertexEmbeddingModel {
    pub fn new(model_id: String, config: VertexConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> Vec<(String, String)> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        match &self.config.auth {
            VertexAuth::BearerToken(token) => {
                headers.push(("Authorization".to_string(), format!("Bearer {}", token)));
            }
            VertexAuth::ApiKey(key) => {
                headers.push(("x-goog-api-key".to_string(), key.clone()));
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.push((k.clone(), v.clone()));
            }
        }
        headers
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
        "google.vertex"
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
        // Parse provider options: try "googleVertex", then "vertex", then "google".
        let vertex_options = parse_vertex_provider_options(options.provider_options.as_ref());

        let headers = self.build_headers(options.headers.as_ref());

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

            let url = format!(
                "{}/models/{}:embedContent",
                self.config.base_url, self.model_id
            );

            let resp = send(
                HttpRequest {
                    method: HttpMethod::Post,
                    url,
                    headers,
                    body: HttpBody::Json(Value::Object(body)),

                    abort_signal: options.abort_signal.clone(),
                },
                RetryConfig::default(),
                &GOOGLE_ERROR_STRUCTURE,
            )
            .await?;

            let response_headers = resp.headers;

            let raw_value: Value =
                serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Http(e.to_string()))?;

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
                .and_then(|t| t.as_u64())
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

        let url = format!("{}/models/{}:predict", self.config.base_url, self.model_id);

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers,
                body: HttpBody::Json(Value::Object(body)),

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let raw_value: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Http(e.to_string()))?;

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
                            .and_then(|t| t.as_u64())
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
/// Tries the `"googleVertex"` key first, then `"vertex"`, then `"google"`
/// (matching the TS fallback chain).
fn parse_vertex_provider_options(
    options: Option<&SharedProviderOptions>,
) -> VertexEmbeddingProviderOptions {
    let opts = options;
    let provider_opts = opts
        .and_then(|o| o.get("googleVertex"))
        .or_else(|| opts.and_then(|o| o.get("vertex")))
        .or_else(|| opts.and_then(|o| o.get("google")));

    VertexEmbeddingProviderOptions {
        output_dimensionality: provider_opts
            .and_then(|o| o.get("outputDimensionality"))
            .and_then(|d| d.as_u64())
            .map(|d| d as u32),
        task_type: provider_opts
            .and_then(|o| o.get("taskType"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        title: provider_opts
            .and_then(|o| o.get("title"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        auto_truncate: provider_opts
            .and_then(|o| o.get("autoTruncate"))
            .and_then(|v| v.as_bool()),
    }
}
