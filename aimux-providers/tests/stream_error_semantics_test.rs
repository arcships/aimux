//! Stream-error classification and termination semantics (review follow-ups).
//!
//! Locks three behaviors:
//! - no fabricated HTTP 500 for non-numeric provider error codes (the "M3
//!   bug"): such errors carry no status and are not retryable;
//! - payload `retry_after_ms` hints surface through `retry_after_hint()`;
//! - the shared Responses reducer ends its stream right after a terminal
//!   error event instead of waiting on a source that stays open.

use std::collections::HashMap;

use futures::StreamExt;
use serde_json::json;

use aimux_core::AiMuxError;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::stream_error_api_call;
use aimux_providers::openai::responses::responses_convert::build_responses_event_stream;

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

#[tokio::test]
async fn responses_stream_ends_after_terminal_error_event() {
    // First event is benign so the peek passes; the mid-stream error event is
    // followed by a source that never closes (heartbeat-style server).
    let first = json!({"type": "response.created", "response": {"id": "resp_1"}});
    let error_event =
        json!({"type": "error", "error": {"message": "boom", "type": "server_error"}});
    let source = futures::stream::iter(vec![Ok(error_event)]).chain(futures::stream::pending());

    let stream = build_responses_event_stream(
        Some(Ok(first)),
        source,
        "openai".to_string(),
        Vec::new(),
        false,
        "https://example.test".to_string(),
        json!({}),
        HashMap::new(),
    )
    .expect("stream setup");

    let parts: Vec<_> = stream.collect().await;
    assert!(
        parts
            .iter()
            .any(|part| matches!(part, Ok(StreamPart::Error { .. }))),
        "error part yielded"
    );
    let last = parts.last().expect("stream not empty");
    assert!(
        matches!(
            last,
            Ok(StreamPart::Finish { finish_reason, .. })
                if finish_reason.unified == FinishReasonUnified::Error
        ),
        "stream ends with an error Finish, got {last:?}"
    );
}
