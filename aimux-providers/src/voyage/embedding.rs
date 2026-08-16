//! Voyage embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `VoyageEmbeddingModel`
//! (`reference/ai/packages/voyage/src/voyage-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/embeddings`
//!
//! Unlike OpenAI, Voyage returns `data` items with an `index` field that may
//! be out of order; the model sorts by `index` before extracting embeddings.

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

use super::VoyageConfig;

/// A Voyage embedding model (e.g. `"voyage-3.5"`).
pub struct VoyageEmbeddingModel {
    model_id: String,
    config: VoyageConfig,
}

impl VoyageEmbeddingModel {
    #[must_use]
    pub fn new(model_id: String, config: VoyageConfig) -> Self {
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
impl EmbeddingModel for VoyageEmbeddingModel {
    fn provider(&self) -> &str {
        "voyage"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_embeddings_per_call(&self) -> Option<u32> {
        Some(128)
    }

    fn supports_parallel_calls(&self) -> bool {
        true
    }

    async fn do_embed(
        &self,
        options: &EmbeddingCallOptions,
    ) -> Result<EmbeddingResult, AiMuxError> {
        let voyage_options = parse_voyage_provider_options(options.provider_options.as_ref());

        let mut body = Map::new();
        body.insert("input".to_string(), json!(options.values));
        body.insert("model".to_string(), json!(self.model_id));
        if let Some(input_type) = voyage_options.input_type {
            body.insert("input_type".to_string(), json!(input_type));
        }
        if let Some(truncation) = voyage_options.truncation {
            body.insert("truncation".to_string(), json!(truncation));
        }
        if let Some(output_dimension) = voyage_options.output_dimension {
            body.insert("output_dimension".to_string(), json!(output_dimension));
        }
        if let Some(output_dtype) = voyage_options.output_dtype {
            body.insert("output_dtype".to_string(), json!(output_dtype));
        }

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // `send()` returns Ok only for 2xx; non-2xx responses are mapped to an
        // error internally using the shared error structure. `HttpBody::Json`
        // sets `Content-Type: application/json`, so it is intentionally not
        // added to the header list above.
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

        let response_headers: HashMap<String, String> = resp.headers.clone();

        let raw_value: Value = serde_json::from_slice::<Value>(&resp.body)?;

        // Extract embeddings: sort data by index, then map to embedding arrays.
        let embeddings: Vec<Vec<f32>> = raw_value
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                let mut indexed: Vec<(u64, Vec<f32>)> = arr
                    .iter()
                    .map(|item| {
                        let index = item
                            .get("index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0);
                        let embedding = item
                            .get("embedding")
                            .and_then(|e| e.as_array())
                            .map(|vals| {
                                vals.iter()
                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                    .collect()
                            })
                            .unwrap_or_default();
                        (index, embedding)
                    })
                    .collect();
                indexed.sort_by_key(|(i, _)| *i);
                indexed.into_iter().map(|(_, e)| e).collect()
            })
            .unwrap_or_default();

        // Extract usage: response.usage.total_tokens (defaults to 0)
        let tokens = raw_value
            .get("usage")
            .and_then(|u| u.get("total_tokens"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;

        Ok(EmbeddingResult {
            embeddings,
            usage: Some(EmbeddingUsage { tokens }),
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

struct VoyageEmbeddingProviderOptions {
    input_type: Option<String>,
    truncation: Option<bool>,
    output_dimension: Option<u32>,
    output_dtype: Option<String>,
}

fn parse_voyage_provider_options(
    options: Option<&SharedProviderOptions>,
) -> VoyageEmbeddingProviderOptions {
    let provider_opts = options.and_then(|opts| opts.get("voyage"));
    VoyageEmbeddingProviderOptions {
        input_type: provider_opts
            .and_then(|o| o.get("inputType"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        truncation: provider_opts
            .and_then(|o| o.get("truncation"))
            .and_then(serde_json::Value::as_bool),
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
