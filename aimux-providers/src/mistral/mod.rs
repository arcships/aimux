//! Mistral AI provider.
//!
//! OpenAI-compatible chat completions API with Mistral-specific differences:
//! - Tool choice uses `"any"` instead of `"required"`
//! - Content can be a string or an array of typed parts (text, thinking, image_url)
//! - Usage supports `num_cached_tokens`
//! - Finish reasons include `model_length`

pub mod convert;
pub mod embedding;
mod model;
mod types;

pub use embedding::MistralEmbeddingModel;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{RetryConfig, load_api_key, without_trailing_slash};
use serde_json::Value;

pub(crate) fn mistral_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: data
                .get("code")
                .or_else(|| data.get("type"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}

pub(crate) fn mistral_stream_error(
    error: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let status_code = error
        .get("status_code")
        .or_else(|| error.get("status"))
        .or_else(|| error.get("code"))
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok())
        .filter(|status| (400..=599).contains(status));
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Mistral stream failed before any output was generated")
        .to_owned();
    let provider_code =
        error
            .get("code")
            .or_else(|| error.get("type"))
            .and_then(|value| match value {
                Value::String(code) => Some(code.clone()),
                Value::Number(code) => Some(code.to_string()),
                _ => None,
            });
    aimux_provider_utils::stream_error_api_call(
        message,
        provider_code,
        status_code,
        error,
        url,
        request_body_values,
        response_headers,
    )
}

/// Configuration for the Mistral provider.
#[derive(Debug, Clone)]
pub struct MistralConfig {
    pub api_key: String,
    pub base_url: String,
    /// api_key 来源(RFC-0023):`None` = explicit;`Some("env:VAR")` = 环境变量。
    pub api_key_source: Option<String>,
    /// Retry settings used by Core model operations.
    pub retry_config: RetryConfig,
}

impl MistralConfig {
    /// Create from an API key (uses default Mistral base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.mistral.ai/v1".to_string(),
            api_key_source: None,
            retry_config: RetryConfig::default(),
        }
    }

    /// Use a custom base URL.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// 标注 api_key 来源(RFC-0023 回放重建用)。
    #[must_use]
    pub fn with_api_key_source(mut self, source: Option<&str>) -> Self {
        self.api_key_source = source.map(std::string::ToString::to_string);
        self
    }

    /// Set the retry configuration. Pass `max_retries: 0` to disable retries.
    #[must_use]
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Create from environment variable `MISTRAL_API_KEY`.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `MISTRAL_API_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "MISTRAL_API_KEY", "Mistral")?;
        Ok(Self::new(api_key).with_api_key_source(Some("env:MISTRAL_API_KEY")))
    }
}

/// Mistral provider — creates `MistralModel` instances.
pub struct MistralProvider {
    config: MistralConfig,
}

impl MistralProvider {
    #[must_use]
    pub fn new(config: MistralConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given model name (e.g. `"mistral-small-latest"`).
    #[must_use]
    pub fn model(&self, model_id: &str) -> model::MistralModel {
        model::MistralModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an embedding model instance for the given model name (e.g.
    /// `"mistral-embed"`).
    #[must_use]
    pub fn embedding_model(&self, model_id: &str) -> embedding::MistralEmbeddingModel {
        embedding::MistralEmbeddingModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for MistralProvider {
    fn name(&self) -> &str {
        "mistral"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    /// List models via `GET {base_url}/models` (OpenAI-compatible, RFC-0027).
    fn list_models(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError>,
                > + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        Box::pin(async move {
            let base = config.base_url.trim_end_matches('/');
            let url = format!("{base}/models");
            let headers = vec![
                (
                    "Authorization".to_string(),
                    format!("Bearer {}", config.api_key),
                ),
                ("Content-Type".to_string(), "application/json".to_string()),
            ];
            use aimux_provider_utils::HttpRequest;
            // Retry rationale: see `openai::model::execute_list_models`.
            let resp = aimux_core::retry::prepare_retries(None, config.retry_config, None)
                .retry(|| {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: url.clone(),
                            headers: headers.clone(),
                            abort_signal: None,
                            call_id: None,
                            recording_context: None,
                        },
                        aimux_provider_utils::create_json_response_handler(),
                        mistral_failed_response_handler(),
                    )
                })
                .await?;
            #[derive(serde::Deserialize)]
            struct Resp {
                #[serde(default)]
                data: Vec<Entry>,
            }
            #[derive(serde::Deserialize)]
            struct Entry {
                id: String,
                #[serde(default)]
                owned_by: Option<String>,
            }
            let parsed: Resp = resp.value;
            let runtime: Vec<aimux_core::model_catalogue::RuntimeModel> = parsed
                .data
                .into_iter()
                .map(|e| aimux_core::model_catalogue::RuntimeModel {
                    id: e.id,
                    owned_by: e.owned_by,
                    created: None,
                })
                .collect();
            Ok(runtime)
        })
    }
}
