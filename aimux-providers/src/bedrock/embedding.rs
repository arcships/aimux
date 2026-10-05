//! Amazon Bedrock embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `AmazonBedrockEmbeddingModel`
//! (`reference/ai/packages/amazon-bedrock/src/amazon-bedrock-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/model/{model_id}/invoke`
//!
//! Different embedding model families (Titan, Cohere, Nova) expect different
//! request/response payloads. This implementation adapts based on the model ID,
//! matching the TS reference.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use aimux_provider_utils::HttpBody;

use super::options;
use crate::shared::EndpointConfig;

/// An Amazon Bedrock embedding model (e.g. `"amazon.titan-embed-text-v2:0"`).
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct BedrockEmbeddingModel {
    model_id: String,
    config: EndpointConfig,
    model_family: Option<String>,
}

impl BedrockEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self {
            model_id,
            config,
            model_family: None,
        }
    }

    pub(crate) fn with_model_family(mut self, model_family: Option<String>) -> Self {
        self.model_family = model_family;
        self
    }

    fn model_family(&self) -> &str {
        self.model_family.as_deref().unwrap_or_else(|| {
            if is_nova_embedding_model(&self.model_id) {
                "nova"
            } else if is_cohere_embedding_model(&self.model_id) {
                options::COHERE_MODEL_FAMILY
            } else {
                "titan"
            }
        })
    }
}

/// Returns `true` if the model ID is a Cohere embedding model.
/// Uses `contains` so cross-region inference profile ids (e.g.
/// `us.cohere.embed-v4:0`) are detected too.
fn is_cohere_embedding_model(model_id: &str) -> bool {
    model_id.contains("cohere.embed-")
}

/// Returns `true` if the model ID is an Amazon Nova embedding model.
fn is_nova_embedding_model(model_id: &str) -> bool {
    model_id.contains("amazon.nova-") && model_id.contains("embed")
}

