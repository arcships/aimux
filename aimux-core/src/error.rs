//! Error types for aimux-core.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// What a failed API call observed — the field set of the AI SDK's
/// `APICallError` (`statusCode` / `responseBody`), which this project's error
/// taxonomy mirrors (docs/internal/vercel-research/07-kernel-infrastructure.md).
///
/// Carried by [`AiMuxError::ApiCall`] alone. There are no per-status
/// variants: a 401, 404 or 429 is an `ApiCall` error whose classification is
/// read from `status_code`, exactly as the AI SDK reads
/// `APICallError.statusCode`. Transport failures (connection reset, DNS,
/// body read) are `ApiCall` errors too — no response arrived, so
/// `status_code` is `None` and `is_retryable` is `true`, exactly as the AI
/// SDK's `handleFetchError` builds an `APICallError` with no `statusCode`
/// and `isRetryable: true`. `message` holds the provider's text **only** —
/// the status is a field, never baked into the string; `Display` composes
/// the human-readable form from the fields at print time, so nothing
/// downstream has to parse it back out.
///
/// `APICallError` fields not carried in this round: `url` /
/// `requestBodyValues` / `responseHeaders` (the request context is not
/// available at the error-construction sites today; all fields are
/// `#[serde(default)]`, so adding them later is not a breaking change).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiCallError {
    /// HTTP status of the response, when it came from one
    /// (`APICallError.statusCode`). Always the *observed* status: the HTTP
    /// layer fills it for every response-derived error; errors built without
    /// an HTTP exchange leave it `None`.
    #[serde(default)]
    pub status_code: Option<u16>,
    /// Provider's machine-readable code (OpenAI's `code`/`type`,
    /// e.g. `"rate_limit_exceeded"` vs `"insufficient_quota"`). Never an HTTP
    /// status. Our normalized take on `APICallError.data`.
    #[serde(default)]
    pub provider_code: Option<String>,
    /// The provider's message, verbatim. No status prefix.
    pub message: String,
    /// The raw response body, verbatim (`APICallError.responseBody`) — the
    /// lossless evidence when `message`/`provider_code` are extractions.
    /// `None` when the error did not come from a response body.
    #[serde(default)]
    pub response_body: Option<String>,
    /// Provider-assigned request id, from the `x-request-id` / `request-id`
    /// response header when one was sent. Filled by the shared HTTP layer.
    #[serde(default)]
    pub request_id: Option<String>,
    /// Retry hint in milliseconds, distilled from the `retry-after-ms` /
    /// `retry-after` response headers on a 429. The classification lives in
    /// `status_code`; this is the matching action input.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub retry_after_ms: Option<u64>,
    /// Whether retrying can help (`APICallError.isRetryable`) — stored at
    /// construction, exactly like the AI SDK: the response path computes it
    /// from the status (408/409/429 or 5xx), and the transport path (no response, so
    /// no status to compute from) sets it to `true` explicitly.
    #[serde(default)]
    pub is_retryable: bool,
}

impl std::fmt::Display for ApiCallError {
    /// `HTTP {status}: {message}` when a status is known (just `HTTP {status}`
    /// if the response had no body), else `{message}`. `provider_code` and
    /// `response_body` are machine-readable only and stay out of the text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.status_code, self.message.is_empty()) {
            (Some(status), false) => write!(f, "HTTP {}: {}", status, self.message),
            (Some(status), true) => write!(f, "HTTP {status}"),
            (None, _) => f.write_str(&self.message),
        }
    }
}

/// Unified error type for all aimux operations.
///
/// Variants are cut by *what the caller does about it*, not by where the
/// failure came from. `ApiCall` carries the [`ApiCallError`] detail inline
/// (unboxed, like async-openai's `ApiError(ApiError)`); the size guard in
/// `error_value_golden_test` keeps the enum under clippy's
/// `result_large_err` threshold.
///
/// Constructed as a named struct literal everywhere
/// (`ApiCallError { status_code: .., ..Default::default() }`), the same
/// shape as the AI SDK's named-options constructor.
#[derive(Debug, Clone, Serialize, Deserialize, TS, Error)]
#[ts(export)]
pub enum AiMuxError {
    #[error("API call error: {0}")]
    ApiCall(ApiCallError),

