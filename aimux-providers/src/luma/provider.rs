//! The Luma provider (`luma-provider.ts`).

use std::sync::OnceLock;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, ProviderHeaders};

use super::image_model::LumaImageModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.lumalabs.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "LUMA_API_KEY";
const DEFAULT_NAME: &str = "luma";

/// Settings of [`create_luma`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct LumaProviderSettings {
    /// Base URL for the API calls. Default `https://api.lumalabs.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `LUMA_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.image"`).
    /// Default `"luma"`. The providerOptions key stays `luma`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for LumaProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LumaProviderSettings")
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

/// Create a Luma provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_luma(settings: LumaProviderSettings) -> Result<LumaProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(LumaProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::bearer(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Luma"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_luma` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn luma() -> &'static LumaProvider {
    static DEFAULT: OnceLock<LumaProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_luma(LumaProviderSettings::default())
            .expect("default Luma settings are always valid")
    })
}

/// A Luma provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct LumaProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl LumaProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// An image model (e.g. `"photon-1"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> LumaImageModel {
        LumaImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(LumaProvider, image_model, |p, id| p.image(id));
