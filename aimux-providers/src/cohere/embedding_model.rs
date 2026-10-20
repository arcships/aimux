//! Cohere embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `CohereEmbeddingModel`
//! (`reference/ai/packages/cohere/src/cohere-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/embed`

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;

use crate::shared::EndpointConfig;

use super::embedding_model_options::parse_cohere_provider_options;

/// A Cohere embedding model (e.g. `"embed-english-v3.0"`).
pub struct CohereEmbeddingModel {
    model_id: String,
    config: EndpointConfig,
}

impl CohereEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl EmbeddingModel for CohereEmbeddingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        Some(96)
    }

    fn supports_parallel_calls(&self) -> bool {
        true
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        let cohere_options = parse_cohere_provider_options(options.provider_options.as_ref())?;
        if options.values.len() > 96 {
            return Err(AiMuxError::InvalidArgument(format!(
                "Too many values for a single embedding call: {} supports at most 96 values, received {}",
                self.provider(),
                options.values.len()
            )));
        }

        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model_id));
        body.insert("embedding_types".to_string(), json!(["float"]));
        body.insert("texts".to_string(), json!(options.values));
        // Default input_type is "search_query" when not provided.
        body.insert(
            "input_type".to_string(),
            json!(
                cohere_options
                    .input_type
                    .unwrap_or_else(|| "search_query".to_string())
            ),
        );
        if let Some(truncate) = cohere_options.truncate {
            body.insert("truncate".to_string(), json!(truncate));
        }
        if let Some(output_dimension) = cohere_options.output_dimension {
            body.insert("output_dimension".to_string(), json!(output_dimension));
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/embed"), options),
            exchange.transform_body(Value::Object(body)),
            aimux_provider_utils::create_json_response_handler::<CohereEmbeddingResponse>(),
            super::error::cohere_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_value = resp.raw_value;
        let data = resp.value;
        let embeddings = data.embeddings.float;
        let usage = EmbeddingUsage {
            tokens: data.meta.billed_units.input_tokens as u32,
        };

        Ok(EmbeddingResult {
            embeddings,
            usage: Some(usage),
            provider_metadata: None,
            response: Some(EmbeddingResponse {
                headers: Some(response_headers),
                body: raw_value,
            }),
            warnings: Vec::new(),
        })
    }
}

#[derive(Deserialize)]
struct CohereEmbeddingResponse {
    embeddings: CohereFloatEmbeddings,
    meta: CohereEmbeddingMeta,
}

#[derive(Deserialize)]
struct CohereFloatEmbeddings {
    float: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct CohereEmbeddingMeta {
    billed_units: CohereEmbeddingBilledUnits,
}

#[derive(Deserialize)]
struct CohereEmbeddingBilledUnits {
    input_tokens: f64,
}