    #[error("JSON parse error: {0}")]
    JsonParse(String),

    #[error("invalid response data: {0}")]
    InvalidResponseData(String),

    #[error("tool error: {0}")]
    Tool(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("invalid prompt: {0}")]
    InvalidPrompt(String),

    /// The access token has expired (or was invalidated) and must be refreshed
    /// by the caller. RFC-0018 subscription mode: the library maps a 401 from
    /// the Codex subscription endpoint to this variant; the integrator
    /// orchestrates `codex_refresh` + retry.
    ///
    /// This is *not* a status-code avatar: `Auth`-style 401s and token
    /// expiry share the same status, so `status_code` alone cannot express
    /// the difference — the variant carries the extra bit ("refresh helps").
    #[error("token expired: {0}")]
    TokenExpired(String),

    #[error("unsupported functionality: {0}")]
    UnsupportedFunctionality(String),

    /// Registry-level "model id does not resolve" (the AI SDK's
    /// `NoSuchModelError { modelId, modelType }`). Pre-HTTP: the upstream API
    /// was never called, so there is no status.
    #[error("no such model: {model_id}")]
    NoSuchModel {
        model_id: String,
        /// What kind of model was requested (`"languageModel"`,
        /// `"imageModel"`, …), the AI SDK's `modelType`.
        #[serde(default)]
        model_type: String,
    },

    /// Registry-level "provider name does not resolve" (the AI SDK's
    /// `NoSuchProviderError`).
    #[error("No such provider: {provider_id}")]
    NoSuchProvider { provider_id: String },

    #[error("request timed out: {0}")]
    Timeout(String),

    #[error("request aborted")]
    Aborted,

    #[error("{0}")]
    Other(String),
}

/// Canonical serde classification: syntactically broken/truncated JSON is a
/// parse failure; well-formed JSON that violates the expected shape is invalid
/// response data. `Io` has no transport context here, so it stays `JsonParse`;
/// call sites with a concrete transport error should preserve that instead.
impl From<serde_json::Error> for AiMuxError {
    fn from(e: serde_json::Error) -> Self {
        use serde_json::error::Category;
        match e.classify() {
            Category::Data => AiMuxError::InvalidResponseData(e.to_string()),
            Category::Syntax | Category::Eof | Category::Io => AiMuxError::JsonParse(e.to_string()),
        }
    }
}

