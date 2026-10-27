//! `mistral-provider.ts`: `createMistral`, `MistralProviderSettings` and
//! the default `mistral` instance.

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, ProviderHeaders, SupportedUrlsSource};

use super::error::mistral_failed_response_handler;
use super::{MistralEmbeddingModel, MistralModel, MistralSpeechModel, MistralTranscriptionModel};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.mistral.ai/v1";
pub(crate) const API_KEY_ENV_VAR: &str = "MISTRAL_API_KEY";

/// The URL patterns the chat model fetches itself (`supportedUrls` of the AI
/// SDK's `MistralChatLanguageModel`): `https` PDFs.
fn chat_supported_urls() -> SupportedUrls {
    let https = regex::Regex::new(r"^https://.*$").expect("static pattern");
    SupportedUrls(std::iter::once(("application/pdf".to_string(), vec![https])).collect())
}

/// Settings of [`create_mistral`] (the AI SDK's `MistralProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct MistralProviderSettings {
    /// Base URL for the API calls. Default `https://api.mistral.ai/v1`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `MISTRAL_API_KEY` when a request is made and
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
    /// Generates identifiers for reasoning and streamed tool calls.
    pub generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl std::fmt::Debug for MistralProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MistralProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.is_some())
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("fetch", &self.fetch.is_some())
            .field("generate_id", &self.generate_id.is_some())
            .finish()
    }
}

/// Create a Mistral provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_mistral(settings: MistralProviderSettings) -> Result<MistralProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    let headers = ProviderHeaders::bearer(
        Credential::explicit_or_env(
            settings.api_key.map(Resolvable::Value),
            API_KEY_ENV_VAR,
            "Mistral",
        ),
        Vec::new(),
        settings.headers,
    )
    .with_user_agent(
        crate::shared::aisdk_pins::MISTRAL.name,
        crate::shared::aisdk_pins::MISTRAL.version,
    );
    Ok(MistralProvider {
        base_url,
        headers,
        fetch: Some(crate::shared::package_fetch(
            settings.fetch,
            crate::shared::aisdk_pins::MISTRAL,
        )),
        generate_id: settings.generate_id,
    })
}

/// The default provider: `create_mistral` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn mistral() -> &'static MistralProvider {
    static DEFAULT: OnceLock<MistralProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_mistral(MistralProviderSettings::default())
            .expect("default Mistral settings are always valid")
    })
}

/// A Mistral provider (the AI SDK's `MistralProvider`). Cheap to clone the
/// models out of; it holds no HTTP client.
pub struct MistralProvider {
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
    generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl MistralProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("mistral.{method}"),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A chat model; `provider()` is `"mistral.chat"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> MistralModel {
        MistralModel::from_config(
            model_id.to_string(),
            self.model_config("chat")
                .with_supported_urls(SupportedUrlsSource::Fixed(chat_supported_urls())),
            self.generate_id.clone(),
        )
    }

    /// An embedding model (e.g. `"mistral-embed"`); `provider()` is
    /// `"mistral.embedding"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> MistralEmbeddingModel {
        MistralEmbeddingModel::from_config(model_id.to_string(), self.model_config("embedding"))
    }

    /// A speech (TTS) model (e.g. `"voxtral-mini-tts-2603"`); `provider()` is
    /// `"mistral.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> MistralSpeechModel {
        MistralSpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }

    /// A transcription (STT) model (e.g. `"voxtral-mini-latest"`);
    /// `provider()` is `"mistral.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> MistralTranscriptionModel {
        MistralTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as [`chat`](Self::chat) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for MistralProvider {
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
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }

    fn speech_model(&self, model_id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.speech(model_id))))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.transcription(model_id))))
    }
}

impl ProviderDiscovery for MistralProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move {
            crate::shared::list_data_models(&config, mistral_failed_response_handler()).await
        })
    }
}
