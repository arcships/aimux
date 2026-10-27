//! The OpenAI-compatible embedding model (`{name}.embedding`,
//! `openai-compatible-embedding-model.ts`).
//!
//! The Rust form of `OpenAICompatibleEmbeddingModel`: `POST {base_url}/embeddings`
//! with `encoding_format: "float"`, the `dimensions` and `user` options read
//! from the compatible namespaces and then the provider's own.

use aimux_core::ErrorCause;
use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::openai_compatible::CompatModelConfig;

use super::embedding_model_options::embedding_options;

/// An OpenAI-compatible embedding model.
pub struct OpenAICompatibleEmbeddingModel {
    model_id: String,
    pub(in crate::openai_compatible) config: CompatModelConfig,
}

impl OpenAICompatibleEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: CompatModelConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl EmbeddingModel for OpenAICompatibleEmbeddingModel {
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
        let mut warnings = Vec::new();
        let compatible_options =
            embedding_options(options, self.config.provider_options_name(), &mut warnings)?;
        if options.values.len() > self.max_embeddings_per_call().unwrap_or(2048) as usize {
            return Err(AiMuxError::invalid_argument(format!(
                "Too many embedding values for {} / {}: maximum 2048, received {}",
                self.provider(),
                self.model_id(),
                options.values.len()
            )));
        }

        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model_id));
        body.insert("input".to_string(), json!(options.values));
        body.insert("encoding_format".to_string(), json!("float"));
        body.extend(compatible_options);
        let body = Value::Object(body);

        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let resp = aimux_provider_utils::post_json_to_api(
            self.config.http_request("/embeddings", headers, options)?,
            body,
            aimux_provider_utils::create_json_response_handler(),
            self.config.failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_value: Value = resp.value;

        let embeddings: Vec<Vec<f32>> = raw_value
            .get("data")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("embedding")?.as_array().map(|numbers| {
                            numbers
                                .iter()
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
            .and_then(Value::as_u64)
            .map(|tokens| EmbeddingUsage {
                tokens: tokens as u32,
            });

        Ok(EmbeddingResult {
            embeddings,
            usage,
            provider_metadata: raw_value
                .get("providerMetadata")
                .map(|metadata| serde_json::from_value(metadata.clone()))
                .transpose()
                .map_err(|error| AiMuxError::InvalidResponseData {
                    message: error.to_string(),
                    cause: Some(ErrorCause::source(&error)),
                })?,
            response: Some(EmbeddingResponse {
                headers: Some(response_headers),
                body: Some(raw_value),
            }),
            warnings,
        })
    }
}
