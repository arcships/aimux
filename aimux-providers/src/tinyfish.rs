//! TinyFish search provider — implements the `SearchModel` trait.
//!
//! Implements the TinyFish search API
//! (`GET https://api.search.tinyfish.ai?query=...&count=...`).
//!
//! Authentication uses the `X-API-Key` header (env `TINYFISH_API_KEY`).
//! TinyFish is a search-only provider: it exposes a web search protocol
//! (query → results) and does not support language models.
//!
//! [`create_tinyfish`] takes [`TinyfishProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`TinyfishProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `TINYFISH_API_KEY`.
//! [`tinyfish()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, credential_headers};

/// Fixed model id for the TinyFish search model.
const MODEL_ID: &str = "tinyfish-search";

/// Default number of results when `max_results` is unset.
const DEFAULT_COUNT: u32 = 10;

// ── Config ───────────────────────────────────────────────────────────────────

const DEFAULT_BASE_URL: &str = "https://api.search.tinyfish.ai";
const API_KEY_ENV_VAR: &str = "TINYFISH_API_KEY";
const DEFAULT_NAME: &str = "tinyfish";

/// Settings of [`create_tinyfish`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct TinyfishProviderSettings {
    /// Base URL for the API calls. Default `https://api.search.tinyfish.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `TINYFISH_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.search"`).
    /// Default `"tinyfish"`. The providerOptions key stays `tinyfish`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for TinyfishProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TinyfishProviderSettings")
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

/// Create a TinyFish provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_tinyfish(settings: TinyfishProviderSettings) -> Result<TinyfishProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(TinyfishProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: credential_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "TinyFish"),
            AuthScheme::Header("X-API-Key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_tinyfish` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn tinyfish() -> &'static TinyfishProvider {
    static DEFAULT: OnceLock<TinyfishProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_tinyfish(TinyfishProviderSettings::default())
            .expect("default TinyFish settings are always valid")
    })
}

/// A TinyFish provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct TinyfishProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl TinyfishProvider {
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
    pub fn search_model(&self) -> TinyfishSearchModel {
        TinyfishSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(TinyfishProvider, search_model, |p, _id| p.search_model());

// ── Request builder ──────────────────────────────────────────────────────────

/// Resolve the `count` query parameter from the call options.
///
/// Pure function: maps `max_results` to the TinyFish `count` parameter,
/// defaulting to [`DEFAULT_COUNT`] when unset.
fn resolve_count(max_results: Option<u32>) -> u32 {
    max_results.unwrap_or(DEFAULT_COUNT)
}

// ── Response types ───────────────────────────────────────────────────────────

/// The response from the TinyFish search endpoint.
///
/// Only the fields used by the trait are deserialized; extra fields returned
/// by the API (e.g. `query`, `total_results`, per-result `position` /
/// `site_name`) are ignored so unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct TinyfishSearchResponse {
    #[serde(default)]
    results: Vec<TinyfishResult>,
}

#[derive(Debug, Deserialize)]
struct TinyfishResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Map a TinyFish result into a [`SearchResultItem`].
///
/// Field mapping: `title` → `title`, `url` → `url`, `snippet` → `content`.
fn map_result(r: TinyfishResult) -> SearchResultItem {
    SearchResultItem {
        title: r.title,
        url: r.url,
        content: r.snippet,
        raw_content: None,
        score: None,
        provider_metadata: None,
    }
}

// ── Search model ─────────────────────────────────────────────────────────────

/// A TinyFish search model.
pub struct TinyfishSearchModel {
    config: EndpointConfig,
}

impl TinyfishSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for TinyfishSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let count = resolve_count(options.max_results);
        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let mut url = url::Url::parse(exchange.base_url())
            .map_err(|e| AiMuxError::InvalidArgument(format!("invalid tinyfish endpoint: {e}")))?;
        url.query_pairs_mut()
            .append_pair("query", &options.query)
            .append_pair("count", &count.to_string());

        let resp = aimux_provider_utils::get_from_api(
            exchange.request(url.to_string(), options),
            aimux_provider_utils::create_json_response_handler::<TinyfishSearchResponse>(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        let results: Vec<SearchResultItem> = data.results.into_iter().map(map_result).collect();

        Ok(SearchResult {
            results,
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
