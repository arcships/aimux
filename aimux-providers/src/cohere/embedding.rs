//! Cohere embedding model — implements the `EmbeddingModel` trait.
//!
//! Aligned with Vercel AI SDK `CohereEmbeddingModel`
//! (`reference/ai/packages/cohere/src/cohere-embedding-model.ts`).
//!
//! Endpoint: `POST {base_url}/embed`

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

use super::CohereConfig;

/// A Cohere embedding model (e.g. `"embed-english-v3.0"`).
pub struct CohereEmbeddingModel {
    model_id: String,
    config: CohereConfig,
}

impl CohereEmbeddingModel {
    pub fn new(model_id: String, config: CohereConfig) -> Self {
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
        format!("{}/embed", self.config.base_url)
    }
}

#[async_trait]
impl EmbeddingModel for CohereEmbeddingModel {
    fn provider(&self) -> &str {
        "cohere"
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
        let cohere_options = parse_cohere_provider_options(options.provider_options.as_ref());

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

        let headers = self.build_headers(options.headers.as_ref());

        let mut header_list: Vec<(String, String)> =
            headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        header_list.push(("Content-Type".to_string(), "application/json".to_string()));

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: header_list,
                body: HttpBody::Json(Value::Object(body)),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let raw_value: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        // Extract embeddings: response.embeddings.float
        let embeddings: Vec<Vec<f32>> = raw_value
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
            .unwrap_or_default();

        // Extract usage: response.meta.billed_units.input_tokens
        let usage = raw_value
            .get("meta")
            .and_then(|m| m.get("billed_units"))
            .and_then(|b| b.get("input_tokens"))
            .and_then(|t| t.as_u64())
            .map(|tokens| EmbeddingUsage {
                tokens: tokens as u32,
            })
            .unwrap_or_default();

        Ok(EmbeddingResult {
            embeddings,
            usage: Some(usage),
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

struct CohereEmbeddingProviderOptions {
    input_type: Option<String>,
    truncate: Option<String>,
    output_dimension: Option<u32>,
}

fn parse_cohere_provider_options(
    options: Option<&SharedProviderOptions>,
) -> CohereEmbeddingProviderOptions {
    let provider_opts = options.and_then(|opts| opts.get("cohere"));
    CohereEmbeddingProviderOptions {
        input_type: provider_opts
            .and_then(|o| o.get("inputType"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        truncate: provider_opts
            .and_then(|o| o.get("truncate"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        output_dimension: provider_opts
            .and_then(|o| o.get("outputDimension"))
            .and_then(|d| d.as_u64())
            .map(|d| d as u32),
    }
}
