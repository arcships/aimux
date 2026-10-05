//! The OpenAI-compatible embedding model (`{name}.embedding`).
//!
//! The Rust form of `OpenAICompatibleEmbeddingModel`: `POST {base_url}/embeddings`
//! with `encoding_format: "float"`, the `dimensions` and `user` options read
//! from the `openaiCompatible` namespace and then the provider's own.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;

use super::config::CompatModelConfig;
use super::convert::to_camel_case;

/// An OpenAI-compatible embedding model.
pub struct OpenAICompatibleEmbeddingModel {
    model_id: String,
    config: CompatModelConfig,
}

impl OpenAICompatibleEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: CompatModelConfig) -> Self {
        Self { model_id, config }
    }
}

/// `dimensions` and `user` from the generic namespace, then the provider's
/// own (later wins).
fn embedding_options(options: &EmbeddingCallOptions, name: &str) -> (Option<u64>, Option<String>) {
    let camel = to_camel_case(name);
    let mut dimensions = None;
    let mut user = None;
    for key in ["openaiCompatible", name, camel.as_str()] {
        let Some(object) = options
            .provider_options
            .as_ref()
            .and_then(|all| all.get(key))
        else {
            continue;
        };
        if let Some(value) = object.get("dimensions").and_then(Value::as_u64) {
            dimensions = Some(value);
        }
        if let Some(value) = object.get("user").and_then(Value::as_str) {
            user = Some(value.to_string());
        }
    }
    (dimensions, user)
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
        let (dimensions, user) = embedding_options(options, self.config.provider_options_name());

        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model_id));
        body.insert("input".to_string(), json!(options.values));
        body.insert("encoding_format".to_string(), json!("float"));
        if let Some(dimensions) = dimensions {
            body.insert("dimensions".to_string(), json!(dimensions));
        }
        if let Some(user) = user {
            body.insert("user".to_string(), json!(user));
        }
        let body = self.config.transform_body(Value::Object(body));

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
            provider_metadata: None,
            response: Some(EmbeddingResponse {
                headers: Some(response_headers),
                body: Some(raw_value),
            }),
            warnings: Vec::new(),
        })
    }
}
