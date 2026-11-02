//! The Deepgram provider (`deepgram-provider.ts`).

use std::sync::OnceLock;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AuthScheme, Credential, FetchFunction, HeaderMapOpt, ProviderHeaders, Resolvable,
    validate_base_url,
};

use crate::shared::EndpointConfig;

use super::transcription_model::DeepgramTranscriptionModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.deepgram.com";
pub(crate) const API_KEY_ENV_VAR: &str = "DEEPGRAM_API_KEY";
const DEFAULT_NAME: &str = "deepgram";

/// Settings of [`create_deepgram`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct DeepgramProviderSettings {
    /// Base URL for the API calls. Default `https://api.deepgram.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `DEEPGRAM_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.transcription"`).
    /// Default `"deepgram"`. The providerOptions key stays `deepgram`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for DeepgramProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeepgramProviderSettings")
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

/// Create a Deepgram provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_deepgram(settings: DeepgramProviderSettings) -> Result<DeepgramProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(DeepgramProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::new(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Deepgram"),
            AuthScheme::Scheme("Token"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_deepgram` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn deepgram() -> &'static DeepgramProvider {
    static DEFAULT: OnceLock<DeepgramProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_deepgram(DeepgramProviderSettings::default())
            .expect("default Deepgram settings are always valid")
    })
}

/// A Deepgram provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct DeepgramProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl DeepgramProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A transcription (STT) model (e.g. `"nova-3"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> DeepgramTranscriptionModel {
        DeepgramTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(DeepgramProvider, transcription_model, |p, id| p
    .transcription(id));
