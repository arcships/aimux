//! Error types for aimux-core.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// Unified error type for all aimux operations.
#[derive(Debug, Clone, Serialize, Deserialize, TS, Error)]
#[ts(export)]
pub enum AiMuxError {
    #[error("provider error: {0}")]
    Provider(String),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("JSON error: {0}")]
    Json(String),

    #[error("stream error: {0}")]
    Stream(String),

    #[error("tool error: {0}")]
    Tool(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("invalid prompt: {0}")]
    InvalidPrompt(String),

    #[error("rate limited: retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("unsupported functionality: {0}")]
    Unsupported(String),

    #[error("no such model: {0}")]
    NoSuchModel(String),

    #[error("API call error: {0}")]
    ApiCall(String),

    #[error("{0}")]
    Other(String),
}

impl From<serde_json::Error> for AiMuxError {
    fn from(e: serde_json::Error) -> Self {
        AiMuxError::Json(e.to_string())
    }
}

impl AiMuxError {
    /// Returns `true` if the error is transient and can be retried.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            AiMuxError::RateLimited { .. } | AiMuxError::Http(_) | AiMuxError::ApiCall(_)
        )
    }

    /// Returns the retry-after delay hint (in milliseconds) carried by this
    /// error, if any. Currently only `RateLimited` carries a hint (from a
    /// `retry-after-ms` / `retry-after` response header). Returns `None` for
    /// errors that don't advertise a delay, in which case the retry strategy
    /// falls back to exponential backoff.
    ///
    /// Mirrors the header-consulting behaviour of
    /// `retryWithExponentialBackoffRespectingRetryHeaders` in the TS SDK.
    pub fn retry_after_hint(&self) -> Option<i64> {
        match self {
            AiMuxError::RateLimited { retry_after_ms } => Some(*retry_after_ms as i64),
            _ => None,
        }
    }
}
