//! OpenAI embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `OpenAIEmbeddingModel`
//! (`reference/ai/packages/openai/src/embedding/openai-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/embeddings`

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send};

use super::OpenAIConfig;

/// An OpenAI-compatible embedding model.
///
/// Works with any OpenAI-compatible `/embeddings` endpoint. Does **not** hold an
/// HTTP client — `http::send` uses the shared `Client` internally (RFC-0009 §4.1).
pub struct OpenAIEmbeddingModel {
    model_id: String,
    config: OpenAIConfig,
}

impl OpenAIEmbeddingModel {
    pub fn new(model_id: String, config: OpenAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(ref org) = self.config.org_id {
            headers.insert("OpenAI-Organization".to_string(), org.clone());
        }
        if let Some(ref project) = self.config.project {
            headers.insert("OpenAI-Project".to_string(), project.clone());
        }
        // Config-level extra headers (lowest priority after auth/org/project).
        if let Some(ref config_headers) = self.config.headers {
            for (k, v) in config_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/embeddings", self.config.base_url)
    }
}

#[async_trait]
impl EmbeddingModel for OpenAIEmbeddingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        Some(2048)
    }

    fn supports_parallel_calls(&self) -> bool {
        true
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        let openai_options = parse_openai_provider_options(options.provider_options.as_ref());

        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model_id));
        body.insert("input".to_string(), json!(options.values));
        body.insert("encoding_format".to_string(), json!("float"));
        if let Some(dimensions) = openai_options.dimensions {
            body.insert("dimensions".to_string(), json!(dimensions));
        }
        if let Some(user) = openai_options.user {
            body.insert("user".to_string(), json!(user));
        }

        let headers = self.build_headers(options.headers.as_ref());

        let mut header_list: Vec<(String, String)> =
            headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        header_list.push(("Content-Type".to_string(), "application/json".to_string()));

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: header_list,
                body: HttpBody::Json(Value::Object(body)),
            },
            self.config.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        // `send` retries 429/5xx and returns an error for non-2xx, so an `Ok`
        // response here is guaranteed to be 2xx — no manual is_success() check.
        let response_headers = resp.headers;

        let raw_value: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        // Extract embeddings: response.data[].embedding
        // The embedding field can be a JSON array of floats (default) or a
        // base64-encoded string (when encoding_format="base64").
        let embeddings: Vec<Vec<f32>> = raw_value
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let emb = item.get("embedding")?;
                        if let Some(arr) = emb.as_array() {
                            // Standard format: array of numbers
                            Some(
                                arr.iter()
                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                    .collect(),
                            )
                        } else if let Some(s) = emb.as_str() {
                            // Base64 format: decode to little-endian f32 array
                            decode_base64_embedding(s)
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Extract usage: response.usage.prompt_tokens
        let usage = raw_value
            .get("usage")
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .map(|tokens| EmbeddingUsage {
                tokens: tokens as u32,
            });

        Ok(EmbeddingResult {
            embeddings,
            usage,
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

/// Parsed `openai` embedding provider options.
struct OpenAIEmbeddingProviderOptions {
    dimensions: Option<u32>,
    user: Option<String>,
}

/// Extract OpenAI-specific embedding options from the shared provider options.
///
/// Mirrors the TS `parseProviderOptions({ provider: 'openai', ... })`.
fn parse_openai_provider_options(
    options: Option<&SharedProviderOptions>,
) -> OpenAIEmbeddingProviderOptions {
    let provider_opts = options.and_then(|opts| opts.get("openai"));
    OpenAIEmbeddingProviderOptions {
        dimensions: provider_opts
            .and_then(|o| o.get("dimensions"))
            .and_then(|d| d.as_u64())
            .map(|d| d as u32),
        user: provider_opts
            .and_then(|o| o.get("user"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string()),
    }
}

/// Decode a base64-encoded embedding string into a `Vec<f32>`.
///
/// OpenAI's API returns embeddings as base64-encoded little-endian f32
/// arrays when `encoding_format: "base64"` is requested. The raw bytes
/// are decoded from base64, then reinterpreted as little-endian f32.
fn decode_base64_embedding(s: &str) -> Option<Vec<f32>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}
