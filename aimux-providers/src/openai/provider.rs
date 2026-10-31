//! The OpenAI provider factory (openai-provider.ts): [`create_openai`],
//! [`openai()`] and [`OpenAIProvider`], with the `/models` listing of
//! provider discovery (aimux, RFC-0027).

use std::sync::{Arc, OnceLock};

use aimux_core::decision_model::DecisionModel;
use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::files_model::Files;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;
use aimux_provider_utils::{
    DecisionLanguageModel, FetchFunction, HeaderMapOpt, HttpRequest, Resolvable, validate_base_url,
};
use futures::future::BoxFuture;

use crate::openai::chat::chat_language_model::OpenAIModel;
use crate::openai::completion::completion_language_model::OpenAICompletionModel;
use crate::openai::config::OpenAIModelConfig;
use crate::openai::decision::OpenAIDecisionModel;
use crate::openai::embedding::embedding_model::OpenAIEmbeddingModel;
use crate::openai::files::files::OpenAIFiles;
use crate::openai::image::image_model::OpenAIImageModel;
use crate::openai::responses::responses_language_model::OpenAIResponsesModel;
use crate::openai::speech::speech_model::OpenAISpeechModel;
use crate::openai::transcription::transcription_model::OpenAITranscriptionModel;
use crate::shared::{Credential, ProviderHeaders};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub(crate) const API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";

/// Settings of [`create_openai`] (the AI SDK's `OpenAIProviderSettings`).
///
/// Every field is optional. The base URL (including `OPENAI_BASE_URL`) and
/// name are resolved when the provider is created; `api_key` and `headers` are evaluated
/// on every request.
#[derive(Clone, Default)]
pub struct OpenAIProviderSettings {
    /// Base URL for the API calls. `None` reads `OPENAI_BASE_URL` when the
    /// provider is created, else `https://api.openai.com/v1`; a trailing
    /// slash is removed. An invalid explicit value fails [`create_openai`];
    /// an invalid `OPENAI_BASE_URL` fails every request instead.
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
/// Returns `AiMuxError::invalid_argument` when the explicit `base_url` is not
/// an `http(s)` URL with a host. That is the only way this fails: the key is
/// loaded per request, and an invalid `OPENAI_BASE_URL` is reported by each
/// request, not here.
pub fn create_openai(settings: OpenAIProviderSettings) -> Result<OpenAIProvider, AiMuxError> {
    let base_url = aimux_provider_utils::base_url_setting(
        settings.base_url.as_deref(),
        "OPENAI_BASE_URL",
        DEFAULT_BASE_URL,
        validate_base_url,
    )?;
    let name = settings.name.unwrap_or_else(|| "openai".to_string());
    Ok(OpenAIProvider {
        name,
        base_url,
        headers: ProviderHeaders::bearer(
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
        )
        .with_user_agent(
            crate::shared::aisdk_pins::OPENAI.name,
            crate::shared::aisdk_pins::OPENAI.version,
        ),
        fetch: Some(crate::shared::package_fetch(
            settings.fetch,
            crate::shared::aisdk_pins::OPENAI,
        )),
        #[cfg(feature = "realtime")]
        web_socket: settings.web_socket,
        tools: super::tools::OpenAITools,
    })
}

/// The default provider: `create_openai` with default settings, created on
/// first use. `OPENAI_BASE_URL` is read at creation (an invalid value fails
/// each request, not the creation); the key is loaded per request.
pub fn openai() -> &'static OpenAIProvider {
    static DEFAULT: OnceLock<OpenAIProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_openai(OpenAIProviderSettings::default())
            .expect("default settings have no explicit base URL to reject")
    })
}

/// An OpenAI provider (the AI SDK's `OpenAIProvider`). Cheap to clone the
/// models out of; it holds no HTTP client.
pub struct OpenAIProvider {
    name: String,
    /// The validated base URL, or the error of an invalid `OPENAI_BASE_URL`,
    /// which every request reports.
    base_url: Result<String, AiMuxError>,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
    /// The provider tools (`openai.tools` in the AI SDK).
    pub tools: super::tools::OpenAITools,
}

impl OpenAIProvider {
    fn model_config(&self, method: &str) -> OpenAIModelConfig {
        let mut config = OpenAIModelConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone().unwrap_or_default(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        );
        config.supported_urls = super::config::supported_urls(method);
        if method == "responses" {
            config.responses.file_id_prefixes = vec!["file-"];
        }
        if let Err(error) = &self.base_url {
            config.url = super::config::OpenAIUrl::Invalid(error.clone());
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
        OpenAICompletionModel::from_config(model_id.to_string(), self.model_config("completion"))
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

    /// A native Decisions model (an aimux extension: the AI SDK's OpenAI
    /// provider has no native decision endpoint). Its `provider()` is
    /// `"openai"`. It is the provider's [`decision_model`](Provider::decision_model).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an empty model id.
    pub fn decision(&self, model_id: &str) -> Result<OpenAIDecisionModel, AiMuxError> {
        OpenAIDecisionModel::from_config(model_id, self.model_config("decision"))
    }

    /// Decisions from structured Responses output; `provider()` is
    /// `"{name}.evaluation"` (`@ai-sdk/openai@4.0.83
    /// src/openai-provider.ts:355-358`, `evaluationModel`).
    #[must_use]
    pub fn evaluation(&self, model_id: &str) -> DecisionLanguageModel {
        DecisionLanguageModel::new(
            Arc::new(self.responses(model_id)),
            Some(format!("{}.evaluation", self.name)),
        )
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

    /// The native Decisions model; the registry takes it before the
    /// evaluation model (`ai@7.0.128 src/registry/provider-registry.ts:447-461`),
    /// which stays reachable as [`OpenAIProvider::evaluation`].
    fn decision_model(&self, model_id: &str) -> Option<Result<Arc<dyn DecisionModel>, AiMuxError>> {
        Some(
            self.decision(model_id)
                .map(|model| Arc::new(model) as Arc<dyn DecisionModel>),
        )
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
        Box::pin(async move { list_models_once(&config).await })
    }
}

// ── Model listing (RFC-0027) ─────────────────────────────────────────────────

/// OpenAI-compatible `/models` response shape.
#[derive(serde::Deserialize)]
struct ModelsListResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(serde::Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<u64>,
}

/// One `GET {base_url}/models` exchange (OpenAI-compatible): no retry, no
/// recording. Discovery is not a Core operation and the AI SDK has no
/// equivalent, so a failure is reported to the caller as it happened.
///
/// # Errors
///
/// Returns the header-resolution error (a missing key is `LoadApiKey`),
/// `ApiCall` for HTTP/transport failures and `JsonParse` when the body does
/// not deserialize into the models list.
pub(crate) async fn list_models_once(
    config: &OpenAIModelConfig,
) -> Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError> {
    let headers = config.request_headers(None).await?;
    let resp = aimux_provider_utils::get_from_api(
        config.with_transport(HttpRequest {
            url: config.url("/models")?,
            headers,
            ..Default::default()
        }),
        aimux_provider_utils::create_json_response_handler(),
        crate::openai::error::openai_failed_response_handler(),
    )
    .await?;
    let parsed: ModelsListResponse = resp.value;

    Ok(parsed
        .data
        .into_iter()
        .map(|m| aimux_core::model_catalogue::RuntimeModel {
            id: m.id,
            owned_by: m.owned_by,
            created: m.created,
        })
        .collect())
}
