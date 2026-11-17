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

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};
use aimux_provider_utils::AuthScheme;

use crate::shared::{EndpointConfig, define_vendor_provider, post_json_search};

/// Fixed model ID for the Parallel AI search model.
/// Failed-response handler of the Parallel AI endpoint: `message`, and `type`
/// else `code` as the provider code, from the body's `error` object, or from
/// the top level when there is none. This is the mapping the endpoint used
/// with the removed shared handler; it is not an AI SDK schema.
fn parallel_ai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::message_type_error_response_handler()
}

const MODEL_ID: &str = "parallel-search";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.parallel.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "PARALLEL_API_KEY";
const DEFAULT_NAME: &str = "parallel_ai";

define_vendor_provider! {
    settings: ParallelAiProviderSettings,
    provider: ParallelAiProvider,
    create: create_parallel_ai,
    singleton: parallel_ai,
    display: "Parallel AI",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("x-api-key"),
    extra_headers: Vec::new(),
    package: None,
}

impl ParallelAiProvider {
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
        post_json_search(
            &self.config,
            "/v1/search",
            options,
            body,
            parallel_ai_failed_response_handler(),
            |data: ParallelAiResponse| (map_results(data.results), None),
        )
        .await
    }
}
