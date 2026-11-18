use std::sync::OnceLock;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    Credential, FetchFunction, HeaderMapOpt, Resolvable, validate_base_url,
};

use crate::shared::{EndpointConfig, EndpointHeaders};

use super::search_model::DataforseoSearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.dataforseo.com";
const LOGIN_ENV_VAR: &str = "DATAFORSEO_LOGIN";
const PASSWORD_ENV_VAR: &str = "DATAFORSEO_PASSWORD";
const DEFAULT_NAME: &str = "dataforseo";

/// Settings of [`create_dataforseo`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; the credentials and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct DataforseoProviderSettings {
    /// Base URL for the API calls. Default `https://api.dataforseo.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The HTTP Basic login. `None` loads `DATAFORSEO_LOGIN` when a request is
    /// made and fails that request with `AiMuxError::LoadApiKey` if it is
    /// unset. An explicit value is used as given, `""` included.
    pub login: Option<Resolvable<String>>,
    /// The HTTP Basic password. `None` loads `DATAFORSEO_PASSWORD` when a
    /// request is made and fails that request with `AiMuxError::LoadApiKey`
    /// if it is unset. An explicit value is used as given, `""` included.
    pub password: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` string
    /// (`"{name}.search"`). Default `"dataforseo"`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for DataforseoProviderSettings {
    /// Never prints the credentials or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataforseoProviderSettings")
            .field("base_url", &self.base_url)
            .field("login", &self.login)
            .field("password", &self.password)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a DataForSEO provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the credentials are
/// loaded per request, not here.
pub fn create_dataforseo(
    settings: DataforseoProviderSettings,
) -> Result<DataforseoProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(DataforseoProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: EndpointHeaders::Basic {
            login: Credential::explicit_or_env(settings.login, LOGIN_ENV_VAR, "DataForSEO login"),
            password: Credential::explicit_or_env(
                settings.password,
                PASSWORD_ENV_VAR,
                "DataForSEO password",
            ),
            user: settings.headers,
        },
        fetch: settings.fetch,
    })
}

/// The default provider: `create_dataforseo` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing login or password surfaces from the first request instead.
pub fn dataforseo() -> &'static DataforseoProvider {
    static DEFAULT: OnceLock<DataforseoProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_dataforseo(DataforseoProviderSettings::default())
            .expect("default DataForSEO settings are always valid")
    })
}

/// A DataForSEO provider. Search only; it holds no HTTP client.
pub struct DataforseoProvider {
    name: String,
    base_url: String,
    headers: EndpointHeaders,
    fetch: Option<FetchFunction>,
}

impl DataforseoProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> DataforseoSearchModel {
        DataforseoSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(DataforseoProvider, search_model, |p, _id| p.search_model());
