//! Serper provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Serper search API
//! (`POST https://google.serper.dev/search`). Uses `X-API-KEY` header auth
//! via `SERPER_API_KEY`.
//!
//! [`create_serper`] takes [`SerperProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`SerperProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `SERPER_API_KEY`.
//! [`serper()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, ProviderHeaders};

const MODEL_ID: &str = "serper-search";

const DEFAULT_BASE_URL: &str = "https://google.serper.dev";
const API_KEY_ENV_VAR: &str = "SERPER_API_KEY";
const DEFAULT_NAME: &str = "serper";

/// Settings of [`create_serper`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct SerperProviderSettings {
    /// Base URL for the API calls. Default `https://google.serper.dev`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `SERPER_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.search"`).
    /// Default `"serper"`. The providerOptions key stays `serper`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for SerperProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerperProviderSettings")
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

/// Create a Serper provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_serper(settings: SerperProviderSettings) -> Result<SerperProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(SerperProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::new(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Serper"),
            AuthScheme::Header("X-API-KEY"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_serper` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn serper() -> &'static SerperProvider {
    static DEFAULT: OnceLock<SerperProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_serper(SerperProviderSettings::default())
            .expect("default Serper settings are always valid")
    })
}

/// A Serper provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct SerperProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl SerperProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> SerperSearchModel {
        SerperSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(SerperProvider, search_model, |p, _id| p.search_model());

fn build_request_body(options: &SearchCallOptions) -> Value {
    let mut body = json!({
        "q": options.query,
    });
    if let Some(max) = options.max_results {
        body["num"] = json!(max);
    }
    body
}

#[derive(Debug, Deserialize)]
struct SerperOrganicResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerperResponse {
    #[serde(default)]
    organic: Vec<SerperOrganicResult>,
}

fn map_results(entries: Vec<SerperOrganicResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.link,
            content: r.snippet,
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// A Serper search model.
pub struct SerperSearchModel {
    config: EndpointConfig,
}

impl SerperSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for SerperSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let body = build_request_body(options);
        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/search"), options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;
        let response_body = resp.raw_value;
        let parsed: SerperResponse = resp.value;

        Ok(SearchResult {
            results: map_results(parsed.organic),
            answer: None,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: response_body,
            }),
        })
    }
}
