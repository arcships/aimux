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
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{HttpRequest, load_api_key, without_trailing_slash};

fn exa_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Exa request failed")
                .to_string(),
            provider_code: error
                .and_then(|value| value.get("code").or_else(|| value.get("type")))
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    })
}

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

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `EXA_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
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
    #[must_use]
    pub fn new(config: ExaAiConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn search_model(&self) -> ExaAiSearchModel {
        ExaAiSearchModel::new(self.config.clone())
    }
}

impl Provider for ExaAiProvider {
    fn name(&self) -> &str {
        "exa_ai"
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
    #[must_use]
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

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            body,
            aimux_provider_utils::create_json_response_handler(),
            exa_failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers.unwrap_or_default();
        let response_body = resp.raw_value;
        let parsed: ExaResponse = resp.value;

        Ok(SearchResult {
            results: map_results(parsed.results),
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
