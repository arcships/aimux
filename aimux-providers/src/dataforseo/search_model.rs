use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};

use crate::shared::{EndpointConfig, post_json_search};

/// Fixed model id for the DataForSEO search model.
const MODEL_ID: &str = "dataforseo-search";

/// Default result depth when `max_results` is unset.
const DEFAULT_DEPTH: u32 = 10;

/// Fixed `max_credits` budget per request.
const MAX_CREDITS: u32 = 1;

/// DataForSEO error response structure: `{ status_code, status_message }`.
fn dataforseo_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("status_message")
                .and_then(Value::as_str)
                .unwrap_or("DataForSEO request failed")
                .to_string(),
            provider_code: data
                .get("status_code")
                .and_then(crate::shared::json_error_code),
        }
    })
}

/// Resolve the `depth` request field from the call options.
///
/// Pure function: maps `max_results` to the DataForSEO `depth` field (the
/// number of organic results to return), defaulting to [`DEFAULT_DEPTH`] when
/// unset.
fn resolve_depth(max_results: Option<u32>) -> u32 {
    max_results.unwrap_or(DEFAULT_DEPTH)
}

/// Build the DataForSEO request body (a JSON array of one task object).
///
/// Pure function. The body shape is:
/// `[{"keyword": <query>, "max_credits": 1, "depth": <depth>}]`.
fn build_request_body(query: &str, depth: u32) -> Value {
    json!([{ "keyword": query, "max_credits": MAX_CREDITS, "depth": depth }])
}

/// The response from the DataForSEO live/advanced endpoint.
///
/// The result list is nested as `tasks[].result[].organic[]`; only the fields
/// used by the trait are deserialized, and extra fields are ignored so
/// unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct DataforseoResponse {
    #[serde(default)]
    tasks: Vec<DataforseoTask>,
}

#[derive(Debug, Deserialize)]
struct DataforseoTask {
    #[serde(default)]
    result: Vec<DataforseoTaskResult>,
}

#[derive(Debug, Deserialize)]
struct DataforseoTaskResult {
    #[serde(default)]
    organic: Vec<DataforseoOrganic>,
}

#[derive(Debug, Deserialize)]
struct DataforseoOrganic {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Map a DataForSEO organic result into a [`SearchResultItem`].
///
/// Field mapping: `title` → `title`, `url` → `url`, `description` → `content`.
fn map_result(r: DataforseoOrganic) -> SearchResultItem {
    SearchResultItem {
        title: r.title,
        url: r.url,
        content: r.description,
        raw_content: None,
        score: None,
        provider_metadata: None,
    }
}

/// A DataForSEO search model.
pub struct DataforseoSearchModel {
    config: EndpointConfig,
}

impl DataforseoSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for DataforseoSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let depth = resolve_depth(options.max_results);
        let body = build_request_body(&options.query, depth);
        post_json_search(
            &self.config,
            "/v3/serp/google/organic/live/advanced",
            options,
            body,
            dataforseo_failed_response_handler(),
            |data: DataforseoResponse| {
                // Flatten tasks[].result[].organic[] preserving provider order.
                let results = data
                    .tasks
                    .into_iter()
                    .flat_map(|t| t.result)
                    .flat_map(|r| r.organic)
                    .map(map_result)
                    .collect();
                (results, None)
            },
        )
        .await
    }
}
