//! Prodia provider factory and configuration.

use std::sync::OnceLock;

use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AuthScheme, Credential, FetchFunction, HeaderMapOpt, ProviderHeaders, Resolvable,
    validate_base_url,
};

use crate::shared::EndpointConfig;

use super::image_model::ProdiaImageModel;
use super::video_model::ProdiaVideoModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://prodia.com/api";
pub(crate) const API_KEY_ENV_VAR: &str = "PRODIA_API_KEY";
const DEFAULT_NAME: &str = "prodia";

pub(super) fn prodia_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let detail = data.get("detail");
        let message = detail
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                detail
                    .filter(|value| !value.is_null())
                    .map(Value::to_string)
            })
            .or_else(|| {
                data.get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| {
                data.get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Unknown Prodia error".to_string());
        aimux_provider_utils::ProviderErrorParts {
            message,
            provider_code: None,
        }
    })
}

/// Settings of [`create_prodia`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct ProdiaProviderSettings {
    /// Base URL for the API calls. Default `https://prodia.com/api`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `PRODIA_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.image"`, `"{name}.video"`).
    /// Default `"prodia"`. The providerOptions key stays `prodia`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for ProdiaProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProdiaProviderSettings")
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

/// Create a Prodia provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_prodia(settings: ProdiaProviderSettings) -> Result<ProdiaProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(ProdiaProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        credential: Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Prodia"),
        user_headers: settings.headers,
        fetch: settings.fetch,
    })
}

/// The default provider: `create_prodia` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn prodia() -> &'static ProdiaProvider {
    static DEFAULT: OnceLock<ProdiaProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_prodia(ProdiaProviderSettings::default())
            .expect("default Prodia settings are always valid")
    })
}

/// A Prodia provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct ProdiaProvider {
    name: String,
    base_url: String,
    credential: Credential,
    user_headers: Option<HeaderMapOpt>,
    fetch: Option<FetchFunction>,
}

impl ProdiaProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            match method {
                "image" => ProviderHeaders::new(
                    self.credential.clone(),
                    AuthScheme::Header("X-Prodia-Key"),
                    vec![(
                        "Accept".to_string(),
                        "multipart/form-data; image/png".to_string(),
                    )],
                    self.user_headers.clone(),
                ),
                _ => ProviderHeaders::new(
                    self.credential.clone(),
                    AuthScheme::Header("X-Prodia-Key"),
                    Vec::new(),
                    self.user_headers.clone(),
                ),
            },
            self.fetch.clone(),
            None,
        )
    }

    /// An image model (e.g. `"inference.flux-fast.schnell.txt2img.v2"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> ProdiaImageModel {
        ProdiaImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }

    /// A video model; `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> ProdiaVideoModel {
        ProdiaVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }
}

crate::impl_modality_provider!(ProdiaProvider {
    image_model: |p, id| p.image(id),
    video_model: |p, id| p.video(id),
});
