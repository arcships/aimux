//! The `RerankingModel` trait — the provider-facing interface for reranking.
//!
//! Aligned with Vercel AI SDK `RerankingModelV4`
//! (`reference/ai/packages/provider/src/reranking-model/v4/`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::shared::{
    AbortSignal, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Warning,
};

/// Documents to rerank: either a list of texts or a list of JSON objects.
///
/// Aligned with V4 `RerankingModelV4CallOptions.documents`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum RerankingDocuments {
    /// A list of plain-text documents.
    Text { values: Vec<String> },
    /// A list of JSON-object documents.
    Object { values: Vec<serde_json::Value> },
}

/// Options passed to [`RerankingModel::do_rerank`].
///
/// Aligned with V4 `RerankingModelV4CallOptions`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RerankingCallOptions {
    /// Documents to rerank.
    pub documents: RerankingDocuments,

    /// The query to rerank the documents against.
    pub query: String,

    /// Optional limit: return only the top `n` documents.
    pub top_n: Option<u32>,

    /// Abort signal for cancelling the operation.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: Option<SharedProviderOptions>,

    /// Additional HTTP headers to send with the request.
    pub headers: Option<SharedHeaders>,
}

impl RerankingCallOptions {
    /// Create options with text documents and a query.
    pub fn new(query: impl Into<String>, documents: RerankingDocuments) -> Self {
        Self {
            documents,
            query: query.into(),
            top_n: None,
            abort_signal: None,
            provider_options: None,
            headers: None,
        }
    }
}

/// A single reranked entry: the original index and its relevance score.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RerankingRank {
    /// The index of the document in the original list (before reranking).
    pub index: u32,
    /// The relevance score of the document after reranking.
    pub relevance_score: f64,
}

/// The result of [`RerankingModel::do_rerank`].
///
/// Aligned with V4 `RerankingModelV4Result`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RerankingResult {
    /// Ordered list of reranked documents, sorted by descending relevance
    /// score. Each entry's `index` refers to the position in the original
    /// `documents` list.
    pub ranking: Vec<RerankingRank>,

    /// Additional provider-specific metadata, keyed by provider name.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Warnings for the call, e.g. unsupported settings.
    pub warnings: Option<Vec<Warning>>,

    /// Optional response information for debugging.
    pub response: Option<RerankingResponse>,
}

/// Optional response information for a reranking call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RerankingResponse {
    /// ID for the generated response, if the provider sends one.
    pub id: Option<String>,
    /// Timestamp for the start of the generated response (ISO 8601 string).
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    pub model_id: Option<String>,
    /// Response headers.
    pub headers: Option<SharedHeaders>,
    /// Response body (opaque JSON).
    pub body: Option<serde_json::Value>,
}

/// The unified reranking model trait (provider-facing).
///
/// Aligned with V4 `RerankingModelV4`.
#[async_trait]
pub trait RerankingModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"cohere"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID, e.g. `"rerank-english-v3.0"`.
    fn model_id(&self) -> &str;

    /// Rerank a list of documents using the query.
    ///
    /// Naming: the `do_` prefix prevents accidental direct usage by users.
    async fn do_rerank(
        &self,
        options: &RerankingCallOptions,
    ) -> Result<RerankingResult, AiMuxError>;
}
