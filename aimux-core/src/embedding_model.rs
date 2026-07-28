//! The `EmbeddingModel` trait — the provider-facing interface for text embeddings.
//!
//! Aligned with Vercel AI SDK `EmbeddingModelV4`
//! (`reference/ai/packages/provider/src/embedding-model/v4/`).
//!
//! Providers implement [`EmbeddingModel::do_embed`]; users never call it
//! directly.

use async_trait::async_trait;

use crate::error::AiMuxError;
use crate::shared::{
    AbortSignal, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Warning,
};

/// A single embedding vector.
///
/// The TS spec types this as `Array<number>`; we use `f32` because that is the
/// de-facto precision returned by every major embedding provider (OpenAI,
/// Voyage, Cohere, …) and halves the memory footprint of `f64`.
pub type Embedding = Vec<f32>;

/// Options passed to [`EmbeddingModel::do_embed`].
///
/// Aligned with V4 `EmbeddingModelV4CallOptions`.
#[derive(Debug, Clone)]
pub struct EmbeddingCallOptions {
    /// List of text values to generate embeddings for.
    pub values: Vec<String>,

    /// Abort signal for cancelling the operation.
    pub abort_signal: Option<AbortSignal>,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: Option<SharedProviderOptions>,

    /// Additional HTTP headers to send with the request.
    pub headers: Option<SharedHeaders>,
}

impl EmbeddingCallOptions {
    /// Create options for a single value with everything else unset.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            values: vec![value.into()],
            abort_signal: None,
            provider_options: None,
            headers: None,
        }
    }
}

/// Token usage for an embedding call. Embeddings only report input tokens.
#[derive(Debug, Clone, Default)]
pub struct EmbeddingUsage {
    /// Number of input tokens consumed.
    pub tokens: u32,
}

/// The result of [`EmbeddingModel::do_embed`].
///
/// Aligned with V4 `EmbeddingModelV4Result`.
#[derive(Debug, Clone)]
pub struct EmbeddingResult {
    /// Generated embeddings, in the same order as the input `values`.
    pub embeddings: Vec<Embedding>,

    /// Token usage (input tokens only).
    pub usage: Option<EmbeddingUsage>,

    /// Additional provider-specific metadata.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Optional response information for debugging.
    pub response: Option<EmbeddingResponse>,

    /// Warnings for the call, e.g. unsupported settings.
    pub warnings: Vec<Warning>,
}

/// Debugging response info specific to embedding calls.
#[derive(Debug, Clone, Default)]
pub struct EmbeddingResponse {
    /// Response headers.
    pub headers: Option<SharedHeaders>,
    /// The response body (opaque JSON).
    pub body: Option<serde_json::Value>,
}

/// The unified embedding model trait (provider-facing).
///
/// Aligned with V4 `EmbeddingModelV4`. Specific to text embeddings.
///
/// # Implementation notes
///
/// - `do_embed` is **not** user-facing API.
/// - `embeddings` in the result must be in the same order as the input
///   `values`.
/// - Providers that cannot serve a request in a single call because
///   `values.len() > max_embeddings_per_call()` should batch internally only
///   when [`supports_parallel_calls`](Self::supports_parallel_calls) returns
///   `true`; otherwise they should return an error.
#[async_trait]
pub trait EmbeddingModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"openai"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID, e.g. `"text-embedding-3-small"`.
    fn model_id(&self) -> &str;

    /// Limit of how many embeddings can be generated in a single API call.
    ///
    /// `None` means the model has no fixed limit. The TS spec allows this to
    /// be a promise/function; in Rust we resolve it to a plain `Option<u32>`.
    fn max_embeddings_per_call(&self) -> Option<u32>;

    /// `true` if the model can handle multiple embedding calls in parallel.
    fn supports_parallel_calls(&self) -> bool;

    /// Generate a list of embeddings for the given input text.
    ///
    /// Naming: the `do_` prefix prevents accidental direct usage by users.
    async fn do_embed(&self, options: &EmbeddingCallOptions)
    -> Result<EmbeddingResult, AiMuxError>;
}
