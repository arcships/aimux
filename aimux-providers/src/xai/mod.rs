//! xAI (Grok) provider — a thin wrapper with xAI-specific behaviour.
//!
//! xAI exposes an OpenAI-compatible Chat Completions API at
//! `https://api.x.ai/v1`. While the wire format is OpenAI-compatible, xAI has
//! enough provider-specific behaviour (reasoning content extraction, citations,
//! search parameters, xai-keyed provider options, non-inclusive cached tokens,
//! reasoning-effort model gating, 200-status errors) to warrant its own model
//! implementation ([`XaiModel`]) rather than reusing `OpenAIModel`.

pub mod convert;
mod model;
pub mod responses;
mod types;

pub use model::XaiModel;
pub use responses::XaiResponsesModel;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::openai::OpenAIConfig;

const DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
const ENV_VAR: &str = "XAI_API_KEY";
const PROVIDER_NAME: &str = "xai";

pub(crate) fn xai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        if let (Some(code), Some(message)) = (
            data.get("code").and_then(Value::as_str),
            data.get("error").and_then(Value::as_str),
        ) {
            return aimux_provider_utils::ProviderErrorParts {
                message: format!("{code}: {message}"),
                provider_code: Some(code.to_owned()),
            };
        }
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("code")
                .or_else(|| error.get("type"))
                .and_then(|value| match value {
                    Value::String(value) => Some(value.clone()),
                    Value::Number(value) => Some(value.to_string()),
                    _ => None,
                }),
        }
    })
}

pub(crate) fn xai_stream_error(
    event: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let error = event.get("error").unwrap_or(event);
    let message = error
        .as_str()
        .or_else(|| error.get("message").and_then(Value::as_str))
        .or_else(|| event.get("message").and_then(Value::as_str))
        .unwrap_or("xAI stream failed");
    let status_code = event
        .get("status")
        .or_else(|| event.get("code"))
        .or_else(|| error.get("status"))
        .or_else(|| error.get("code"))
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok())
        .filter(|status| (400..=599).contains(status));
    let provider_code = event
        .get("code")
        .or_else(|| error.get("code"))
        .or_else(|| error.get("type"))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        });
    aimux_provider_utils::stream_error_api_call(
        message,
        provider_code,
        status_code,
        event,
        url,
        request_body_values,
        response_headers,
    )
}

pub(crate) fn xai_successful_response_handler<T>() -> aimux_provider_utils::ResponseHandler<T>
where
    T: DeserializeOwned + Send + 'static,
{
    aimux_provider_utils::ResponseHandler::new(|input| async move {
        let status = input.response.status().as_u16();
        let url = input.url.clone();
        let request_body_values = input.request_body_values.clone();
        let output = aimux_provider_utils::create_json_response_handler::<Value>()
            .handle(input)
            .await?;
        let headers = output.response_headers.clone();
        let raw = output.value;
        if let Some(error) = raw.get("error").filter(|value| !value.is_null()) {
            let message = error
                .as_str()
                .or_else(|| error.get("message").and_then(Value::as_str))
                .unwrap_or("xAI request failed");
            let provider_code = raw
                .get("code")
                .or_else(|| error.get("code"))
                .or_else(|| error.get("type"))
                .and_then(|value| match value {
                    Value::String(value) => Some(value.clone()),
                    Value::Number(value) => Some(value.to_string()),
                    _ => None,
                });
            return Err(AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                status_code: Some(status),
                provider_code,
                response_body: Some(raw.to_string()),
                response_headers: Some(headers),
                ..aimux_core::ApiCallError::new(message, url, request_body_values)
            })));
        }
        // Borrow `raw` instead of `from_value(raw.clone())`: the whole body is
        // already a `Value` here and is handed back as `raw_value`, so the
        // clone was a second full tree resident at peak.
        let value = serde::Deserialize::deserialize(&raw).map_err(|error: serde_json::Error| {
            AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                status_code: Some(status),
                response_body: Some(raw.to_string()),
                response_headers: Some(headers.clone()),
                ..aimux_core::ApiCallError::new(
                    format!("Invalid JSON response: {error}"),
                    url,
                    request_body_values,
                )
            }))
        })?;
        Ok(aimux_provider_utils::ResponseHandlerOutput {
            value,
            raw_value: Some(raw),
            response_headers: output.response_headers,
        })
    })
}

