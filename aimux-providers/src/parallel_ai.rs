//! Parallel AI provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Parallel AI search API
//! (`POST https://api.parallel.ai/v1/search`).
//!
//! Parallel AI is a modality-specific provider: it exposes a native web
//! search protocol (objective + search queries → results) and does not
//! support language models. The single `query` is mapped to the
//! `search_queries` array (and used as the `objective`).
//!
//! [`create_parallel_ai`] takes [`ParallelAiProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`ParallelAiProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `PARALLEL_API_KEY`.
//! [`parallel_ai()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, credential_headers};

/// Fixed model ID for the Parallel AI search model.
const MODEL_ID: &str = "parallel-search";

const DEFAULT_BASE_URL: &str = "https://api.parallel.ai";
const API_KEY_ENV_VAR: &str = "PARALLEL_API_KEY";
const DEFAULT_NAME: &str = "parallel_ai";

/// Settings of [`create_parallel_ai`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct ParallelAiProviderSettings {
    /// Base URL for the API calls. Default `https://api.parallel.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `PARALLEL_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.search"`).
    /// Default `"parallel_ai"`. The providerOptions key stays `parallel_ai`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for ParallelAiProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelAiProviderSettings")
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

/// Create a Parallel AI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_parallel_ai(
    settings: ParallelAiProviderSettings,
) -> Result<ParallelAiProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(ParallelAiProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: credential_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Parallel AI"),
            AuthScheme::Header("x-api-key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_parallel_ai` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn parallel_ai() -> &'static ParallelAiProvider {
    static DEFAULT: OnceLock<ParallelAiProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_parallel_ai(ParallelAiProviderSettings::default())
            .expect("default Parallel AI settings are always valid")
    })
}

/// A Parallel AI provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct ParallelAiProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl ParallelAiProvider {
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
    pub fn search_model(&self) -> ParallelAiSearchModel {
        ParallelAiSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(ParallelAiProvider, search_model, |p, _id| p.search_model());

/// Build the Parallel AI `/v1/search` request body (pure function).
///
/// The single `query` is used both as the `objective` and as the sole entry
/// in the `search_queries` array.
fn build_request_body(options: &SearchCallOptions) -> Value {
    json!({
        "objective": options.query,
        "search_queries": [options.query],
        "mode": "advanced",
    })
}

/// A single Parallel AI result entry. `excerpts` is a list of snippet
/// strings; all fields are optional so unknown-but-legal values degrade
/// safely.
#[derive(Debug, Deserialize)]
struct ParallelAiResult {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    excerpts: Vec<String>,
}

/// The response from the Parallel AI `/v1/search` endpoint.
#[derive(Debug, Deserialize)]
struct ParallelAiResponse {
    #[serde(default)]
    results: Vec<ParallelAiResult>,
}

fn map_results(entries: Vec<ParallelAiResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            // Join excerpt snippets into a single content string.
            content: if r.excerpts.is_empty() {
                None
            } else {
                Some(r.excerpts.join("\n"))
            },
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// A Parallel AI search model.
pub struct ParallelAiSearchModel {
    config: EndpointConfig,
}

impl ParallelAiSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for ParallelAiSearchModel {
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
            exchange.request(exchange.url("/v1/search"), options),
            body,
            aimux_provider_utils::create_json_response_handler::<ParallelAiResponse>(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
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
