//! Google Gemini embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `GoogleEmbeddingModel`
//! (`reference/ai/packages/google/src/google-embedding-model.ts`).
//!
//! Uses two endpoints depending on the number of values:
//! - Single value: `POST {base_url}/models/{model}:embedContent`
//! - Multiple values: `POST {base_url}/models/{model}:batchEmbedContents`

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult,
};
use aimux_core::error::AiMuxError;

use super::options::google_options;
use crate::shared::EndpointConfig;

/// A Google Gemini embedding model (e.g. `"gemini-embedding-001"`).
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct GoogleEmbeddingModel {
    model_id: String,
    config: EndpointConfig,
}

impl GoogleEmbeddingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl EmbeddingModel for GoogleEmbeddingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        Some(100)
    }

    fn supports_parallel_calls(&self) -> bool {
        true
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        let provider_options = google_options(options.provider_options.as_ref());
        let mut settings = Map::new();
        if let Some(opts) = provider_options {
            if let Some(dimension) = opts.get("outputDimensionality") {
                if !dimension.is_number() {
                    return Err(AiMuxError::InvalidArgument(
                        "outputDimensionality must be a number".into(),
                    ));
                }
                settings.insert("outputDimensionality".into(), dimension.clone());
            }
            if let Some(task) = opts.get("taskType") {
                if !matches!(
                    task.as_str(),
                    Some(
                        "SEMANTIC_SIMILARITY"
                            | "CLASSIFICATION"
                            | "CLUSTERING"
                            | "RETRIEVAL_DOCUMENT"
                            | "RETRIEVAL_QUERY"
                            | "QUESTION_ANSWERING"
                            | "FACT_VERIFICATION"
                            | "CODE_RETRIEVAL_QUERY"
                    )
                ) {
                    return Err(AiMuxError::InvalidArgument(
                        "Invalid Google embedding taskType".into(),
                    ));
                }
                settings.insert("taskType".into(), task.clone());
            }
        }
        if options.values.len() > 100 {
            return Err(AiMuxError::InvalidArgument(format!(
                "Too many values for a single embedding call. The {} model \"{}\" can only embed up to 100 values per call, but {} values were provided.",
                self.provider(),
                self.model_id,
                options.values.len()
            )));
        }
        let multimodal = provider_options.and_then(|o| o.get("content"));
        let multimodal = multimodal
            .map(|v| {
                v.as_array().ok_or_else(|| {
                    AiMuxError::InvalidArgument("Google embedding content must be an array".into())
                })
            })
            .transpose()?;
        if let Some(content) = multimodal {
            if content.len() != options.values.len() {
                return Err(AiMuxError::InvalidArgument(format!(
                    "The number of multimodal content entries ({}) must match the number of values ({}).",
                    content.len(),
                    options.values.len()
                )));
            }
            for entry in content.iter().filter(|v| !v.is_null()) {
                let parts = entry.as_array().filter(|p| !p.is_empty()).ok_or_else(|| {
                    AiMuxError::InvalidArgument(
                        "Google embedding content entries must be nonempty arrays or null".into(),
                    )
                })?;
                for part in parts {
                    let valid = part.get("text").is_some_and(Value::is_string)
                        || part.get("inlineData").is_some_and(|p| {
                            p.get("mimeType").is_some_and(Value::is_string)
                                && p.get("data").is_some_and(Value::is_string)
                        })
                        || part.get("fileData").is_some_and(|p| {
                            p.get("mimeType").is_some_and(Value::is_string)
                                && p.get("fileUri").is_some_and(Value::is_string)
                        });
                    if !valid {
                        return Err(AiMuxError::InvalidArgument(
                            "Invalid Google embedding content part".into(),
                        ));
                    }
                }
            }
        }
        let single = options.values.len() == 1;
        let requests: Vec<Value> = options
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let extras = multimodal.and_then(|entries| entries[index].as_array());
                let mut parts = Vec::new();
                if extras.is_none() || !value.is_empty() {
                    parts.push(json!({"text": value}));
                }
                if let Some(extras) = extras {
                    parts.extend(extras.iter().map(|part| {
                        if let Some(text) = part.get("text").and_then(Value::as_str) { json!({"text": text}) }
                        else if let Some(inline) = part.get("inlineData").filter(|v| v.get("mimeType").is_some_and(Value::is_string) && v.get("data").is_some_and(Value::is_string)) {
                            json!({"inlineData": {"mimeType": inline["mimeType"], "data": inline["data"]}})
                        } else { json!({"fileData": {"mimeType": part["fileData"]["mimeType"], "fileUri": part["fileData"]["fileUri"]}}) }
                    }));
                }
                let mut content = json!({"parts": parts});
                if !single {
                    content["role"] = json!("user");
                }
                let mut request = settings.clone();
                request.insert("model".into(), json!(format!("models/{}", self.model_id)));
                request.insert("content".into(), content);
                Value::Object(request)
            })
            .collect();
        let body = if single {
            requests[0].clone()
        } else {
            json!({"requests": requests})
        };
        let endpoint = if single {
            "embedContent"
        } else {
            "batchEmbedContents"
        };
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(
                exchange.url(&format!("/models/{}:{endpoint}", self.model_id)),
                options,
            ),
            exchange.transform_body(body),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            super::google_failed_response_handler(),
        )
        .await?;
        let raw_value = resp.value;
        let items = if single {
            vec![
                raw_value
                    .get("embedding")
                    .ok_or_else(|| AiMuxError::InvalidResponseData("Missing embedding".into()))?,
            ]
        } else {
            raw_value
                .get("embeddings")
                .and_then(Value::as_array)
                .ok_or_else(|| AiMuxError::InvalidResponseData("Missing embeddings".into()))?
                .iter()
                .collect()
        };
        let embeddings = items
            .into_iter()
            .map(|item| {
                let values = item
                    .get("values")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        AiMuxError::InvalidResponseData("Missing embedding values".into())
                    })?;
                values
                    .iter()
                    .map(|v| {
                        v.as_f64().map(|v| v as f32).ok_or_else(|| {
                            AiMuxError::InvalidResponseData(
                                "Embedding values must be numbers".into(),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(EmbeddingResult {
            embeddings,
            usage: None,
            provider_metadata: None,
            response: Some(EmbeddingResponse {
                headers: Some(resp.response_headers),
                body: Some(raw_value),
            }),
            warnings: Vec::new(),
        })
    }
}
