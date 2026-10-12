//! The `RerankingModel` trait — the provider-facing interface for reranking.
//!
//! Aligned with Vercel AI SDK `RerankingModelV4`
//! (`reference/ai/packages/provider/src/reranking-model/v4/`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::shared::{SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Warning};
use crate::{AbortSignal, retry, timeout};

/// Documents to rerank: either a list of texts or a list of JSON objects.
///
/// Aligned with V4 `RerankingModelV4CallOptions.documents`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
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
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RerankingCallOptions {
    /// Documents to rerank.
    pub documents: RerankingDocuments,

    /// The query to rerank the documents against.
    pub query: String,

    /// Optional limit: return only the top `n` documents.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u32>,

    /// Abort signal for cancelling the operation.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// Per-call retry override. `None` uses the model default.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,

    /// Per-call operation timeout.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<crate::options::TimeoutConfiguration>,

    /// Additional provider-specific options, keyed by provider name.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,

    /// Additional HTTP headers to send with the request.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
            max_retries: None,
            timeout: None,
            provider_options: None,
            headers: None,
        }
    }
}

/// A single reranked entry: the original index and its relevance score.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RerankingResult {
    /// Ordered list of reranked documents, sorted by descending relevance
    /// score. Each entry's `index` refers to the position in the original
    /// `documents` list.
    pub ranking: Vec<RerankingRank>,

    /// Additional provider-specific metadata, keyed by provider name.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Warnings for the call, e.g. unsupported settings.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<Warning>>,

    /// Optional response information for debugging.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<RerankingResponse>,
}

/// Optional response information for a reranking call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RerankingResponse {
    /// ID for the generated response, if the provider sends one.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Timestamp for the start of the generated response (ISO 8601 string).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Response headers.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
    /// Response body (opaque JSON).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// The unified reranking model trait (provider-facing).
///
/// Aligned with V4 `RerankingModelV4`.
#[async_trait]
pub trait RerankingModel: Send + Sync {
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

/// User-facing reranking with Core-owned retry and timeout.
///
/// # Errors
///
/// Returns the provider failure, retry exhaustion, timeout, or caller abort.
pub async fn rerank(
    model: &dyn RerankingModel,
    options: RerankingCallOptions,
) -> Result<RerankingResult, AiMuxError> {
    let timeout = timeout::OperationTimeout::new(options.timeout.unwrap_or_default())?;
    let abort_signal = options.abort_signal.clone();
    let retries = retry::prepare_retries(options.max_retries, abort_signal.clone());
    timeout::run(
        retries.retry(|| model.do_rerank(&options)),
        abort_signal.as_ref(),
        timeout,
    )
    .await
}
