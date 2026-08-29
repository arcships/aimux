//! Firecrawl provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Firecrawl search API
//! (`POST https://api.firecrawl.dev/v2/search`). Bearer auth via
//! `FIRECRAWL_API_KEY`.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{HttpRequest, load_api_key, without_trailing_slash};

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

/// Configuration for the Firecrawl provider.
#[derive(Debug, Clone)]
pub struct FirecrawlConfig {
    pub api_key: String,
    pub base_url: String,
}

impl FirecrawlConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.firecrawl.dev".to_string(),
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `FIRECRAWL_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "FIRECRAWL_API_KEY", "Firecrawl")?;
        Ok(Self::new(api_key))
    }
}

/// Firecrawl provider — search-only.
pub struct FirecrawlProvider {
    config: FirecrawlConfig,
}

impl FirecrawlProvider {
    #[must_use]
    pub fn new(config: FirecrawlConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn search_model(&self) -> FirecrawlSearchModel {
        FirecrawlSearchModel::new(self.config.clone())
    }
}

impl Provider for FirecrawlProvider {
    fn name(&self) -> &str {
        "firecrawl"
    }
}

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

/// Firecrawl search model — implements `SearchModel`.
pub struct FirecrawlSearchModel {
    config: FirecrawlConfig,
}

impl FirecrawlSearchModel {
    #[must_use]
    pub fn new(config: FirecrawlConfig) -> Self {
        Self { config }
    }

    fn endpoint(&self) -> String {
        format!("{}/v2/search", self.config.base_url)
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.config.api_key),
            ),
        ];
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.push((k.clone(), v.clone()));
            }
        }
        headers
    }
}

#[async_trait]
impl SearchModel for FirecrawlSearchModel {
    fn provider(&self) -> &str {
        "firecrawl"
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let body = build_request_body(options);
        let headers = self.build_headers(options.headers.as_ref());

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            body,
            aimux_provider_utils::create_json_response_handler(),
            firecrawl_failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers.unwrap_or_default();
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
