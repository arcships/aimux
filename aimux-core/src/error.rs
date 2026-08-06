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

    /// Rate-limited (HTTP 429). Carries both the retry hint and the provider's
    /// response message (e.g. "quota exceeded" vs "too many requests") so
    /// callers can tell the two apart.
    #[error("rate limited: {message} (retry after {retry_after_ms}ms)")]
    RateLimited {
        #[ts(type = "number")]
        retry_after_ms: u64,
        message: String,
    },

    #[error("authentication failed: {0}")]
    Auth(String),

    /// The access token has expired (or was invalidated) and must be refreshed
    /// by the caller. RFC-0018 subscription mode: the library maps a 401 from
    /// the Codex subscription endpoint to this variant; the integrator
    /// orchestrates `codex_refresh` + retry.
    #[error("token expired: {0}")]
    TokenExpired(String),

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("unsupported functionality: {0}")]
    Unsupported(String),

    #[error("no such model: {0}")]
    NoSuchModel(String),

    #[error("unknown provider: {0}")]
    UnknownProvider(String),

    #[error("API call error: {0}")]
    ApiCall(String),

    #[error("request timed out: {0}")]
    Timeout(String),

    #[error("request aborted")]
    Aborted,

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
            AiMuxError::RateLimited { .. }
                | AiMuxError::Http(_)
                | AiMuxError::ApiCall(_)
                | AiMuxError::Timeout(_)
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
            AiMuxError::RateLimited { retry_after_ms, .. } => Some(*retry_after_ms as i64),
            _ => None,
        }
    }

    /// Returns the error variant name as a string (e.g. `"Auth"`, `"Provider"`,
    /// `"RateLimited"`). Used by FFI bindings to give callers a machine-readable
    /// error type alongside the human-readable message.
    pub fn error_type(&self) -> &'static str {
        match self {
            AiMuxError::Provider(_) => "Provider",
            AiMuxError::Http(_) => "Http",
            AiMuxError::Json(_) => "Json",
            AiMuxError::Stream(_) => "Stream",
            AiMuxError::Tool(_) => "Tool",
            AiMuxError::InvalidArgument(_) => "InvalidArgument",
            AiMuxError::InvalidPrompt(_) => "InvalidPrompt",
            AiMuxError::RateLimited { .. } => "RateLimited",
            AiMuxError::Auth(_) => "Auth",
            AiMuxError::TokenExpired(_) => "TokenExpired",
            AiMuxError::ModelNotFound(_) => "ModelNotFound",
            AiMuxError::Unsupported(_) => "Unsupported",
            AiMuxError::NoSuchModel(_) => "NoSuchModel",
            AiMuxError::UnknownProvider(_) => "UnknownProvider",
            AiMuxError::ApiCall(_) => "ApiCall",
            AiMuxError::Timeout(_) => "Timeout",
            AiMuxError::Aborted => "Aborted",
            AiMuxError::Other(_) => "Other",
        }
    }

    /// Returns the HTTP status code carried by this error, if any.
    ///
    /// Status codes are now derived structurally per variant where the variant
    /// implies a single status (`Auth`/`TokenExpired` → 401,
    /// `RateLimited` → 429, `ModelNotFound` → 404). Status-bearing variants
    /// created by the HTTP layer (`Http`/`Provider`/`ApiCall`) embed the code
    /// in the message as `"HTTP {status}: ..."` (that prefix is stamped by
    /// `parse_provider_error` and `send_with_retry_raw`); this method extracts
    /// it as a fallback. Errors that don't originate from an HTTP response
    /// return `None`.
    pub fn status_code(&self) -> Option<u16> {
        match self {
            AiMuxError::Auth(_) | AiMuxError::TokenExpired(_) => Some(401),
            AiMuxError::RateLimited { .. } => Some(429),
            AiMuxError::ModelNotFound(_) => Some(404),
            AiMuxError::Http(m) | AiMuxError::Provider(m) | AiMuxError::ApiCall(m) => {
                // Parse "HTTP 403: ..." → 403 (only the HTTP layer stamps this
                // prefix, so the match is unambiguous).
                let rest = m.strip_prefix("HTTP ")?;
                let code_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                code_str.parse().ok()
            }
            _ => None,
        }
    }
}