#[async_trait]
impl EmbeddingModel for BedrockEmbeddingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        if self.model_family() == options::COHERE_MODEL_FAMILY {
            Some(96)
        } else {
            Some(1)
        }
    }

    fn supports_parallel_calls(&self) -> bool {
        true
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        if options.values.len() > self.max_embeddings_per_call().unwrap_or(1) as usize {
            return Err(AiMuxError::InvalidArgument(format!(
                "Too many embedding values for amazon-bedrock model {}: maximum {} per call",
                self.model_id,
                self.max_embeddings_per_call().unwrap_or(1)
            )));
        }
        let bedrock_options = parse_bedrock_provider_options(options.provider_options.as_ref())?;

        let is_nova = self.model_family() == "nova";
        let is_cohere = self.model_family() == options::COHERE_MODEL_FAMILY;

        // Build request body based on model family.
        let body = if is_nova {
            // Nova embedding models use a SINGLE_EMBEDDING payload.
            let mut text_obj = Map::new();
            text_obj.insert(
                "truncationMode".to_string(),
                json!(
                    bedrock_options
                        .truncate
                        .unwrap_or_else(|| "END".to_string())
                ),
            );
            text_obj.insert("value".to_string(), json!(options.values[0]));

            let mut single_params = Map::new();
            single_params.insert(
                "embeddingPurpose".to_string(),
                json!(
                    bedrock_options
                        .embedding_purpose
                        .unwrap_or_else(|| "GENERIC_INDEX".to_string())
                ),
            );
            single_params.insert(
                "embeddingDimension".to_string(),
                json!(bedrock_options.embedding_dimension.unwrap_or(1024)),
            );
            single_params.insert("text".to_string(), Value::Object(text_obj));

            let mut body = Map::new();
            body.insert("taskType".to_string(), json!("SINGLE_EMBEDDING"));
            body.insert(
                "singleEmbeddingParams".to_string(),
                Value::Object(single_params),
            );
            Value::Object(body)
        } else if is_cohere {
            // Cohere embedding models on Bedrock.
            let mut body = Map::new();
            body.insert(
                "input_type".to_string(),
                json!(
                    bedrock_options
                        .input_type
                        .unwrap_or_else(|| "search_query".to_string())
                ),
            );
            body.insert("texts".to_string(), json!(options.values));
            if let Some(truncate) = bedrock_options.truncate {
                body.insert("truncate".to_string(), json!(truncate));
            }
            if let Some(output_dimension) = bedrock_options.output_dimension {
                body.insert("output_dimension".to_string(), json!(output_dimension));
            }
            Value::Object(body)
        } else {
            // Titan embedding models (default).
            let mut body = Map::new();
            body.insert("inputText".to_string(), json!(options.values[0]));
            if let Some(dimensions) = bedrock_options.dimensions {
                body.insert("dimensions".to_string(), json!(dimensions));
            }
            if let Some(normalize) = bedrock_options.normalize {
                body.insert("normalize".to_string(), json!(normalize));
            }
            Value::Object(body)
        };

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body);
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = exchange.url(&format!(
            "/model/{}/invoke",
            super::encode_model_id(&self.model_id)
        ));

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(url, options),
            HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            aimux_provider_utils::create_json_response_handler(),
            super::bedrock_failed_response_handler(),
        )
        .await?;

        // Capture response headers (needed for token count extraction).
        let response_headers = resp.response_headers;

        let raw_value: Value = resp.value;

        // Extract embeddings based on response format.
        let embeddings: Vec<Vec<f32>> = if raw_value.get("embedding").is_some() {
            // Titan response: { embedding: [...] }
            vec![
                raw_value
                    .get("embedding")
                    .and_then(|e| e.as_array())
                    .map(|vals| {
                        vals.iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect()
                    })
                    .unwrap_or_default(),
            ]
        } else if let Some(embeddings_arr) = raw_value.get("embeddings").and_then(|e| e.as_array())
        {
            if embeddings_arr
                .first()
                .is_some_and(|first| first.get("embeddingType").is_some())
            {
                let first = &embeddings_arr[0];
                // Nova response: { embeddings: [{ embeddingType, embedding }] }
                vec![
                    first
                        .get("embedding")
                        .and_then(|e| e.as_array())
                        .map(|vals| {
                            vals.iter()
                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                .collect()
                        })
                        .unwrap_or_default(),
                ]
            } else {
                // Cohere v3 response: { embeddings: [[...], ...] }
                embeddings_arr
                    .iter()
                    .map(|row| {
                        row.as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect()
                    })
                    .collect()
            }
        } else {
            // Cohere v4 response: { embeddings: { float: [[...], ...] } }
            raw_value
                .get("embeddings")
                .and_then(|e| e.get("float"))
                .and_then(|f| f.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|row| {
                            row.as_array()
                                .unwrap_or(&vec![])
                                .iter()
                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

        // Extract token count based on response format.
        let header_token_count = response_headers
            .get("x-amzn-bedrock-input-token-count")
            .and_then(|v| v.parse::<f64>().ok());

        let tokens = if let Some(count) = raw_value
            .get("inputTextTokenCount")
            .and_then(serde_json::Value::as_u64)
        {
            // Titan response
            count as u32
        } else if let Some(count) = raw_value
            .get("inputTokenCount")
            .and_then(serde_json::Value::as_u64)
        {
            // Nova response
            count as u32
        } else if let Some(header_count) = header_token_count {
            // Fall back to header token count (Cohere models).
            // Matches the TS `Number(...)` behaviour, which produces NaN when
            // the header is absent.
            header_count as u32
        } else {
            // No token count available — matches TS `Number(undefined)` = NaN.
            // We use 0 as a sane default since Rust's u32 can't represent NaN.
            0
        };

        Ok(EmbeddingResult {
            embeddings,
            usage: Some(EmbeddingUsage { tokens }),
            provider_metadata: None,
            response: None,
            warnings: Vec::new(),
        })
    }
}

// ── Provider options parsing ─────────────────────────────────────────────────

struct BedrockEmbeddingProviderOptions {
    dimensions: Option<u32>,
    normalize: Option<bool>,
    embedding_dimension: Option<u32>,
    embedding_purpose: Option<String>,
    input_type: Option<String>,
    truncate: Option<String>,
    output_dimension: Option<u32>,
}

/// Parse Bedrock embedding provider options.
///
/// Reads the `amazonBedrock` key only.
fn parse_bedrock_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<BedrockEmbeddingProviderOptions, AiMuxError> {
    let provider_opts = options::read(options);
    if let Some(opts) = provider_opts {
        let valid_dimensions = [
            ("dimensions", &[1024, 512, 256][..]),
            ("embeddingDimension", &[256, 384, 1024, 3072][..]),
            ("outputDimension", &[256, 512, 1024, 1536][..]),
        ]
        .iter()
        .all(|(key, allowed)| {
            opts.get(*key).is_none_or(|v| {
                v.as_f64()
                    .is_some_and(|n| allowed.iter().any(|allowed| n == f64::from(*allowed)))
            })
        });
        let valid_enums = [
            (
                "embeddingPurpose",
                &[
                    "GENERIC_INDEX",
                    "TEXT_RETRIEVAL",
                    "IMAGE_RETRIEVAL",
                    "VIDEO_RETRIEVAL",
                    "DOCUMENT_RETRIEVAL",
                    "AUDIO_RETRIEVAL",
                    "GENERIC_RETRIEVAL",
                    "CLASSIFICATION",
                    "CLUSTERING",
                ][..],
            ),
            (
                "inputType",
                &[
                    "search_document",
                    "search_query",
                    "classification",
                    "clustering",
                ][..],
            ),
            ("truncate", &["NONE", "START", "END"][..]),
        ]
        .iter()
        .all(|(key, allowed)| {
            opts.get(*key)
                .is_none_or(|v| v.as_str().is_some_and(|s| allowed.contains(&s)))
        });
        if !valid_dimensions
            || !valid_enums
            || opts.get("normalize").is_some_and(|v| !v.is_boolean())
        {
            return Err(AiMuxError::InvalidArgument(
                "invalid amazonBedrock provider options".into(),
            ));
        }
    }

    Ok(BedrockEmbeddingProviderOptions {
        dimensions: provider_opts
            .and_then(|o| o.get("dimensions"))
            .and_then(serde_json::Value::as_f64)
            .map(|d| d as u32),
        normalize: provider_opts
            .and_then(|o| o.get("normalize"))
            .and_then(serde_json::Value::as_bool),
        embedding_dimension: provider_opts
            .and_then(|o| o.get("embeddingDimension"))
            .and_then(serde_json::Value::as_f64)
            .map(|d| d as u32),
        embedding_purpose: provider_opts
            .and_then(|o| o.get("embeddingPurpose"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        input_type: provider_opts
            .and_then(|o| o.get("inputType"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        truncate: provider_opts
            .and_then(|o| o.get("truncate"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        output_dimension: provider_opts
            .and_then(|o| o.get("outputDimension"))
            .and_then(serde_json::Value::as_f64)
            .map(|d| d as u32),
    })
}
