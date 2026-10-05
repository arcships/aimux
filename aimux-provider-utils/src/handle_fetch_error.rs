//! Fetch error normalization.

use aimux_core::{AiMuxError, ApiCallError};

use crate::fetch::FetchError;

/// Attach request context to transport failures without changing other errors.
#[must_use]
pub fn handle_fetch_error(
    error: AiMuxError,
    url: &str,
    request_body_values: &serde_json::Value,
) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(mut detail) => {
            detail.url = url.to_string();
            detail.request_body_values = request_body_values.clone();
            AiMuxError::ApiCall(detail)
        }
        AiMuxError::Aborted(_) | AiMuxError::Timeout(_) => error,
        other => other,
    }
}

/// Convert a transport failure into the public error model: an abort stays an
/// abort; everything else is an `ApiCall` error with no status (no response
/// arrived), retryable exactly when [`FetchError::is_retryable`] says so.
#[must_use]
pub(crate) fn fetch_error_to_ai_mux_error(
    error: FetchError,
    url: &str,
    request_body_values: &serde_json::Value,
) -> AiMuxError {
    match error {
        FetchError::Aborted => AiMuxError::Aborted("request aborted".into()),
        other => AiMuxError::ApiCall(Box::new(ApiCallError {
            is_retryable: other.is_retryable(),
            ..ApiCallError::new(other.to_string(), url, request_body_values.clone())
        })),
    }
}
