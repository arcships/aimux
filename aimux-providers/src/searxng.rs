//! SearXNG provider — search modality only.
//!
//! Implements the `SearchModel` trait against a self-hosted SearXNG instance
//! (`GET {SEARXNG_URL}/search?q=...&format=json`).
//!
//! SearXNG is a self-hosted, normally unauthenticated provider: there is no
//! default instance. The instance URL is the `base_url` setting or the
//! `SEARXNG_URL` environment variable, read when a request is made (a missing
//! URL fails that request with `AiMuxError::LoadSetting`). A `403` response
//! typically indicates that the `json` output format is not enabled on the
//! instance.
//!
//! [`create_searxng`] takes [`SearxngProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`SearxngProvider`]. The instance URL is not read
//! there: the instance URL is resolved for every request.
//! [`searxng()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{BaseUrl, Credential, EndpointConfig, EndpointSource, ProviderHeaders};

/// Fixed model ID for the SearXNG search model.
const MODEL_ID: &str = "searxng-search";

const BASE_URL_ENV_VAR: &str = "SEARXNG_URL";
const DEFAULT_NAME: &str = "searxng";

/// Settings of [`create_searxng`].
///
/// Every field is optional. The instance URL is validated when the provider is
/// created if it is given; `api_key` and `headers` are evaluated on every
/// request.
#[derive(Clone, Default)]
pub struct SearxngProviderSettings {
    /// The instance URL; a trailing slash is removed. `None` reads
    /// `SEARXNG_URL` when a request is made and fails that request with
    /// `AiMuxError::LoadSetting` if it is unset: there is no default instance.
    pub base_url: Option<String>,
    /// An optional bearer token for an instance behind an authenticating
    /// proxy. `None` sends no `Authorization` header. A [`Resolvable::Future`]
    /// is awaited once, an [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header.
    /// Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` string
    /// (`"{name}.search"`). Default `"searxng"`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for SearxngProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearxngProviderSettings")
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

/// Create a SearXNG provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is given and is not
/// an `http(s)` URL with a host. That is the only way this fails: a missing
/// instance URL is reported by the first request.
pub fn create_searxng(settings: SearxngProviderSettings) -> Result<SearxngProvider, AiMuxError> {
    let base_url = settings
        .base_url
        .as_deref()
        .map(validate_base_url)
        .transpose()?
        .map_or(
            BaseUrl::Env {
                var: BASE_URL_ENV_VAR,
                description: "SearXNG instance URL",
            },
            BaseUrl::Fixed,
        );
    let credential = match settings.api_key {
        Some(key) => Credential::Explicit(key),
        None => Credential::None,
    };
    Ok(SearxngProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::bearer(credential, Vec::new(), settings.headers),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_searxng` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing instance URL surfaces from the first request instead.
pub fn searxng() -> &'static SearxngProvider {
    static DEFAULT: OnceLock<SearxngProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_searxng(SearxngProviderSettings::default())
            .expect("default SearXNG settings are always valid")
    })
}

/// A SearXNG provider. Search only; it holds no HTTP client.
pub struct SearxngProvider {
    name: String,
    base_url: BaseUrl,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl SearxngProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::new(
            format!("{}.{method}", self.name),
            EndpointSource::Http {
                base_url: self.base_url.clone(),
                headers: self.headers.clone().into(),
            },
            self.fetch.clone(),
        )
    }

    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> SearxngSearchModel {
        SearxngSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(SearxngProvider, search_model, |p, _id| p.search_model());

/// A single SearXNG result entry. All fields are optional so unknown-but-legal
/// values degrade safely.
#[derive(Debug, Deserialize)]
struct SearxngResult {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
    // `engine` and other extra fields returned by SearXNG are ignored.
    #[serde(default)]
    score: Option<f64>,
}

/// The response from the SearXNG `/search` endpoint.
#[derive(Debug, Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngResult>,
}

fn map_results(entries: Vec<SearxngResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            content: r.content,
            raw_content: None,
            score: r.score,
            provider_metadata: None,
        })
        .collect()
}

/// A SearXNG search model.
pub struct SearxngSearchModel {
    config: EndpointConfig,
}

impl SearxngSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for SearxngSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let mut url = url::Url::parse(&exchange.url("/search"))
            .map_err(|e| AiMuxError::InvalidArgument(format!("invalid searxng endpoint: {e}")))?;
        url.query_pairs_mut()
            .append_pair("q", &options.query)
            .append_pair("format", "json");

        let resp = aimux_provider_utils::get_from_api(
            exchange.request(url.to_string(), options),
            aimux_provider_utils::create_json_response_handler::<SearxngResponse>(),
            aimux_provider_utils::create_status_code_error_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        Ok(SearchResult {
            results: map_results(data.results),
            answer: None,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
