//! The ElevenLabs provider (`elevenlabs-provider.ts`, `elevenlabs-config.ts`).

use std::sync::{Arc, OnceLock};

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;
use aimux_provider_utils::{
    AuthScheme, Credential, FetchFunction, HeaderMapOpt, ProviderHeaders, Resolvable,
    validate_base_url,
};

use crate::shared::{EndpointConfig, TransformRequestBody};

use super::speech_model::ElevenLabsSpeechModel;
use super::transcription_model::ElevenLabsTranscriptionModel;

// ── Settings ─────────────────────────────────────────────────────────────────

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
pub(crate) const API_KEY_ENV_VAR: &str = "ELEVENLABS_API_KEY";
const DEFAULT_NAME: &str = "elevenlabs";

/// Settings of [`create_elevenlabs`] (the AI SDK's `ElevenLabsProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request, including the WebSocket handshake of realtime
/// transcription.
#[derive(Clone, Default)]
pub struct ElevenLabsProviderSettings {
    /// Base URL for the API calls. Default `https://api.elevenlabs.io`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key, sent as `xi-api-key`. `None` loads `ELEVENLABS_API_KEY`
    /// when a request is made and fails that request with
    /// `AiMuxError::LoadApiKey` if it is unset. An explicit value is used as
    /// given, `""` included: it never falls back to the environment. A
    /// [`Resolvable::Future`] is awaited once, an [`Resolvable::AsyncFn`] on
    /// every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `xi-api-key`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings
    /// (`"{name}.speech"`, `"{name}.transcription"`). Default `"elevenlabs"`.
    /// The providerOptions key stays `elevenlabs`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
    /// Opens the WebSocket of realtime transcription (the AI SDK's
    /// `webSocket`). `None` uses the built-in tungstenite connector.
    #[cfg(feature = "realtime")]
    pub web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl std::fmt::Debug for ElevenLabsProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("ElevenLabsProviderSettings");
        debug
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
            );
        #[cfg(feature = "realtime")]
        debug.field("web_socket", &self.web_socket.is_some());
        debug.finish()
    }
}

/// Create an ElevenLabs provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_elevenlabs(
    settings: ElevenLabsProviderSettings,
) -> Result<ElevenLabsProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(ElevenLabsProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::new(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "ElevenLabs"),
            AuthScheme::Header("xi-api-key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
        #[cfg(feature = "realtime")]
        web_socket: settings.web_socket,
    })
}

/// The default provider: `create_elevenlabs` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing key surfaces from the first request instead.
pub fn elevenlabs() -> &'static ElevenLabsProvider {
    static DEFAULT: OnceLock<ElevenLabsProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_elevenlabs(ElevenLabsProviderSettings::default())
            .expect("default ElevenLabs settings are always valid")
    })
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// ElevenLabs provider (the AI SDK's `ElevenLabsProvider`) — creates speech
/// and transcription models. Cheap to clone the models out of; it holds no
/// HTTP client.
pub struct ElevenLabsProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl ElevenLabsProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        )
    }

    /// A speech (TTS) model (e.g. `"eleven_multilingual_v2"`); `provider()`
    /// is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> ElevenLabsSpeechModel {
        ElevenLabsSpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }

    /// A transcription (STT) model (e.g. `"scribe_v1"`); `provider()` is
    /// `"{name}.transcription"`. Uses the `/v1/speech-to-text` endpoint.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> ElevenLabsTranscriptionModel {
        ElevenLabsTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
            #[cfg(feature = "realtime")]
            self.web_socket.clone(),
        )
    }
}

impl Provider for ElevenLabsProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "languageModel"))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
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
}
