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

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};
use aimux_provider_utils::AuthScheme;

use crate::shared::{EndpointConfig, define_vendor_provider, post_json_search};

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

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.firecrawl.dev";
pub(crate) const API_KEY_ENV_VAR: &str = "FIRECRAWL_API_KEY";
const DEFAULT_NAME: &str = "firecrawl";

define_vendor_provider! {
    settings: FirecrawlProviderSettings,
    provider: FirecrawlProvider,
    create: create_firecrawl,
    singleton: firecrawl,
    display: "Firecrawl",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl FirecrawlProvider {
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
        post_json_search(
            &self.config,
            "/v2/search",
            options,
            body,
            firecrawl_failed_response_handler(),
            |parsed: FirecrawlResponse| (map_results(parsed.data.web), None),
        )
        .await
    }
}
