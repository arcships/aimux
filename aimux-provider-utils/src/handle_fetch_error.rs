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
pub fn fetch_error_to_ai_mux_error(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(error: FetchError) -> AiMuxError {
        fetch_error_to_ai_mux_error(error, "https://example.test/v1", &serde_json::json!({}))
    }

    #[test]
    fn transport_failures_become_retryable_api_calls_without_a_status() {
        for error in [
            FetchError::Connect("refused".into()),
            FetchError::Timeout,
            FetchError::Io("reset".into()),
        ] {
            match convert(error) {
                AiMuxError::ApiCall(detail) => {
                    assert!(detail.is_retryable);
                    assert_eq!(detail.status_code, None);
                    assert_eq!(detail.url, "https://example.test/v1");
                }
                other => panic!("expected ApiCall, got {other:?}"),
            }
        }
    }

    #[test]
    fn other_failures_are_not_retried_and_aborts_stay_aborts() {
        match convert(FetchError::Other("bad".into())) {
            AiMuxError::ApiCall(detail) => assert!(!detail.is_retryable),
            other => panic!("expected ApiCall, got {other:?}"),
        }
        assert!(matches!(
            convert(FetchError::Aborted),
            AiMuxError::Aborted(_)
        ));
    }
}
