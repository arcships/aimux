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

pub mod completion;
pub(crate) mod config;
pub mod convert;
mod convert_common;
pub mod embedding;
pub mod files;
pub mod image;
pub mod model;
pub(crate) mod options;
pub mod responses;
pub mod speech;
pub mod tools;
pub mod transcription;
mod types;

pub use completion::OpenAICompletionModel;
pub use embedding::OpenAIEmbeddingModel;
pub use files::OpenAIFiles;
pub use image::OpenAIImageModel;
pub use model::OpenAIModel;
pub use responses::OpenAIResponsesModel;
pub use speech::OpenAISpeechModel;
pub use tools::{OpenAIComputerAction, OpenAIComputerSafetyCheck};
pub use transcription::OpenAITranscriptionModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::evaluation_model::EvaluationModel;
use aimux_core::files_model::Files;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;
use aimux_provider_utils::{
    EvaluationLanguageModel, FetchFunction, HeaderMapOpt, HeadersFn, Resolvable,
    load_optional_setting, validate_base_url,
};

use crate::shared::{Credential, provider_headers};

use config::OpenAIModelConfig;

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
    let payload = if error.get("type").and_then(Value::as_str) == Some("response.failed") {
        error
            .get("response")
            .and_then(|response| response.get("error"))
            .unwrap_or(error)
    } else {
        error.get("error").unwrap_or(error)
    };
    let message = payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("OpenAI stream failed before any output was generated")
        .to_owned();
    let provider_code = payload.get("code").and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    });
    let error_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let discriminator = format!(
        "{} {error_type}",
        provider_code.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    let explicit_status = payload
        .get("code")
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .or_else(|| {
                    value
                        .as_str()
                        .filter(|value| {
                            value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit())
                        })
                        .and_then(|value| value.parse::<u16>().ok())
                })
        })
        .filter(|value| (400..=599).contains(value));
    let status = explicit_status.unwrap_or_else(|| {
        if ["insufficient_quota", "rate_limit"]
            .iter()
            .any(|term| discriminator.contains(term))
        {
            429
        } else if discriminator.contains("authentication") {
            401
        } else if discriminator.contains("permission") {
            403
        } else if discriminator.contains("not_found") {
            404
        } else if ["invalid", "bad_request", "context_length"]
            .iter()
            .any(|term| discriminator.contains(term))
        {
            400
        } else if discriminator.contains("overload") {
            503
        } else if discriminator.contains("timeout") {
            504
        } else {
            500
        }
    });
    let quota = provider_code.as_deref() == Some("insufficient_quota")
        || error_type == "insufficient_quota";
    let mut result = aimux_provider_utils::stream_error_api_call(
        message,
        provider_code,
        Some(status),
        error,
        url,
        request_body_values,
        response_headers,
    );
    if quota && let AiMuxError::ApiCall(details) = &mut result {
        details.is_retryable = false;
    }
    result
}

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";

/// Settings of [`create_openai`] (the AI SDK's `OpenAIProviderSettings`).
///
/// Every field is optional. The base URL (including `OPENAI_BASE_URL`) and
/// name are resolved when the provider is created; `api_key` and `headers` are evaluated
/// on every request.
#[derive(Clone, Default)]
pub struct OpenAIProviderSettings {
    /// Base URL for API calls. Reads `OPENAI_BASE_URL` when absent, then
    /// defaults to `https://api.openai.com/v1`. A trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `OPENAI_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment.
    pub api_key: Option<String>,
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
    /// Custom socket connector for streaming transcription.
    #[cfg(feature = "realtime")]
    pub web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl std::fmt::Debug for OpenAIProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.is_some())
            .field("organization", &self.organization)
            .field("project", &self.project)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
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
    let configured_url = load_optional_setting(settings.base_url.as_deref(), "OPENAI_BASE_URL");
    let base_url = match configured_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    let name = settings.name.unwrap_or_else(|| "openai".to_string());
    Ok(OpenAIProvider {
        name,
        base_url,
        headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
            provider_headers(
                Credential::explicit_or_env(
                    settings.api_key.map(Resolvable::Value),
                    API_KEY_ENV_VAR,
                    "OpenAI",
                ),
                [
                    ("OpenAI-Organization", settings.organization),
                    ("OpenAI-Project", settings.project),
                ]
                .into_iter()
                .filter_map(|(name, value)| value.map(|value| (name.to_string(), value)))
                .collect(),
                settings.headers,
            ),
            "openai",
            "4.0.80",
        ),
        fetch: settings.fetch,
        #[cfg(feature = "realtime")]
        web_socket: settings.web_socket,
    })
}

/// The default provider, created on first use. The base URL is read then;
/// a missing API key surfaces from the first request.
pub fn openai() -> &'static OpenAIProvider {
    static DEFAULT: OnceLock<OpenAIProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_openai(OpenAIProviderSettings::default())
            .expect("default OpenAI base URL must be valid")
    })
}

/// An OpenAI provider (the AI SDK's `OpenAIProvider`). Cheap to clone the
/// models out of; it holds no HTTP client.
pub struct OpenAIProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl OpenAIProvider {
    fn model_config(&self, method: &str) -> OpenAIModelConfig {
        let mut config = OpenAIModelConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        );
        config.supported_urls = config::supported_urls(method);
        if method == "responses" {
            config.responses.file_id_prefixes = vec!["file-"];
        }
        config
    }

    /// A chat-completions model; `provider()` is `"{name}.chat"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> OpenAIModel {
        OpenAIModel::from_config(model_id.to_string(), self.model_config("chat"))
    }

    /// A text-completion model; `provider()` is `"{name}.completion"`.
    #[must_use]
    pub fn completion(&self, model_id: &str) -> OpenAICompletionModel {
        OpenAICompletionModel::from_native_config(
            model_id.to_string(),
            self.model_config("completion"),
        )
    }

    /// A Responses API model; `provider()` is `"{name}.responses"`.
    #[must_use]
    pub fn responses(&self, model_id: &str) -> OpenAIResponsesModel {
        let mut config = self.model_config("responses");
        config.responses.file_id_prefixes = vec!["file-"];
        OpenAIResponsesModel::from_config(model_id.to_string(), config)
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
        let model = OpenAITranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        );
        #[cfg(feature = "realtime")]
        let model = model.with_web_socket(self.web_socket.clone());
        model
    }

    /// The files interface; `provider()` is `"{name}.files"`.
    #[must_use]
    pub fn files(&self) -> OpenAIFiles {
        OpenAIFiles::from_config(self.model_config("files"))
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; it returns the Responses model, the same
    /// model as [`responses`](Self::responses) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.responses(model_id))
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

    fn evaluation_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn EvaluationModel>, AiMuxError>> {
        Some(Ok(Arc::new(EvaluationLanguageModel::new(
            Arc::new(self.responses(model_id)),
            Some(format!("{}.evaluation", self.name)),
        ))))
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
