//! Serper provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Serper search API
//! (`POST https://google.serper.dev/search`). Uses `X-API-KEY` header auth
//! via `SERPER_API_KEY`.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, without_trailing_slash,
};

const MODEL_ID: &str = "serper-search";

/// Configuration for the Serper provider.
#[derive(Debug, Clone)]
pub struct SerperConfig {
    pub api_key: String,
    pub base_url: String,
}

impl SerperConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://google.serper.dev".to_string(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "SERPER_API_KEY", "Serper")?;
        Ok(Self::new(api_key))
    }
}

/// Serper provider — search-only.
pub struct SerperProvider {
    config: SerperConfig,
}

impl SerperProvider {
    pub fn new(config: SerperConfig) -> Self {
        Self { config }
    }

    pub fn search_model(&self) -> SerperSearchModel {
        SerperSearchModel::new(self.config.clone())
    }
}

impl Provider for SerperProvider {
    fn name(&self) -> &str {
        "serper"
    }
}

fn build_request_body(options: &SearchCallOptions) -> Value {
    let mut body = json!({
        "q": options.query,
    });
    if let Some(max) = options.max_results {
        body["num"] = json!(max);
    }
    body
}

#[derive(Debug, Deserialize)]
struct SerperOrganicResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerperResponse {
    #[serde(default)]
    organic: Vec<SerperOrganicResult>,
}

fn map_results(entries: Vec<SerperOrganicResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.link,
            content: r.snippet,
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// Serper search model — implements `SearchModel`.
pub struct SerperSearchModel {
    config: SerperConfig,
}

impl SerperSearchModel {
    pub fn new(config: SerperConfig) -> Self {
        Self { config }
    }

    fn endpoint(&self) -> String {
        format!("{}/search", self.config.base_url)
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-API-KEY".to_string(), self.config.api_key.clone()),
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
impl SearchModel for SerperSearchModel {
    fn provider(&self) -> &str {
        "serper"
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
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;
        let response_headers = resp.headers;

        let parsed: SerperResponse = serde_json::from_slice(&resp.body)
            .map_err(|e| AiMuxError::Provider(format!("Failed to parse Serper response: {e}")))?;

        Ok(SearchResult {
            results: map_results(parsed.organic),
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
