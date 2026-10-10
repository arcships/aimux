//! Tavily provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Tavily search API
//! (`POST https://api.tavily.com/search`). Bearer auth via `TAVILY_API_KEY`.
//!
//! [`create_tavily`] takes [`TavilyProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`TavilyProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `TAVILY_API_KEY`.
//! [`tavily()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, ProviderHeaders};

const MODEL_ID: &str = "tavily-search";

/// Tavily-specific error structure: `{ "detail": { "error": "..." } }`.
fn tavily_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("detail")
                .and_then(|detail| detail.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("Tavily request failed")
                .to_string(),
            provider_code: None,
        }
    })
}

const DEFAULT_BASE_URL: &str = "https://api.tavily.com";
const API_KEY_ENV_VAR: &str = "TAVILY_API_KEY";
const DEFAULT_NAME: &str = "tavily";

/// Settings of [`create_tavily`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct TavilyProviderSettings {
    /// Base URL for the API calls. Default `https://api.tavily.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `TAVILY_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.search"`).
    /// Default `"tavily"`. The providerOptions key stays `tavily`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for TavilyProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TavilyProviderSettings")
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

/// Create a Tavily provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_tavily(settings: TavilyProviderSettings) -> Result<TavilyProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(TavilyProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::bearer(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Tavily"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_tavily` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn tavily() -> &'static TavilyProvider {
    static DEFAULT: OnceLock<TavilyProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_tavily(TavilyProviderSettings::default())
            .expect("default Tavily settings are always valid")
    })
}

/// A Tavily provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct TavilyProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl TavilyProvider {
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
    pub fn search_model(&self) -> TavilySearchModel {
        TavilySearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(TavilyProvider, search_model, |p, _id| p.search_model());

fn build_request_body(options: &SearchCallOptions) -> Value {
    let mut body = json!({
        "query": options.query,
        "include_answer": false,
    });
    if let Some(max) = options.max_results {
        body["max_results"] = json!(max);
    }
    if let Some(include) = &options.include_domains {
        body["include_domains"] = json!(include);
    }
    if let Some(exclude) = &options.exclude_domains {
        body["exclude_domains"] = json!(exclude);
    }
    if let Some(raw) = options.include_raw_content {
        body["include_raw_contents"] = json!(raw);
    }
    body
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    raw_content: Option<String>,
    #[serde(default)]
    score: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    results: Vec<TavilyResult>,
    #[serde(default)]
    answer: Option<String>,
}

fn map_results(entries: Vec<TavilyResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            content: r.content,
            raw_content: r.raw_content,
            score: r.score,
            provider_metadata: None,
        })
        .collect()
}

/// A Tavily search model.
pub struct TavilySearchModel {
    config: EndpointConfig,
}

impl TavilySearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for TavilySearchModel {
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
            tavily_failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;
        let response_body = resp.raw_value;
        let parsed: TavilyResponse = resp.value;

        Ok(SearchResult {
            results: map_results(parsed.results),
            answer: parsed.answer,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: response_body,
            }),
        })
    }
}
