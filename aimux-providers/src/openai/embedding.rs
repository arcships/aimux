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
        if options.values.len() > 2048 {
            return Err(AiMuxError::InvalidArgument(format!(
                "too many embedding values: {} exceeds 2048",
                options.values.len()
            )));
        }
        let openai_options = parse_openai_provider_options(options.provider_options.as_ref())?;

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
        let body = Value::Object(body);

        let resp = aimux_provider_utils::post_json_to_api(
            self.config
                .http_request(self.config.url("/embeddings")?, headers, options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            super::openai_failed_response_handler(),
        )
        .await?;

        // `send` retries 408/409/429/5xx and returns an error for non-2xx, so an `Ok`
        // response here is guaranteed to be 2xx — no manual is_success() check.
        let response_headers = resp.response_headers;

        let raw_value: Value = resp.value;

        let parsed: OpenAIEmbeddingResponse = serde_json::from_value(raw_value.clone())
            .map_err(|error| AiMuxError::InvalidResponseData(error.to_string()))?;
        let embeddings = parsed.data.into_iter().map(|item| item.embedding).collect();
        let usage = parsed.usage.map(|usage| EmbeddingUsage {
            tokens: usage.prompt_tokens as u32,
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

#[derive(serde::Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
    usage: Option<OpenAIEmbeddingUsage>,
}

#[derive(serde::Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(serde::Deserialize)]
struct OpenAIEmbeddingUsage {
    prompt_tokens: f64,
}

#[derive(Default, serde::Deserialize)]
struct OpenAIEmbeddingProviderOptions {
    dimensions: Option<f64>,
    user: Option<String>,
}

fn parse_openai_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<OpenAIEmbeddingProviderOptions, AiMuxError> {
    match options.and_then(|options| options.get("openai")) {
        Some(value) => {
            if value
                .get("dimensions")
                .is_some_and(|value| !value.is_number())
                || value.get("user").is_some_and(|value| !value.is_string())
            {
                return Err(AiMuxError::InvalidArgument(
                    "invalid openai embedding options".into(),
                ));
            }
            serde_json::from_value(Value::Object(value.clone())).map_err(|error| {
                AiMuxError::InvalidArgument(format!("invalid openai embedding options: {error}"))
            })
        }
        None => Ok(OpenAIEmbeddingProviderOptions::default()),
    }
}
