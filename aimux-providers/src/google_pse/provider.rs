use std::sync::OnceLock;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AuthScheme, Credential, FetchFunction, HeaderMapOpt, ProviderHeaders, Resolvable,
    validate_base_url,
};

use crate::shared::EndpointConfig;

use super::search_model::GooglePseSearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://www.googleapis.com/customsearch/v1";
pub(crate) const API_KEY_ENV_VAR: &str = "GOOGLE_API_KEY";
const DEFAULT_NAME: &str = "google_pse";

/// Settings of [`create_google_pse`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct GooglePseProviderSettings {
    /// The endpoint URL, query parameters excluded. Default
    /// `https://www.googleapis.com/customsearch/v1`; a trailing slash is
    /// removed.
    pub base_url: Option<String>,
    /// The API key, sent as the `key` query parameter. `None` loads
    /// `GOOGLE_API_KEY` when a request is made and fails that request with
    /// `AiMuxError::LoadApiKey` if it is unset. An explicit value is used as
    /// given, `""` included: it never falls back to the environment.
    pub api_key: Option<Resolvable<String>>,
    /// The search-engine ID (`cx`). `None` uses
    /// `providerOptions.google_pse.cx` of the call, then `GOOGLE_CSE_ID`;
    /// a call with none of them fails with `AiMuxError::invalid_argument`.
    pub cx: Option<String>,
    /// Extra headers on every request. A `None` value removes the header.
    /// Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` string
    /// (`"{name}.search"`). Default `"google_pse"`. The providerOptions key
    /// stays `google_pse`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for GooglePseProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GooglePseProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field("cx", &self.cx)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Google PSE provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_google_pse(
    settings: GooglePseProviderSettings,
) -> Result<GooglePseProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(GooglePseProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        credential: Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Google PSE"),
        cx: settings.cx,
        // The key travels in the query string, so the headers carry none.
        headers: ProviderHeaders::new(
            Credential::None,
            AuthScheme::Bearer,
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_google_pse` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing key surfaces from the first request instead.
pub fn google_pse() -> &'static GooglePseProvider {
    static DEFAULT: OnceLock<GooglePseProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_google_pse(GooglePseProviderSettings::default())
            .expect("default Google PSE settings are always valid")
    })
}

/// A Google PSE provider. Search only; it holds no HTTP client.
pub struct GooglePseProvider {
    name: String,
    base_url: String,
    credential: Credential,
    cx: Option<String>,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl GooglePseProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> GooglePseSearchModel {
        GooglePseSearchModel::from_config(
            EndpointConfig::fixed(
                format!("{}.search", self.name),
                self.base_url.clone(),
                self.headers.clone(),
                self.fetch.clone(),
                None,
            ),
            self.credential.clone(),
            self.cx.clone(),
        )
    }
}

crate::impl_single_modality_provider!(GooglePseProvider, search_model, |p, _id| p.search_model());
