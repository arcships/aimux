//! The xAI provider factory and default instance (`xai-provider.ts`).

use std::sync::{Arc, OnceLock};

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;
use aimux_core::video_model::VideoModel;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable};
use futures::future::BoxFuture;

use crate::shared::{Credential, EndpointConfig, ProviderHeaders};

use super::error::xai_failed_response_handler;
use super::{XaiResponsesModel, XaiSpeechModel, XaiTranscriptionModel, XaiVideoModel};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
pub(crate) const API_KEY_ENV_VAR: &str = "XAI_API_KEY";

/// Settings of [`create_xai`] (the AI SDK's `XaiProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct XAIProviderSettings {
    /// Base URL for the API calls. Default `https://api.x.ai/v1`; a trailing
    /// slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `XAI_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment.
    pub api_key: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Opens the WebSocket of streaming transcription (the AI SDK's
    /// `webSocket`). `None` uses the built-in tungstenite connector.
    #[cfg(feature = "realtime")]
    pub web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl std::fmt::Debug for XAIProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("XAIProviderSettings");
        #[cfg(feature = "realtime")]
        debug.field("web_socket", &self.web_socket.is_some());
        debug
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.is_some())
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("fetch", &self.fetch.is_some());
        debug.finish()
    }
}

/// Create an xAI provider.
///
/// # Errors
///
/// Creation does not validate the URL or load the API key; request errors
/// surface when the model is called.
pub fn create_xai(settings: XAIProviderSettings) -> Result<XAIProvider, AiMuxError> {
    let base_url = settings
        .base_url
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    let base_url = base_url.strip_suffix('/').unwrap_or(&base_url).to_string();
    Ok(XAIProvider {
        base_url,
        headers: ProviderHeaders::bearer(
            Credential::explicit_or_env(
                settings.api_key.map(Resolvable::Value),
                API_KEY_ENV_VAR,
                "xAI API key",
            ),
            Vec::new(),
            settings.headers,
        )
        .with_user_agent(
            crate::shared::aisdk_pins::XAI.name,
            crate::shared::aisdk_pins::XAI.version,
        ),
        fetch: Some(crate::shared::package_fetch(
            settings.fetch,
            crate::shared::aisdk_pins::XAI,
        )),
        #[cfg(feature = "realtime")]
        web_socket: settings.web_socket,
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
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl XAIProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("xai.{method}"),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A Responses model (e.g. `"grok-4"`); `provider()` is
    /// `"xai.responses"`.
    ///
    /// Uses the xAI `/responses` endpoint with the Responses API wire format
    /// (input items, reasoning objects, provider-executed tools, etc.).
    #[must_use]
    pub fn responses(&self, model_id: &str) -> XaiResponsesModel {
        XaiResponsesModel::from_config(model_id.to_string(), self.model_config("responses"))
    }

    /// A video model (e.g. `"grok-imagine-video"`); `provider()` is
    /// `"xai.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> XaiVideoModel {
        XaiVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }

    /// The speech (TTS) model; `provider()` is `"xai.speech"`. It has no
    /// model id.
    #[must_use]
    pub fn speech(&self) -> XaiSpeechModel {
        XaiSpeechModel::from_config(self.model_config("speech"))
    }

    /// The transcription (STT) model; `provider()` is `"xai.transcription"`.
    /// It has no model id.
    #[must_use]
    pub fn transcription(&self) -> XaiTranscriptionModel {
        XaiTranscriptionModel::from_config(
            self.model_config("transcription"),
            #[cfg(feature = "realtime")]
            self.web_socket.clone(),
        )
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

    /// The model id is ignored: xAI has a single transcription model.
    fn transcription_model(
        &self,
        _model_id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.transcription())))
    }

    /// The model id is ignored: xAI has a single speech model.
    fn speech_model(&self, _model_id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.speech())))
    }

    fn video_model(&self, model_id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.video(model_id))))
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
