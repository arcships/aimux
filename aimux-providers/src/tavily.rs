//! Tavily provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Tavily search API
//! (`POST https://api.tavily.com/search`). Bearer auth via `TAVILY_API_KEY`.
//!
//! [`create_tavily`] takes [`TavilyProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`TavilyProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `TAVILY_API_KEY`.
//! [`tavily()`] is the default instance; it reads nothing and cannot fail.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};
use aimux_provider_utils::AuthScheme;

use crate::shared::{EndpointConfig, define_vendor_provider, post_json_search};

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

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.tavily.com";
pub(crate) const API_KEY_ENV_VAR: &str = "TAVILY_API_KEY";
const DEFAULT_NAME: &str = "tavily";

define_vendor_provider! {
    settings: TavilyProviderSettings,
    provider: TavilyProvider,
    create: create_tavily,
    singleton: tavily,
    display: "Tavily",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl TavilyProvider {
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
        post_json_search(
            &self.config,
            "/search",
            options,
            body,
            tavily_failed_response_handler(),
            |parsed: TavilyResponse| (map_results(parsed.results), parsed.answer),
        )
        .await
    }
}
