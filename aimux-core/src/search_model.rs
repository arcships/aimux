//! The `SearchModel` trait — the provider-facing interface for web search.
//!
//! Designed per [RFC-0007](../../rfc/0007-search-model-trait.md). Covers the
//! common `query → results[]` pattern shared by search providers (Tavily,
//! Serper, Exa, etc.). Provider-specific fields are passed via
//! `provider_options`; the core trait only models the shared structure.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::AiMuxError;
use crate::shared::{SharedHeaders, SharedProviderMetadata, SharedProviderOptions};
use crate::types::Warning;
use crate::{AbortSignal, retry, timeout};

/// Options passed to [`SearchModel::do_search`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchCallOptions {
    /// The search query string.
    pub query: String,

    /// Maximum number of results to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,

    /// Whether to include raw page content in results.
    ///
    /// Provider support varies; providers that cannot honor this should
    /// issue a warning rather than erroring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_raw_content: Option<bool>,

    /// Optional time range filter (e.g. `"day"`, `"week"`, `"month"`,
    /// `"year"`). Provider support varies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range: Option<String>,

    /// Optional list of domains to include in results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_domains: Option<Vec<String>>,

    /// Optional list of domains to exclude from results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_domains: Option<Vec<String>>,

    /// Abort signal for cancelling the operation.
    #[serde(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// Maximum number of retries, the AI SDK's `maxRetries`; `Some(0)` disables
    /// retrying. `None` uses 2, the AI SDK default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,

    /// Per-call operation timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<crate::options::TimeoutConfiguration>,

    /// Additional provider-specific options, keyed by provider name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,

    /// Additional HTTP headers to send with the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
}

impl SearchCallOptions {
    /// Create options with a query and all else unset.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            max_results: None,
            include_raw_content: None,
            time_range: None,
            include_domains: None,
            exclude_domains: None,
            abort_signal: None,
            max_retries: None,
            timeout: None,
            provider_options: None,
            headers: None,
        }
    }
}

/// A single search result item.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultItem {
    /// The title of the result (e.g. page title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// The URL of the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// A snippet/summary of the result content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// Raw page content (when `include_raw_content` is requested and
    /// supported by the provider).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_content: Option<String>,

    /// A relevance score (0.0–1.0) if the provider returns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,

    /// Provider-specific metadata for this result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<SharedProviderMetadata>,
}

/// The result of [`SearchModel::do_search`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Ordered list of search results.
    pub results: Vec<SearchResultItem>,

    /// An optional direct answer / summary (some providers return
    /// an AI-generated answer alongside results).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,

    /// Additional provider-specific metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Warnings for the call.
    pub warnings: Vec<Warning>,

    /// Optional response information for debugging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<SearchResponse>,
}

/// Optional response information for a search call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    /// Response headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
    /// The response body (opaque JSON).
    #[serde(
        default,
        deserialize_with = "crate::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("x-aimux-codec" = "presence"))]
    pub body: Option<serde_json::Value>,
}

/// The unified search model trait (provider-facing).
///
/// Designed per [RFC-0007](../../rfc/0007-search-model-trait.md). Providers
/// implement `do_search`; users never call it directly.
#[async_trait]
pub trait SearchModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"tavily"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID (some providers use fixed IDs like
    /// `"tavily-search"`; others accept endpoint-specific names).
    fn model_id(&self) -> &str;

    /// Execute a search query and return results.
    ///
    /// Naming: the `do_` prefix prevents accidental direct usage by users.
    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError>;
}

/// User-facing search with Core-owned retry and timeout.
///
/// # Errors
///
/// Returns the provider failure, retry exhaustion, timeout, or caller abort.
pub async fn search(
    model: &dyn SearchModel,
    options: SearchCallOptions,
) -> Result<SearchResult, AiMuxError> {
    let timeout = timeout::OperationTimeout::new(options.timeout.unwrap_or_default())?;
    let abort_signal = options.abort_signal.clone();
    let retries = retry::prepare_retries(options.max_retries, abort_signal.clone());
    timeout::run(
        retries.retry(|| model.do_search(&options)),
        abort_signal.as_ref(),
        timeout,
    )
    .await
}
