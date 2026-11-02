//! The fal provider (`fal-provider.ts`, `fal-config.ts`).

use std::sync::OnceLock;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AuthScheme, Credential, FetchFunction, HeaderMapOpt, ProviderHeaders, Resolvable,
    validate_base_url,
};

use crate::shared::EndpointConfig;

use super::image_model::FalImageModel;
use super::transcription_model::FalTranscriptionModel;
use super::video_model::FalVideoModel;

/// The default `base_url` (`createFal`'s `defaultBaseURL`). Upstream uses it
/// for the image model; the transcription and video models hard-code the
/// queue host themselves (see [`DEFAULT_QUEUE_BASE_URL`]).
pub(crate) const DEFAULT_BASE_URL: &str = "https://fal.run";
/// The host the transcription and video models talk to, which upstream
/// hard-codes in `fal-transcription-model.ts` and `fal-video-model.ts`
/// rather than reading `baseURL`.
pub(crate) const DEFAULT_QUEUE_BASE_URL: &str = "https://queue.fal.run";
pub(crate) const API_KEY_ENV_VAR: &str = "FAL_KEY";
const DEFAULT_NAME: &str = "fal";

/// Settings of [`create_fal`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct FalProviderSettings {
    /// Base URL for the API calls. Default `https://fal.run` for images and
    /// `https://queue.fal.run` for transcription and video, as upstream; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `FAL_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.transcription"`, `"{name}.video"`, `"{name}.image"`).
    /// Default `"fal"`. The providerOptions key stays `fal`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for FalProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FalProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Fal provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_fal(settings: FalProviderSettings) -> Result<FalProvider, AiMuxError> {
    let explicit_base_url = match settings.base_url.as_deref() {
        Some(url) => Some(validate_base_url(url)?),
        None => None,
    };
    Ok(FalProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url: explicit_base_url,
        headers: ProviderHeaders::new(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Fal"),
            AuthScheme::Scheme("Key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_fal` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn fal() -> &'static FalProvider {
    static DEFAULT: OnceLock<FalProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_fal(FalProviderSettings::default()).expect("default Fal settings are always valid")
    })
}

/// A Fal provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct FalProvider {
    name: String,
    /// The caller's `base_url`; `None` leaves each model on its own default.
    base_url: Option<String>,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl FalProvider {
    /// The endpoint of one model method: the caller's `base_url`, or the
    /// host that model defaults to (upstream reads `baseURL` for images
    /// only and hard-codes the queue host in the other two).
    fn model_config(&self, method: &str) -> EndpointConfig {
        let default = match method {
            "image" => DEFAULT_BASE_URL,
            _ => DEFAULT_QUEUE_BASE_URL,
        };
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.as_deref().unwrap_or(default).to_string(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A transcription (STT) model (e.g. `"wizper"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> FalTranscriptionModel {
        FalTranscriptionModel::from_config(model_id.to_string(), self.model_config("transcription"))
    }

    /// A video model (e.g. `"fal-ai/kling-video"`); `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> FalVideoModel {
        FalVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }

    /// An image model (e.g. `"fal-ai/flux/schnell"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> FalImageModel {
        FalImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

impl ::aimux_core::Provider for FalProvider {
    fn language_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "languageModel"))
    }

    fn embedding_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::ImageModel>, AiMuxError> {
        Ok(::std::sync::Arc::new(self.image(model_id)))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<::std::sync::Arc<dyn ::aimux_core::TranscriptionModel>, AiMuxError>> {
        Some(Ok(::std::sync::Arc::new(self.transcription(model_id))))
    }

    fn video_model(
        &self,
        model_id: &str,
    ) -> Option<Result<::std::sync::Arc<dyn ::aimux_core::VideoModel>, AiMuxError>> {
        Some(Ok(::std::sync::Arc::new(self.video(model_id))))
    }
}