/// xAI occasionally returns a JSON error document with a successful status
/// where an SSE response was requested. Classify that response before handing
/// the body to the standard typed event-source handler.
pub(crate) fn xai_event_source_response_handler<T>()
-> aimux_provider_utils::ResponseHandler<futures::stream::BoxStream<'static, Result<T, AiMuxError>>>
where
    T: DeserializeOwned + Send + 'static,
{
    aimux_provider_utils::ResponseHandler::new(|input| async move {
        let is_json = input
            .response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("application/json"));
        if !is_json {
            return aimux_provider_utils::create_event_source_response_handler::<T>()
                .handle(input)
                .await;
        }

        let status = input.response.status().as_u16();
        let url = input.url.clone();
        let request_body_values = input.request_body_values.clone();
        let output = aimux_provider_utils::create_json_response_handler::<Value>()
            .handle(input)
            .await?;
        let headers = output.response_headers;
        let raw = output.value;
        let error = raw.get("error").filter(|value| !value.is_null());
        let message = error
            .and_then(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("message").and_then(Value::as_str))
            })
            .unwrap_or("Expected an event stream but received JSON");
        let provider_code = raw
            .get("code")
            .or_else(|| error.and_then(|value| value.get("code")))
            .or_else(|| error.and_then(|value| value.get("type")))
            .and_then(|value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            });
        Err(AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
            status_code: Some(status),
            provider_code,
            response_body: Some(raw.to_string()),
            response_headers: Some(headers),
            ..aimux_core::ApiCallError::new(message, url, request_body_values)
        })))
    })
    .streaming()
}

/// Configuration for the xAI provider (wraps [`OpenAIConfig`]).
#[derive(Clone)]
pub struct XAIConfig(OpenAIConfig);

impl XAIConfig {
    /// Create from an API key, using the default xAI base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(OpenAIConfig::new(api_key).with_base_url(DEFAULT_BASE_URL))
    }

    /// Create from the `XAI_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `XAI_API_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "xAI")?;
        Ok(Self::new(key).with_api_key_source(Some("env:XAI_API_KEY")))
    }

    /// 标注 api_key 来源(RFC-0023 回放重建用)。透传到内部 `OpenAIConfig`。
    #[must_use]
    pub fn with_api_key_source(mut self, source: Option<&str>) -> Self {
        self.0 = self.0.with_api_key_source(source);
        self
    }

    /// 内部 `OpenAIConfig` 引用(config_snapshot 复用 OpenAI helper 用,M2b)。
    pub(crate) fn openai_config(&self) -> &OpenAIConfig {
        &self.0
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }

    /// Get the API key.
    pub(crate) fn api_key(&self) -> &str {
        &self.0.api_key
    }

    /// Get the base URL.
    pub(crate) fn base_url(&self) -> &str {
        &self.0.base_url
    }
}

/// xAI provider — creates [`XaiModel`] instances pointed at xAI.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct XAIProvider {
    config: XAIConfig,
}

impl XAIProvider {
    #[must_use]
    pub fn new(config: XAIConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given xAI model id (e.g. `"grok-2"`).
    ///
    /// Clones the provider config so the model inherits the same
    /// `api_key_source` / `max_retries` (M2b: previously reconstructed with
    /// `XAIConfig::new`, which dropped the credential source).
    #[must_use]
    pub fn model(&self, model_id: &str) -> XaiModel {
        XaiModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a Responses API model instance for the given xAI model id.
    ///
    /// Uses the xAI `/responses` endpoint with the Responses API wire format
    /// (input items, reasoning objects, provider-executed tools, etc.).
    #[must_use]
    pub fn responses_model(&self, model_id: &str) -> XaiResponsesModel {
        XaiResponsesModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for XAIProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
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
        let config = OpenAIConfig::new(self.config.api_key())
            .with_base_url(self.config.base_url())
            .with_provider(PROVIDER_NAME);
        // The freshly built OpenAIConfig carries the default retry settings —
        // use the user's configured ones from the wrapped config instead.
        let retry_config = self.config.0.retry_config;
        Box::pin(async move {
            let headers = crate::openai::model::build_auth_headers(&config);
            let runtime =
                crate::openai::model::execute_list_models(&config.base_url, &headers, retry_config)
                    .await?;
            Ok(runtime)
        })
    }
}
