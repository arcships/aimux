//! Google Gemini embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `GoogleEmbeddingModel`
//! (`reference/ai/packages/google/src/google-embedding-model.ts`).
//!
//! Uses two endpoints depending on the number of values:
//! - Single value: `POST {base_url}/models/{model}:embedContent`
//! - Multiple values: `POST {base_url}/models/{model}:batchEmbedContents`

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult,
};
use aimux_core::error::AiMuxError;

use super::options::GOOGLE;
use crate::shared::EndpointConfig;

/// `googleEmbeddingModelOptions` (google-embedding-model-options.ts). Every
/// key is `.optional()`: absent is `None`, `null` fails.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleEmbeddingOptions {
    /// `z.number()`, sent as given.
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    output_dimensionality: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    task_type: Option<TaskType>,
    #[serde(default, deserialize_with = "multimodal_content")]
    content: Option<MultimodalContent>,
}

/// `z.array(z.array(googleEmbeddingContentPartSchema).min(1).nullable())`:
/// per value, its extra parts or `null`.
type MultimodalContent = Vec<Option<Vec<ContentPart>>>;

/// The `taskType` enum, sent under the same name.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum TaskType {
    SemanticSimilarity,
    Classification,
    Clustering,
    RetrievalDocument,
    RetrievalQuery,
    QuestionAnswering,
    FactVerification,
    CodeRetrievalQuery,
}

/// `googleEmbeddingContentPartSchema`, a `z.union` of `z.object`s: the first
/// variant that matches, with only the keys it declares.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum ContentPart {
    Text {
        text: String,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
    FileData {
        #[serde(rename = "fileData")]
        file_data: FileData,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileData {
    mime_type: String,
    file_uri: String,
}

/// `content`: `.optional()` (`null` fails), and each non-null entry has at
/// least one part (`.min(1)`).
fn multimodal_content<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<MultimodalContent>, D::Error> {
    let content = MultimodalContent::deserialize(deserializer)?;
    if content.iter().flatten().any(Vec::is_empty) {
        return Err(serde::de::Error::custom(
            "each content entry has at least one part or is null",
        ));
    }
    Ok(Some(content))
}

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
        // google-embedding-model.ts: `parseProviderOptions` under `google`.
        let google_options: GoogleEmbeddingOptions = aimux_provider_utils::parse_provider_options(
            GOOGLE,
            options.provider_options.as_ref(),
        )?
        .unwrap_or_default();
        let mut settings = Map::new();
        if let Some(dimension) = &google_options.output_dimensionality {
            settings.insert("outputDimensionality".into(), json!(dimension));
        }
        if let Some(task) = google_options.task_type {
            settings.insert("taskType".into(), json!(task));
        }
        if options.values.len() > 100 {
            return Err(AiMuxError::invalid_argument(format!(
                "Too many values for a single embedding call. The {} model \"{}\" can only embed up to 100 values per call, but {} values were provided.",
                self.provider(),
                self.model_id,
                options.values.len()
            )));
        }
        let multimodal = google_options.content.as_deref();
        if let Some(content) = multimodal
            && content.len() != options.values.len()
        {
            return Err(AiMuxError::invalid_argument(format!(
                "The number of multimodal content entries ({}) must match the number of values ({}).",
                content.len(),
                options.values.len()
            )));
        }
        let single = options.values.len() == 1;
        let requests: Vec<Value> = options
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let extras = multimodal.and_then(|entries| entries[index].as_deref());
                let mut parts = Vec::new();
                if extras.is_none() || !value.is_empty() {
                    parts.push(json!({"text": value}));
                }
                if let Some(extras) = extras {
                    parts.extend(extras.iter().map(|part| json!(part)));
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
                    .ok_or_else(|| AiMuxError::invalid_response_data("Missing embedding"))?,
            ]
        } else {
            raw_value
                .get("embeddings")
                .and_then(Value::as_array)
                .ok_or_else(|| AiMuxError::invalid_response_data("Missing embeddings"))?
                .iter()
                .collect()
        };
        let embeddings = items
            .into_iter()
            .map(|item| {
                let values = item
                    .get("values")
                    .and_then(Value::as_array)
                    .ok_or_else(|| AiMuxError::invalid_response_data("Missing embedding values"))?;
                values
                    .iter()
                    .map(|v| {
                        v.as_f64().map(|v| v as f32).ok_or_else(|| {
                            AiMuxError::invalid_response_data("Embedding values must be numbers")
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
