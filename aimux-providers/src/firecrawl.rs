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
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, without_trailing_slash,
};

const MODEL_ID: &str = "firecrawl-search";

/// Firecrawl-specific error structure: `{ "success": false, "error": "..." }`.
const FIRECRAWL_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error"],
    type_path: &[],
};

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

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

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
    pub fn new(config: FirecrawlConfig) -> Self {
        Self { config }
    }

    pub fn search_model(&self) -> FirecrawlSearchModel {
        FirecrawlSearchModel::new(self.config.clone())
    }
}

impl Provider for FirecrawlProvider {
    fn name(&self) -> &str {
        "firecrawl"
    }

    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported(
            "firecrawl does not support language models. Use search_model() instead.".to_string(),
        ))
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

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers,
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &FIRECRAWL_ERROR_STRUCTURE,
        )
        .await?;
        let response_headers = resp.headers;

        let parsed: FirecrawlResponse = serde_json::from_slice(&resp.body).map_err(|e| {
            AiMuxError::Provider(format!("Failed to parse Firecrawl response: {e}"))
        })?;

        Ok(SearchResult {
            results: map_results(parsed.data.web),
            answer: None,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: Some(serde_json::from_slice(&resp.body).unwrap_or(Value::Null)),
            }),
        })
    }
}
