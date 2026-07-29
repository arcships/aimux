//! The `SearchModel` trait — the provider-facing interface for web search.
//!
//! Designed per [RFC-0007](../../rfc/0007-search-model-trait.md). Covers the
//! common `query → results[]` pattern shared by search providers (Tavily,
//! Serper, Exa, etc.). Provider-specific fields are passed via
//! `provider_options`; the core trait only models the shared structure.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::shared::{AbortSignal, SharedHeaders, SharedProviderMetadata, SharedProviderOptions};
use crate::types::Warning;

/// Options passed to [`SearchModel::do_search`].
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchCallOptions {
    /// The search query string.
    pub query: String,

    /// Maximum number of results to return.
    pub max_results: Option<u32>,

    /// Whether to include raw page content in results.
    ///
    /// Provider support varies; providers that cannot honor this should
    /// issue a warning rather than erroring.
    pub include_raw_content: Option<bool>,

    /// Optional time range filter (e.g. `"day"`, `"week"`, `"month"`,
    /// `"year"`). Provider support varies.
    pub time_range: Option<String>,

    /// Optional list of domains to include in results.
    pub include_domains: Option<Vec<String>>,

    /// Optional list of domains to exclude from results.
    pub exclude_domains: Option<Vec<String>>,

    /// Abort signal for cancelling the operation.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: Option<SharedProviderOptions>,

    /// Additional HTTP headers to send with the request.
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
            provider_options: None,
            headers: None,
        }
    }
}

/// A single search result item.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchResultItem {
    /// The title of the result (e.g. page title).
    pub title: Option<String>,

    /// The URL of the result.
    pub url: Option<String>,

    /// A snippet/summary of the result content.
    pub content: Option<String>,

    /// Raw page content (when `include_raw_content` is requested and
    /// supported by the provider).
    pub raw_content: Option<String>,

    /// A relevance score (0.0–1.0) if the provider returns one.
    pub score: Option<f64>,

    /// Provider-specific metadata for this result.
    pub provider_metadata: Option<SharedProviderMetadata>,
}

/// The result of [`SearchModel::do_search`].
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchResult {
    /// Ordered list of search results.
    pub results: Vec<SearchResultItem>,

    /// An optional direct answer / summary (some providers return
    /// an AI-generated answer alongside results).
    pub answer: Option<String>,

    /// Additional provider-specific metadata.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Warnings for the call.
    pub warnings: Vec<Warning>,

    /// Optional response information for debugging.
    pub response: Option<SearchResponse>,
}

/// Optional response information for a search call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchResponse {
    /// Response headers.
    pub headers: Option<SharedHeaders>,
    /// The response body (opaque JSON).
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
