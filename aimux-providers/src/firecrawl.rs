//! Firecrawl provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Firecrawl search API
//! (`POST https://api.firecrawl.dev/v2/search`). Bearer auth via
//! `FIRECRAWL_API_KEY`.
//!
//! [`create_firecrawl`] takes [`FirecrawlProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`FirecrawlProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `FIRECRAWL_API_KEY`.
//! [`firecrawl()`] is the default instance; it reads nothing and cannot fail.

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

const MODEL_ID: &str = "firecrawl-search";

/// Firecrawl-specific error structure: `{ "success": false, "error": "..." }`.
fn firecrawl_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Firecrawl request failed")
                .to_string(),
            provider_code: None,
        }
    })
}

const DEFAULT_BASE_URL: &str = "https://api.firecrawl.dev";
const API_KEY_ENV_VAR: &str = "FIRECRAWL_API_KEY";
const DEFAULT_NAME: &str = "firecrawl";

/// Settings of [`create_firecrawl`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct FirecrawlProviderSettings {
    /// Base URL for the API calls. Default `https://api.firecrawl.dev`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `FIRECRAWL_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.search"`).
    /// Default `"firecrawl"`. The providerOptions key stays `firecrawl`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for FirecrawlProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirecrawlProviderSettings")
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

/// Create a Firecrawl provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_firecrawl(
    settings: FirecrawlProviderSettings,
) -> Result<FirecrawlProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(FirecrawlProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::bearer(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Firecrawl"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_firecrawl` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn firecrawl() -> &'static FirecrawlProvider {
    static DEFAULT: OnceLock<FirecrawlProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_firecrawl(FirecrawlProviderSettings::default())
            .expect("default Firecrawl settings are always valid")
    })
}

/// A Firecrawl provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct FirecrawlProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl FirecrawlProvider {
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
    pub fn search_model(&self) -> FirecrawlSearchModel {
        FirecrawlSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(FirecrawlProvider, search_model, |p, _id| p.search_model());

fn build_request_body(options: &SearchCallOptions) -> Value {
    let mut body = json!({
        "query": options.query,
        "sources": ["web"],
    });
    if let Some(max) = options.max_results {
        body["limit"] = json!(max);
    }
    if let Some(include) = &options.include_domains {
        body["includeDomains"] = json!(include);
    }
    if let Some(exclude) = &options.exclude_domains {
        body["excludeDomains"] = json!(exclude);
    }
    body
}

#[derive(Debug, Deserialize)]
struct FirecrawlWebResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    markdown: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FirecrawlData {
    #[serde(default)]
    web: Vec<FirecrawlWebResult>,
}

#[derive(Debug, Deserialize)]
struct FirecrawlResponse {
    #[serde(default)]
    data: FirecrawlData,
}

fn map_results(entries: Vec<FirecrawlWebResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            content: r.markdown,
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// A Firecrawl search model.
pub struct FirecrawlSearchModel {
    config: EndpointConfig,
}

impl FirecrawlSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for FirecrawlSearchModel {
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
            exchange.request(exchange.url("/v2/search"), options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            firecrawl_failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;
        let response_body = resp.raw_value;
        let parsed: FirecrawlResponse = resp.value;

        Ok(SearchResult {
            results: map_results(parsed.data.web),
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
