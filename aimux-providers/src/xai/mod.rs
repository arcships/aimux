//! xAI (Grok) provider.
//!
//! [`create_xai`] is the Rust form of the AI SDK's `createXai`: it takes
//! [`XAIProviderSettings`], validates the base URL, fixes the provider name and
//! returns an [`XAIProvider`]. The API key is not read there; it is loaded in
//! the request headers of every call, from the setting or from
//! `XAI_API_KEY`. [`xai()`] is the default instance.
//!
//! The AI SDK's xAI package serves the Responses API only
//! ([`XaiResponsesModel`], `provider()` = `xai.responses`), and
//! [`language_model`](Provider::language_model) returns it. There is no Chat
//! Completions model in this package; xAI's OpenAI-compatible endpoint is
//! reachable through `create_openai_compatible` with `https://api.x.ai/v1`.

pub mod convert;
pub(crate) mod options;
pub mod responses;

pub use crate::shared::TransformRequestBody;
pub use responses::XaiResponsesModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

const DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
const API_KEY_ENV_VAR: &str = "XAI_API_KEY";
const DEFAULT_NAME: &str = "xai";

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

/// Settings of [`create_xai`] (the AI SDK's `XaiProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct XAIProviderSettings {
    /// Base URL for the API calls. Default `https://api.x.ai/v1`; a trailing
    /// slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `XAI_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings
    /// (`"{name}.responses"`). Default `"xai"`. The providerOptions key stays
    /// `xai`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for XAIProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XAIProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// Create an xAI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_xai(settings: XAIProviderSettings) -> Result<XAIProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(XAIProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "xAI API key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_xai` with default settings, created on first
/// use. Creating it reads nothing from the environment and cannot fail; a
/// missing key surfaces from the first request instead.
pub fn xai() -> &'static XAIProvider {
    static DEFAULT: OnceLock<XAIProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_xai(XAIProviderSettings::default()).expect("default xAI settings are always valid")
    })
}

/// An xAI provider (the AI SDK's `XaiProvider`). Cheap to clone the models out
/// of; it holds no HTTP client.
pub struct XAIProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
}

impl XAIProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        )
    }

    /// A Responses model (e.g. `"grok-4"`); `provider()` is
    /// `"{name}.responses"`.
    ///
    /// Uses the xAI `/responses` endpoint with the Responses API wire format
    /// (input items, reasoning objects, provider-executed tools, etc.).
    #[must_use]
    pub fn responses(&self, model_id: &str) -> XaiResponsesModel {
        XaiResponsesModel::from_config(model_id.to_string(), self.model_config("responses"))
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as
    /// [`responses`](Self::responses) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.responses(model_id))
    }
}

impl Provider for XAIProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

impl ProviderDiscovery for XAIProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move {
            crate::shared::list_data_models(&config, xai_failed_response_handler()).await
        })
    }
}
