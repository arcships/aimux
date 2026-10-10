//! The OpenAI-compatible embedding model (`{name}.embedding`).
//!
//! The Rust form of `OpenAICompatibleEmbeddingModel`: `POST {base_url}/embeddings`
//! with `encoding_format: "float"`, the `dimensions` and `user` options read
//! from the compatible namespaces and then the provider's own.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::embedding_model::{
    EmbeddingCallOptions, EmbeddingModel, EmbeddingResponse, EmbeddingResult, EmbeddingUsage,
};
use aimux_core::error::AiMuxError;

use aimux_core::shared::Warning;

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

/// Validate each namespace before merging, as upstream's provider-options schema does.
fn embedding_options(
    options: &EmbeddingCallOptions,
    name: &str,
    warnings: &mut Vec<Warning>,
) -> Result<Map<String, Value>, AiMuxError> {
    let mut merged = Map::new();
    for key in ["openai-compatible", "openaiCompatible", name] {
        let Some(object) = options
            .provider_options
            .as_ref()
            .and_then(|all| all.get(key))
        else {
            continue;
        };
        for (setting, valid) in [
            ("dimensions", Value::is_number as fn(&Value) -> bool),
            ("user", Value::is_string),
        ] {
            if let Some(value) = object.get(setting) {
                if !valid(value) {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "invalid providerOptions.{key}.{setting}"
                    )));
                }
                merged.insert(setting.to_string(), value.clone());
            }
        }
    }
    if options
        .provider_options
        .as_ref()
        .is_some_and(|all| all.contains_key("openai-compatible"))
    {
        warnings.push(Warning::Deprecated {
            setting: "providerOptions key 'openai-compatible'".into(),
            message: "Use 'openaiCompatible' instead.".into(),
        });
    }
    let camel = to_camel_case(name);
    if camel != name
        && options
            .provider_options
            .as_ref()
            .is_some_and(|all| all.contains_key(name))
    {
        warnings.push(Warning::Deprecated {
            setting: format!("providerOptions key '{name}'"),
            message: format!("Use '{camel}' instead."),
        });
    }
    Ok(merged)
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
            return Err(AiMuxError::InvalidArgument(format!(
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
                .map_err(|error| AiMuxError::InvalidResponseData(error.to_string()))?,
            response: Some(EmbeddingResponse {
                headers: Some(response_headers),
                body: Some(raw_value),
            }),
            warnings,
        })
    }
}
