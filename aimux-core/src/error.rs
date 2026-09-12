//! Error types for aimux-core.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::SystemTime;

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
/// and `isRetryable: true`. `message` holds the provider's text verbatim when
/// one is available; transport and parsing failures use a library message
/// with the source detail appended. The HTTP status is always a field, never
/// baked into the string; `Display` composes the human-readable form from the
/// fields at print time, so nothing downstream has to parse it back out.
///
/// Every producer must provide the sanitized request URL and request values.
/// This keeps transport and response failures self-contained and matches the
/// AI SDK's `APICallError` contract.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiCallError {
    /// Sanitized request URL. Required for every API-derived failure.
    pub url: String,
    /// Sanitized request values used to create the API request.
    pub request_body_values: serde_json::Value,
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
    /// Human-readable failure text. Provider text is verbatim when available;
    /// locally detected transport/parse failures include their source detail.
    /// Never includes an HTTP status prefix.
    pub message: String,
    /// The raw response body, verbatim (`APICallError.responseBody`) — the
    /// lossless evidence when `message`/`provider_code` are extractions.
    /// `None` when the error did not come from a response body.
    #[serde(default)]
    pub response_body: Option<String>,
    /// Sanitized response headers.
    #[serde(default)]
    pub response_headers: Option<HashMap<String, String>>,
    /// Parsed provider error data.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    /// Whether retrying can help (`APICallError.isRetryable`) — stored at
    /// construction, exactly like the AI SDK: the response path computes it
    /// from the status (408/409/429 or 5xx), and the transport path (no response, so
    /// no status to compute from) sets it to `true` explicitly.
    #[serde(default)]
    pub is_retryable: bool,
}

impl ApiCallError {
    /// Create an API failure with its required request context.
    pub fn new(
        message: impl Into<String>,
        url: impl Into<String>,
        request_body_values: serde_json::Value,
    ) -> Self {
        Self {
            url: url.into(),
            request_body_values,
            status_code: None,
            provider_code: None,
            message: message.into(),
            response_body: None,
            response_headers: None,
            data: None,
            is_retryable: false,
        }
    }
}

/// Why the retry wrapper stopped retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RetryErrorReason {
    /// Every permitted retry attempt failed with a retryable error.
    MaxRetriesExceeded,
    /// A later attempt produced a non-retryable error.
    ErrorNotRetryable,
}

/// Complete error history for a retried model operation.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RetryError {
    pub reason: RetryErrorReason,
    pub errors: Vec<AiMuxError>,
}

static EMPTY_RETRY_HISTORY_ERROR: LazyLock<AiMuxError> =
    LazyLock::new(|| AiMuxError::Other("Retry failed without recorded errors".into()));

impl RetryError {
    /// The final attempt error, or a stable invalid-history error when a
    /// deserialized or manually constructed value has no attempts.
    #[must_use]
    pub fn last_error(&self) -> &AiMuxError {
        match self.errors.last() {
            Some(error) => error,
            None => &EMPTY_RETRY_HISTORY_ERROR,
        }
    }
}

impl std::fmt::Display for RetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let attempts = self.errors.len();
        let Some(last_error) = self.errors.last() else {
            return f.write_str("Retry failed without recorded errors");
        };
        match self.reason {
            RetryErrorReason::MaxRetriesExceeded => {
                write!(
                    f,
                    "Failed after {attempts} attempts. Last error: {last_error}"
                )
            }
            RetryErrorReason::ErrorNotRetryable => write!(
                f,
                "Failed after {attempts} attempts with non-retryable error: '{last_error}'"
            ),
        }
    }
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
/// failure came from. `ApiCallError` is boxed only to keep the Rust enum
/// compact; serde and every binding still observe the same object shape.
#[derive(Debug, Clone, Serialize, Deserialize, TS, Error)]
#[ts(export)]
pub enum AiMuxError {
    #[error("API call error: {0}")]
    ApiCall(Box<ApiCallError>),

    #[error("{0}")]
    Retry(RetryError),

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

    #[error("{0}")]
    Aborted(String),

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

/// The AI SDK's status-based retryability rule: 408 (timeout), 409
/// (conflict), 429 (rate limit) and every 5xx are worth retrying; everything
/// else is not. One definition — response handlers and providers must not
/// hand-roll this formula.
#[must_use]
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429) || status >= 500
}

impl AiMuxError {
    /// Returns `true` for a malformed individual stream frame that does not
    /// prove the underlying transport has failed. Stream reducers may report
    /// these errors and continue polling for later, independently framed data.
    #[must_use]
    pub fn is_recoverable_stream_error(&self) -> bool {
        matches!(self, Self::JsonParse(_) | Self::InvalidResponseData(_))
    }

