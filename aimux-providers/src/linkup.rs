//! Linkup provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Linkup search API
//! (`POST https://api.linkup.so/v1/search`).
//!
//! Linkup is a modality-specific provider: it exposes a native web search
//! protocol (query → results) and does not support language models. The
//! `depth` and `outputType` request fields are passed through via
//! `provider_options` under the `"linkup"` key.
//!
//! [`create_linkup`] takes [`LinkupProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`LinkupProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `LINKUP_API_KEY`.
//! [`linkup()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

/// Fixed model ID for the Linkup search model.
const MODEL_ID: &str = "linkup-search";

/// Linkup provider-specific search options (passed through via
/// `provider_options["linkup"]`).
#[derive(Debug, Clone, Default)]
struct LinkupOptions {
    /// Search depth: `"standard"` (default) or `"deep"`.
    depth: Option<String>,
    /// Output type: `"searchResults"` (default) or `"sourcedAnswer"`.
    output_type: Option<String>,
}

fn parse_linkup_options(provider_options: Option<&SharedProviderOptions>) -> LinkupOptions {
    let mut opts = LinkupOptions::default();
    if let Some(linkup) = options::linkup_options(provider_options) {
        if let Some(v) = linkup.get("depth").and_then(|v| v.as_str()) {
            opts.depth = Some(v.to_string());
        }
        if let Some(v) = linkup.get("outputType").and_then(|v| v.as_str()) {
            opts.output_type = Some(v.to_string());
        }
    }
    opts
}

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.linkup.so";
const API_KEY_ENV_VAR: &str = "LINKUP_API_KEY";
const DEFAULT_NAME: &str = "linkup";

/// Settings of [`create_linkup`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct LinkupProviderSettings {
    /// Base URL for the API calls. Default `https://api.linkup.so`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `LINKUP_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.search"`).
    /// Default `"linkup"`. The providerOptions key stays `linkup`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for LinkupProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkupProviderSettings")
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

/// Create a Linkup provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_linkup(settings: LinkupProviderSettings) -> Result<LinkupProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(LinkupProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Linkup"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_linkup` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn linkup() -> &'static LinkupProvider {
    static DEFAULT: OnceLock<LinkupProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_linkup(LinkupProviderSettings::default())
            .expect("default Linkup settings are always valid")
    })
}

/// A Linkup provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct LinkupProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl LinkupProvider {
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
    pub fn search_model(&self) -> LinkupSearchModel {
        LinkupSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(LinkupProvider, search_model, |p, _id| p.search_model());

/// Build the Linkup `/v1/search` request body (pure function).
fn build_request_body(options: &SearchCallOptions, linkup_options: &LinkupOptions) -> Value {
    let mut body = json!({
        "q": options.query,
        "depth": linkup_options
            .depth
            .clone()
            .unwrap_or_else(|| "standard".to_string()),
        "outputType": linkup_options
            .output_type
            .clone()
            .unwrap_or_else(|| "searchResults".to_string()),
    });
    if let Some(include) = &options.include_domains {
        body["includeDomains"] = json!(include);
    }
    if let Some(exclude) = &options.exclude_domains {
        body["excludeDomains"] = json!(exclude);
    }
    body
}

/// A single Linkup result entry (`{name, url, content}`). All fields are
/// optional so unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct LinkupResult {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

/// The response from the Linkup `/v1/search` endpoint.
///
/// Covers both output types: `searchResults` (`results`) and `sourcedAnswer`
/// (`answer` + `sources`). Fields not present for a given output type are
/// `None` by default.
#[derive(Debug, Deserialize)]
struct LinkupResponse {
    #[serde(default)]
    results: Option<Vec<LinkupResult>>,
    #[serde(default)]
    answer: Option<String>,
    #[serde(default)]
    sources: Option<Vec<LinkupResult>>,
}

fn map_results(entries: Vec<LinkupResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.name,
            url: r.url,
            content: r.content,
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// A Linkup search model.
pub struct LinkupSearchModel {
    config: EndpointConfig,
}

impl LinkupSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for LinkupSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let linkup_options = parse_linkup_options(options.provider_options.as_ref());
        let body = build_request_body(options, &linkup_options);

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/v1/search"), options),
            body,
            aimux_provider_utils::create_json_response_handler::<LinkupResponse>(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        // Prefer `results` (searchResults); fall back to `sources`
        // (sourcedAnswer), which also carries an `answer`.
        let (results, answer) = if let Some(results) = data.results {
            (map_results(results), None)
        } else {
            let sources = data.sources.unwrap_or_default();
            (map_results(sources), data.answer)
        };

        Ok(SearchResult {
            results,
            answer,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
