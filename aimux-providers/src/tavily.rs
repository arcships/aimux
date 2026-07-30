//! Tavily provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Tavily search API
//! (`POST https://api.tavily.com/search`). Bearer auth via `TAVILY_API_KEY`.

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

const MODEL_ID: &str = "tavily-search";

/// Tavily-specific error structure: `{ "detail": { "error": "..." } }`.
const TAVILY_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["detail", "error"],
    type_path: &[],
};

/// Configuration for the Tavily provider.
#[derive(Debug, Clone)]
pub struct TavilyConfig {
    pub api_key: String,
    pub base_url: String,
}

impl TavilyConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.tavily.com".to_string(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "TAVILY_API_KEY", "Tavily")?;
        Ok(Self::new(api_key))
    }
}

/// Tavily provider — search-only.
pub struct TavilyProvider {
    config: TavilyConfig,
}

impl TavilyProvider {
    pub fn new(config: TavilyConfig) -> Self {
        Self { config }
    }

    pub fn search_model(&self) -> TavilySearchModel {
        TavilySearchModel::new(self.config.clone())
    }
}

impl Provider for TavilyProvider {
    fn name(&self) -> &str {
        "tavily"
    }

    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported(
            "tavily does not support language models. Use search_model() instead.".to_string(),
        ))
    }
}

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

/// Tavily search model — implements `SearchModel`.
pub struct TavilySearchModel {
    config: TavilyConfig,
}

impl TavilySearchModel {
    pub fn new(config: TavilyConfig) -> Self {
        Self { config }
    }

    fn endpoint(&self) -> String {
        format!("{}/search", self.config.base_url)
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
impl SearchModel for TavilySearchModel {
    fn provider(&self) -> &str {
        "tavily"
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
            },
            RetryConfig::default(),
            &TAVILY_ERROR_STRUCTURE,
        )
        .await?;
        let response_headers = resp.headers;

        let parsed: TavilyResponse = serde_json::from_slice(&resp.body)
            .map_err(|e| AiMuxError::Provider(format!("Failed to parse Tavily response: {e}")))?;

        Ok(SearchResult {
            results: map_results(parsed.results),
            answer: parsed.answer,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: Some(serde_json::from_slice(&resp.body).unwrap_or(Value::Null)),
            }),
        })
    }
}
