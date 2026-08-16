//! Mistral embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `MistralEmbeddingModel`
//! (`reference/ai/packages/mistral/src/mistral-embedding-model.ts`).
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
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send};

use super::MistralConfig;

/// A Mistral embedding model (e.g. `"mistral-embed"`).
pub struct MistralEmbeddingModel {
    model_id: String,
    config: MistralConfig,
}

impl MistralEmbeddingModel {
    #[must_use]
    pub fn new(model_id: String, config: MistralConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
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
impl EmbeddingModel for MistralEmbeddingModel {
    fn provider(&self) -> &str {
        "mistral"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        Some(32)
    }

    fn supports_parallel_calls(&self) -> bool {
        false
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        let mistral_options = parse_mistral_provider_options(options.provider_options.as_ref());

        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model_id));
        body.insert("input".to_string(), json!(options.values));
        if let Some(metadata) = mistral_options.metadata {
            body.insert("metadata".to_string(), metadata);
        }
        if let Some(output_dimension) = mistral_options.output_dimension {
            body.insert("output_dimension".to_string(), json!(output_dimension));
        }
        if let Some(output_dtype) = mistral_options.output_dtype {
            body.insert("output_dtype".to_string(), json!(output_dtype));
        }
        body.insert("encoding_format".to_string(), json!("float"));

        let headers = self.build_headers(options.headers.as_ref());

        let mut header_list: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        header_list.push(("Content-Type".to_string(), "application/json".to_string()));

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: header_list,
                body: HttpBody::Json(Value::Object(body)),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let raw_value: Value = serde_json::from_slice(&resp.body)?;

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

struct MistralEmbeddingProviderOptions {
    metadata: Option<Value>,
    output_dimension: Option<u32>,
    output_dtype: Option<String>,
}

fn parse_mistral_provider_options(
    options: Option<&SharedProviderOptions>,
) -> MistralEmbeddingProviderOptions {
    let provider_opts = options.and_then(|opts| opts.get("mistral"));
    MistralEmbeddingProviderOptions {
        metadata: provider_opts.and_then(|o| o.get("metadata")).cloned(),
        output_dimension: provider_opts
            .and_then(|o| o.get("outputDimension"))
            .and_then(serde_json::Value::as_u64)
            .map(|d| d as u32),
        output_dtype: provider_opts
            .and_then(|o| o.get("outputDtype"))
            .and_then(|d| d.as_str())
            .map(std::string::ToString::to_string),
    }
}