    /// Convert caller cancellation into the public error model.
    #[must_use]
    pub fn from_abort_signal(_signal: &crate::AbortSignal) -> Self {
        Self::Aborted("request aborted".into())
    }

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
    /// error's response headers, if any.
    ///
    /// Mirrors the header-consulting behaviour of
    /// `retryWithExponentialBackoffRespectingRetryHeaders` in the TS SDK.
    #[must_use]
    pub fn retry_after_hint(&self) -> Option<i64> {
        let AiMuxError::ApiCall(detail) = self else {
            return None;
        };
        let headers = detail.response_headers.as_ref()?;
        headers
            .get("retry-after-ms")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| !value.is_nan())
            .map(|value| value as i64)
            .or_else(|| {
                headers.get("retry-after").and_then(|value| {
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|value| !value.is_nan())
                        .map(|seconds| (seconds * 1_000.0) as i64)
                        .or_else(|| {
                            httpdate::parse_http_date(value).ok().map(|date| {
                                match date.duration_since(SystemTime::now()) {
                                    Ok(duration) => duration.as_millis() as i64,
                                    Err(error) => -(error.duration().as_millis() as i64),
                                }
                            })
                        })
                })
            })
            // Upstream's `0 <= ms` check: a hint in the past is no hint. It
            // also keeps the value clear of the C ABI's -1 = absent sentinel
            // (`aimux_error_retry_ms`).
            .filter(|milliseconds| *milliseconds >= 0)
    }

    /// Returns the HTTP status code carried by this error, if any.
    ///
    /// This is the *observed* status only — the field the selected response
    /// handler filled when a response actually arrived. An error built without an HTTP exchange (a
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

    fn api_error(message: &str) -> ApiCallError {
        ApiCallError::new(message, "https://example.test", serde_json::json!({}))
    }

    /// A bare constructor saw no HTTP exchange, so it reports no status —
    /// nothing is invented. Response-derived errors get theirs from the field
    /// (filled by the HTTP layer).
    #[test]
    fn bare_constructors_report_no_status() {
        assert_eq!(
            AiMuxError::ApiCall(Box::new(api_error("no api key"))).status_code(),
            None
        );
        assert_eq!(
            AiMuxError::ApiCall(Box::new(ApiCallError {
                is_retryable: true,
                ..api_error("reset")
            }))
            .status_code(),
            None
        );
        assert_eq!(
            AiMuxError::ApiCall(Box::new(api_error("boom"))).status_code(),
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
            AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(403),
                ..api_error("forbidden")
            }))
            .status_code(),
            Some(403)
        );
        // A message that merely *looks* like an HTTP prefix carries no status.
        assert_eq!(
            AiMuxError::ApiCall(Box::new(api_error("HTTP 403: forbidden"))).status_code(),
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
            AiMuxError::ApiCall(Box::new(ApiCallError {
                is_retryable: true,
                ..api_error("connection reset")
            }))
            .is_retryable()
        );
        assert!(!AiMuxError::JsonParse("parse".into()).is_retryable());
    }

    #[test]
    fn empty_retry_history_round_trips_without_panicking() {
        let json = r#"{"reason":"maxRetriesExceeded","errors":[]}"#;
        let error: RetryError = serde_json::from_str(json).unwrap();

        assert!(error.errors.is_empty());
        assert_eq!(error.to_string(), "Retry failed without recorded errors");
        assert_eq!(
            error.last_error().to_string(),
            "Retry failed without recorded errors"
        );
        assert_eq!(serde_json::to_string(&error).unwrap(), json);
    }

    /// A 429 is retryable and carries its hint wherever it lives — the
    /// classification is the `status_code` field, not a variant.
    #[test]
    fn rate_limit_is_read_from_the_field() {
        let err = AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(429),
            response_headers: Some(HashMap::from([("retry-after-ms".into(), "7".into())])),
            is_retryable: true,
            ..api_error("quota exceeded")
        }));
        assert!(err.is_retryable());
        assert_eq!(err.retry_after_hint(), Some(7));
        // A bare ApiCall error (no stored verdict) is not retryable.
        assert!(!AiMuxError::ApiCall(Box::new(api_error("boom"))).is_retryable());

        let json = serde_json::to_string(&err).unwrap();
        let back: AiMuxError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.retry_after_hint(), Some(7));
        assert_eq!(back.status_code(), Some(429));
    }
}