impl AiMuxError {
    /// Returns `true` if the error is transient and can be retried.
    ///
    /// Reads the stored [`ApiCallError::is_retryable`] field, filled at
    /// construction (the AI SDK's `APICallError.isRetryable`): 408/409/429/5xx on the
    /// response path, `true` on the transport path. `Timeout` is a spent
    /// caller-side time budget — the AI SDK treats it as part of the abort
    /// family (`isAbortError` matches `"TimeoutError"`) and does not retry
    /// it; neither do we.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            AiMuxError::ApiCall(d) => d.is_retryable,
            _ => false,
        }
    }

    /// Returns the retry-after delay hint (in milliseconds) carried by this
    /// error, if any — distilled from the `retry-after-ms` / `retry-after`
    /// response headers into [`ApiCallError::retry_after_ms`]. Returns `None`
    /// for errors that don't advertise a delay, in which case the retry
    /// strategy falls back to exponential backoff.
    ///
    /// Mirrors the header-consulting behaviour of
    /// `retryWithExponentialBackoffRespectingRetryHeaders` in the TS SDK.
    #[must_use]
    pub fn retry_after_hint(&self) -> Option<i64> {
        match self {
            AiMuxError::ApiCall(d) => d.retry_after_ms.map(|ms| ms as i64),
            _ => None,
        }
    }

    /// Returns the HTTP status code carried by this error, if any.
    ///
    /// This is the *observed* status only — the field the HTTP layer filled
    /// when a response actually arrived (`parse_provider_error`,
    /// `send_with_retry_raw`). An error built without an HTTP exchange (a
    /// missing API key, a bare constructor, a transport failure) reports
    /// `None`: no status was seen, so none is invented.
    ///
    /// `TokenExpired` reports `Some(401)` by definition, not by fabrication:
    /// its only producer is the codex subscription channel's observed-401
    /// mapping (RFC-0018), which trades the carried fields for the "refresh
    /// helps" bit — the status is part of the variant's contract.
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            AiMuxError::TokenExpired(_) => Some(401),
            AiMuxError::ApiCall(d) => d.status_code,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare constructor saw no HTTP exchange, so it reports no status —
    /// nothing is invented. Response-derived errors get theirs from the field
    /// (filled by the HTTP layer).
    #[test]
    fn bare_constructors_report_no_status() {
        assert_eq!(
            AiMuxError::ApiCall(ApiCallError {
                message: "no api key".into(),
                ..Default::default()
            })
            .status_code(),
            None
        );
        assert_eq!(
            AiMuxError::ApiCall(ApiCallError {
                message: "reset".into(),
                is_retryable: true,
                ..Default::default()
            })
            .status_code(),
            None
        );
        assert_eq!(
            AiMuxError::ApiCall(ApiCallError {
                message: "boom".into(),
                ..Default::default()
            })
            .status_code(),
            None
        );
    }

    /// `TokenExpired` is definitionally an observed 401 (RFC-0018); the
    /// status is part of the variant's contract, not fabrication.
    #[test]
    fn token_expired_is_a_401_by_contract() {
        assert_eq!(
            AiMuxError::TokenExpired("expired".into()).status_code(),
            Some(401)
        );
        assert!(!AiMuxError::TokenExpired("expired".into()).is_retryable());
    }

    /// H1: the status comes from the field, never from parsing the message.
    #[test]
    fn status_code_reads_the_field_not_the_message() {
        assert_eq!(
            AiMuxError::ApiCall(ApiCallError {
                status_code: Some(403),
                message: "forbidden".into(),
                ..Default::default()
            })
            .status_code(),
            Some(403)
        );
        // A message that merely *looks* like an HTTP prefix carries no status.
        assert_eq!(
            AiMuxError::ApiCall(ApiCallError {
                message: "HTTP 403: forbidden".into(),
                ..Default::default()
            })
            .status_code(),
            None
        );
        assert_eq!(
            AiMuxError::Timeout("HTTP 408: total timeout after 1000ms".into()).status_code(),
            None
        );
        assert_eq!(AiMuxError::JsonParse("HTTP 400".into()).status_code(), None);
        assert_eq!(AiMuxError::Other("HTTP 400".into()).status_code(), None);
    }

    /// A timeout is a spent caller-side time budget, not a transient fault —
    /// the AI SDK files it in the abort family and does not retry it.
    /// Transport failures (no response) are retryable via the stored field.
    #[test]
    fn timeout_is_not_retryable_transport_is() {
        assert!(!AiMuxError::Timeout("total timeout".into()).is_retryable());
        assert!(
            AiMuxError::ApiCall(ApiCallError {
                message: "connection reset".into(),
                is_retryable: true,
                ..Default::default()
            })
            .is_retryable()
        );
        assert!(!AiMuxError::JsonParse("parse".into()).is_retryable());
    }

    /// A 429 is retryable and carries its hint wherever it lives — the
    /// classification is the `status_code` field, not a variant.
    #[test]
    fn rate_limit_is_read_from_the_field() {
        let err = AiMuxError::ApiCall(ApiCallError {
            status_code: Some(429),
            message: "quota exceeded".into(),
            retry_after_ms: Some(7),
            is_retryable: true,
            ..Default::default()
        });
        assert!(err.is_retryable());
        assert_eq!(err.retry_after_hint(), Some(7));
        // A bare ApiCall error (no stored verdict) is not retryable.
        assert!(
            !AiMuxError::ApiCall(ApiCallError {
                message: "boom".into(),
                ..Default::default()
            })
            .is_retryable()
        );

        let json = serde_json::to_string(&err).unwrap();
        let back: AiMuxError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.retry_after_hint(), Some(7));
        assert_eq!(back.status_code(), Some(429));
    }
}
