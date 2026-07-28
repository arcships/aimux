//! OpenAI embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `OpenAIEmbeddingModel`
//! (`reference/ai/packages/openai/src/embedding/openai-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/embeddings`

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use aimux_provider_utils::response::{DEFAULT_ERROR_STRUCTURE, parse_provider_error};

use super::OpenAIConfig;

/// An OpenAI-compatible embedding model.
///
/// Works with any OpenAI-compatible `/embeddings` endpoint.
pub struct OpenAIEmbeddingModel {
    model_id: String,
    config: OpenAIConfig,
    client: Client,
}

impl OpenAIEmbeddingModel {
    pub fn new(model_id: String, config: OpenAIConfig, client: Client) -> Self {
        Self {
            model_id,
            config,
            client,
        }
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

        let resp = self
            .client
            .post(self.endpoint())
            .header("Content-Type", "application/json")
            .headers(reqwest::header::HeaderMap::from_iter(
                headers.iter().filter_map(|(k, v)| {
                    reqwest::header::HeaderName::try_from(k)
                        .ok()
                        .zip(reqwest::header::HeaderValue::try_from(v).ok())
                }),
            ))
            .json(&Value::Object(body))
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &DEFAULT_ERROR_STRUCTURE,
            ));
        }

        let response_headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let raw_value: Value = resp
            .json()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        // Extract embeddings: response.data[].embedding
        let embeddings: Vec<Vec<f32>> = raw_value
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item.get("embedding")
                            .and_then(|e| e.as_array())
                            .map(|vals| {
                                vals.iter()
                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                    .collect()
                            })
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
