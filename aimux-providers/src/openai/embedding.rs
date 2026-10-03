//! OpenAI embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `OpenAIEmbeddingModel`
//! (`reference/ai/packages/openai/src/embedding/openai-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/embeddings`

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use super::OpenAIConfig;
use super::config::OpenAIModelConfig;

/// An OpenAI-compatible embedding model.
///
/// Works with any OpenAI-compatible `/embeddings` endpoint. Does **not** hold an
/// HTTP client — the `aimux-provider-utils` API helpers use the shared `Client` internally (RFC-0009 §4.1).
pub struct OpenAIEmbeddingModel {
    model_id: String,
    config: OpenAIModelConfig,
}

impl OpenAIEmbeddingModel {
    /// An embedding model configured through the transitional [`OpenAIConfig`]
    /// builder. The native package builds models through
    /// [`OpenAIProvider::embedding`](super::OpenAIProvider::embedding).
    #[must_use]
    pub fn new(model_id: String, config: OpenAIConfig) -> Self {
        Self::from_config(model_id, config.into_model_config("embedding"))
    }

    pub(crate) fn from_config(model_id: String, config: OpenAIModelConfig) -> Self {
        Self { model_id, config }
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

        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let body = self.config.transform_body(Value::Object(body));

        let resp = aimux_provider_utils::post_json_to_api(
            self.config
                .http_request(self.config.url("/embeddings"), headers, options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            super::openai_failed_response_handler(),
        )
        .await?;

        // `send` retries 408/409/429/5xx and returns an error for non-2xx, so an `Ok`
        // response here is guaranteed to be 2xx — no manual is_success() check.
        let response_headers = resp.response_headers;

        let raw_value: Value = resp.value;

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
            .and_then(serde_json::Value::as_u64)
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
            .and_then(serde_json::Value::as_u64)
            .map(|d| d as u32),
        user: provider_opts
            .and_then(|o| o.get("user"))
            .and_then(|u| u.as_str())
            .map(std::string::ToString::to_string),
    }
}

/// Decode a base64-encoded embedding string into a `Vec<f32>`.
///
/// OpenAI's API returns embeddings as base64-encoded little-endian f32
/// arrays when `encoding_format: "base64"` is requested. The raw bytes
/// are decoded from base64, then reinterpreted as little-endian f32.
// Rust 1.98 clippy suggests `as_chunks::<4>()`, stabilized in 1.88; the
// workspace MSRV is 1.85. Drop when the MSRV moves past 1.88.
#[allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]
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
