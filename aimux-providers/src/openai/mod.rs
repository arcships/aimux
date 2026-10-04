//! OpenAI provider.
//!
//! [`create_openai`] is the Rust form of the AI SDK's `createOpenAI`: it takes
//! [`OpenAIProviderSettings`], validates the base URL, fixes the provider name
//! and returns an [`OpenAIProvider`]. The API key is not read there; it is
//! loaded in the request headers of every call, from the setting or from
//! `OPENAI_API_KEY`. [`openai()`] is the default instance.
//!
//! This package is the native OpenAI API only. Servers that merely speak the
//! same wire format are served by [`crate::openai_compatible`] (and the
//! registry presets built on it).

pub(crate) mod config;
pub mod convert;
mod convert_common;
pub mod embedding;
pub mod files;
pub mod image;
pub mod model;
pub mod responses;
pub mod speech;
pub mod transcription;
mod types;

pub use config::TransformRequestBody;
pub use embedding::OpenAIEmbeddingModel;
pub use files::OpenAIFiles;
pub use image::OpenAIImageModel;
pub use model::OpenAIModel;
pub use responses::OpenAIResponsesModel;
pub use speech::OpenAISpeechModel;
pub use transcription::OpenAITranscriptionModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::files_model::Files;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, provider_headers};

use config::OpenAIModelConfig;

/// The chat-completions model of the native package (`provider.chat(id)`).
pub type OpenAIChatModel = OpenAIModel;

pub(crate) fn openai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
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

pub(crate) fn openai_stream_error(
    error: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("OpenAI stream failed before any output was generated")
        .to_owned();
    let code = error.get("code").or_else(|| error.get("type"));
    let provider_code = code.and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    });
    // Only a numeric HTTP status in the payload is a status; a string code
    // ("invalid_api_key") must not be laundered into a retryable 500.
    let status_code = code
        .and_then(Value::as_u64)
        .filter(|status| (400..=599).contains(status))
        .map(|status| status as u16);

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

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";

/// Settings of [`create_openai`] (the AI SDK's `OpenAIProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are evaluated
/// on every request.
#[derive(Clone, Default)]
pub struct OpenAIProviderSettings {
    /// Base URL for the API calls. Default `https://api.openai.com/v1`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `OPENAI_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Sent as `OpenAI-Organization`.
    pub organization: Option<String>,
    /// Sent as `OpenAI-Project`.
    pub project: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including one of the fixed ones. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of every model's `provider()` string.
    /// Default `"openai"`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for OpenAIProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field("organization", &self.organization)
            .field("project", &self.project)
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

/// Create an OpenAI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_openai(settings: OpenAIProviderSettings) -> Result<OpenAIProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    let name = settings.name.unwrap_or_else(|| "openai".to_string());
    Ok(OpenAIProvider {
        name,
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "OpenAI"),
            [
                ("OpenAI-Organization", settings.organization),
                ("OpenAI-Project", settings.project),
            ]
            .into_iter()
            .filter_map(|(name, value)| value.map(|value| (name.to_string(), value)))
            .collect(),
            settings.headers,
        ),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_openai` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn openai() -> &'static OpenAIProvider {
    static DEFAULT: OnceLock<OpenAIProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        // The default settings carry no base URL, so validation has nothing to
        // reject.
        create_openai(OpenAIProviderSettings::default())
            .expect("default OpenAI settings are always valid")
    })
}

/// An OpenAI provider (the AI SDK's `OpenAIProvider`). Cheap to clone the
/// models out of; it holds no HTTP client.
pub struct OpenAIProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
}

impl OpenAIProvider {
    fn model_config(&self, method: &str) -> OpenAIModelConfig {
        OpenAIModelConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        )
    }

    /// A chat-completions model; `provider()` is `"{name}.chat"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> OpenAIChatModel {
        OpenAIModel::from_config(model_id.to_string(), self.model_config("chat"))
    }

    /// A Responses API model; `provider()` is `"{name}.responses"`.
    #[must_use]
    pub fn responses(&self, model_id: &str) -> OpenAIResponsesModel {
        OpenAIResponsesModel::from_config(model_id.to_string(), self.model_config("responses"))
    }

    /// An embedding model; `provider()` is `"{name}.embedding"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> OpenAIEmbeddingModel {
        OpenAIEmbeddingModel::from_config(model_id.to_string(), self.model_config("embedding"))
    }

    /// An image model; `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> OpenAIImageModel {
        OpenAIImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }

    /// A speech (TTS) model; `provider()` is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> OpenAISpeechModel {
        OpenAISpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }

    /// A transcription (STT) model; `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> OpenAITranscriptionModel {
        OpenAITranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }

    /// The files interface; `provider()` is `"{name}.files"`.
    #[must_use]
    pub fn files(&self) -> OpenAIFiles {
        OpenAIFiles::from_config(self.model_config("files"))
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; it returns the chat model for now, the same
    /// model as [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for OpenAIProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Ok(Arc::new(self.image(model_id)))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.transcription(model_id))))
    }

    fn speech_model(&self, model_id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.speech(model_id))))
    }

    fn files(&self) -> Option<Arc<dyn Files>> {
        Some(Arc::new(self.files()))
    }
}

impl ProviderDiscovery for OpenAIProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move { model::list_models_once(&config).await })
    }
}
