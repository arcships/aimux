//! Mistral embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `MistralEmbeddingModel`
//! (`reference/ai/packages/mistral/src/mistral-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/embeddings`

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use crate::shared::EndpointConfig;

/// A Mistral embedding model (e.g. `"mistral-embed"`).
pub struct MistralEmbeddingModel {
    model_id: String,
    config: EndpointConfig,
}

impl MistralEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl EmbeddingModel for MistralEmbeddingModel {
    fn provider(&self) -> &str {
        &self.config.provider
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
        if options.values.len() > 32 {
            return Err(AiMuxError::InvalidArgument(format!(
                "{} model {} supports at most 32 embeddings per call; received {}",
                self.provider(),
                self.model_id,
                options.values.len()
            )));
        }
        let mistral_options = parse_mistral_provider_options(options.provider_options.as_ref())?;

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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/embeddings"), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            super::mistral_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_value: Value = resp.value;

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
    output_dimension: Option<Value>,
    output_dtype: Option<String>,
}

fn parse_mistral_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<MistralEmbeddingProviderOptions, AiMuxError> {
    let provider_opts = super::options::mistral_options(options);
    let metadata = provider_opts.and_then(|o| o.get("metadata")).cloned();
    let output_dimension = provider_opts
        .and_then(|o| o.get("outputDimension"))
        .cloned();
    let output_dtype = provider_opts.and_then(|o| o.get("outputDtype"));
    if metadata.as_ref().is_some_and(|value| !value.is_object()) {
        return Err(AiMuxError::InvalidArgument(
            "mistral.metadata must be a record".into(),
        ));
    }
    if output_dimension.as_ref().is_some_and(|value| {
        !value.as_f64().is_some_and(|number| {
            number > 0.0 && number.fract() == 0.0 && number <= 9_007_199_254_740_991.0
        })
    }) {
        return Err(AiMuxError::InvalidArgument(
            "mistral.outputDimension must be a positive integer".into(),
        ));
    }
    if output_dtype.is_some_and(|value| {
        !matches!(
            value.as_str(),
            Some("float" | "int8" | "uint8" | "binary" | "ubinary")
        )
    }) {
        return Err(AiMuxError::InvalidArgument(
            "mistral.outputDtype must be float, int8, uint8, binary, or ubinary".into(),
        ));
    }
    Ok(MistralEmbeddingProviderOptions {
        metadata,
        output_dimension,
        output_dtype: output_dtype.and_then(Value::as_str).map(str::to_owned),
    })
}
