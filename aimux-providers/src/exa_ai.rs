//! Exa AI provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Exa search API
//! (`POST https://api.exa.ai/search`). Uses `x-api-key` header auth
//! via `EXA_API_KEY`.

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
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, without_trailing_slash,
};

const MODEL_ID: &str = "exa-search";

/// Configuration for the Exa AI provider.
#[derive(Debug, Clone)]
pub struct ExaAiConfig {
    pub api_key: String,
    pub base_url: String,
}

impl ExaAiConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.exa.ai".to_string(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "EXA_API_KEY", "Exa AI")?;
        Ok(Self::new(api_key))
    }
}

/// Exa AI provider — search-only.
pub struct ExaAiProvider {
    config: ExaAiConfig,
}

impl ExaAiProvider {
    pub fn new(config: ExaAiConfig) -> Self {
        Self { config }
    }

    pub fn search_model(&self) -> ExaAiSearchModel {
        ExaAiSearchModel::new(self.config.clone())
    }
}

impl Provider for ExaAiProvider {
    fn name(&self) -> &str {
        "exa_ai"
    }

    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported(
            "exa_ai does not support language models. Use search_model() instead.".to_string(),
        ))
    }
}

fn build_request_body(options: &SearchCallOptions) -> Value {
    let mut body = json!({
        "query": options.query,
    });
    if let Some(max) = options.max_results {
        body["numResults"] = json!(max);
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
struct ExaResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExaResponse {
    #[serde(default)]
    results: Vec<ExaResult>,
}

fn map_results(entries: Vec<ExaResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            content: r.text,
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// Exa AI search model — implements `SearchModel`.
pub struct ExaAiSearchModel {
    config: ExaAiConfig,
}

impl ExaAiSearchModel {
    pub fn new(config: ExaAiConfig) -> Self {
        Self { config }
    }

    fn endpoint(&self) -> String {
        format!("{}/search", self.config.base_url)
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("x-api-key".to_string(), self.config.api_key.clone()),
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
impl SearchModel for ExaAiSearchModel {
    fn provider(&self) -> &str {
        "exa_ai"
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
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;
        let response_headers = resp.headers;

        let parsed: ExaResponse = serde_json::from_slice(&resp.body)
            .map_err(|e| AiMuxError::Provider(format!("Failed to parse Exa response: {e}")))?;

        Ok(SearchResult {
            results: map_results(parsed.results),
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
