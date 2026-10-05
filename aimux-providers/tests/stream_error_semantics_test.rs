//! Stream-error classification and termination semantics (review follow-ups).
//!
//! Locks three behaviors:
//! - no fabricated HTTP 500 for non-numeric provider error codes (the "M3
//!   bug"): such errors carry no status and are not retryable;
//! - payload `retry_after_ms` hints surface through `retry_after_hint()`;
//! - the shared Responses reducer ends its stream right after a terminal
//!   error event instead of waiting on a source that stays open.

use std::collections::HashMap;

use serde_json::json;

use aimux_core::AiMuxError;
use aimux_provider_utils::stream_error_api_call;

fn as_api_call(error: &AiMuxError) -> &aimux_core::ApiCallError {
    match error {
        AiMuxError::ApiCall(detail) => detail,
        other => panic!("expected ApiCall, got {other:?}"),
    }
}

#[test]
fn string_error_code_is_not_a_retryable_500() {
    let payload = json!({"message": "Incorrect API key", "code": "invalid_api_key"});
    let error = stream_error_api_call(
        "Incorrect API key",
        Some("invalid_api_key".into()),
        None,
        &payload,
        "https://example.test",
        json!({}),
        HashMap::new(),
    );
    let detail = as_api_call(&error);
    assert_eq!(detail.status_code, None);
    assert!(!error.is_retryable());
}

#[test]
fn numeric_status_keeps_status_based_retryability() {
    let payload = json!({"message": "rate limited", "code": 429});
    let error = stream_error_api_call(
        "rate limited",
        Some("429".into()),
        Some(429),
        &payload,
        "https://example.test",
        json!({}),
        HashMap::new(),
    );
    assert_eq!(as_api_call(&error).status_code, Some(429));
    assert!(error.is_retryable());
}

#[test]
fn payload_retry_after_ms_surfaces_as_hint() {
    let payload = json!({"message": "rate limited", "code": 429, "retry_after_ms": 15000});
    let error = stream_error_api_call(
        "rate limited",
        None,
        Some(429),
        &payload,
        "https://example.test",
        json!({}),
        HashMap::new(),
    );
    assert_eq!(error.retry_after_hint(), Some(15000));
}

#[test]
fn real_retry_after_header_wins_over_payload() {
    let payload = json!({"message": "rate limited", "retry_after_ms": 15000});
    let headers = HashMap::from([("retry-after-ms".to_string(), "7".to_string())]);
    let error = stream_error_api_call(
        "rate limited",
        None,
        Some(429),
        &payload,
        "https://example.test",
        json!({}),
        HashMap::new(),
    );
    assert_eq!(error.retry_after_hint(), Some(15000));
    let error = stream_error_api_call(
        "rate limited",
        None,
        Some(429),
        &payload,
        "https://example.test",
        json!({}),
        headers,
    );
    assert_eq!(error.retry_after_hint(), Some(7));
}
