//! Rust translation of `packages/provider-utils/src/create-provider-stream-error.test.ts`
//! (@ai-sdk/provider-utils).
//!
//! The function returns the `ApiCallError` directly, so there is no marker to
//! test: `isProviderStreamError` and its "unmarked payload" case are
//! TypeScript-only. `ApiCallError` has one code field, so the upstream `type`
//! and `code` are not both kept.

use std::collections::HashMap;

use aimux_core::AiMuxError;
use serde_json::{Value, json};

use crate::{ProviderStreamError, create_provider_stream_error};

fn api_call(error: AiMuxError) -> aimux_core::ApiCallError {
    match error {
        AiMuxError::ApiCall(details) => *details,
        other => panic!("expected an ApiCall error, got {other:?}"),
    }
}

fn build(error: ProviderStreamError, event: &Value) -> aimux_core::ApiCallError {
    api_call(create_provider_stream_error(
        error,
        event,
        "https://api.example.com/v1/stream",
        json!({}),
        HashMap::new(),
    ))
}

/// partial: @ai-sdk/provider-utils src/create-provider-stream-error.test.ts › createProviderStreamError › preserves provider-owned metadata and raw data
#[test]
fn preserves_provider_owned_metadata_and_raw_data() {
    let data = json!({
        "message": "Overloaded",
        "type": "overloaded_error",
        "code": "provider_overloaded",
    });

    let error = build(
        ProviderStreamError {
            message: "Overloaded".into(),
            provider_code: Some("provider_overloaded".into()),
            status_code: Some(529),
            is_retryable: Some(true),
            data: data.clone(),
        },
        &data,
    );

    assert_eq!(error.message, "Overloaded");
    assert_eq!(error.provider_code.as_deref(), Some("provider_overloaded"));
    assert_eq!(error.status_code, Some(529));
    assert!(error.is_retryable);
    assert_eq!(error.data, Some(data));
}

// Not an upstream case: AI SDK Core derives retryability from the status
// when the provider leaves `isRetryable` unset; aimux does it here.
#[test]
fn derives_retryability_from_the_status_when_unset() {
    let event = json!({ "error": { "message": "busy" } });
    let retryable = build(
        ProviderStreamError {
            message: "busy".into(),
            status_code: Some(503),
            data: event.clone(),
            ..ProviderStreamError::default()
        },
        &event,
    );
    let not_retryable = build(
        ProviderStreamError {
            message: "bad".into(),
            status_code: Some(400),
            data: event.clone(),
            ..ProviderStreamError::default()
        },
        &event,
    );
    let quota = build(
        ProviderStreamError {
            message: "quota".into(),
            provider_code: Some("insufficient_quota".into()),
            status_code: Some(429),
            is_retryable: Some(false),
            data: event.clone(),
        },
        &event,
    );

    assert!(retryable.is_retryable);
    assert!(!not_retryable.is_retryable);
    assert!(!quota.is_retryable);
}

// Not an upstream case: the event is the response body, and a retry hint in
// it reaches `retry_after_hint()` through the response headers.
#[test]
fn keeps_the_event_as_response_body_and_surfaces_its_retry_hint() {
    let event =
        json!({ "type": "error", "retry_after_ms": 1500, "error": { "message": "slow down" } });
    let error = build(
        ProviderStreamError {
            message: "slow down".into(),
            status_code: Some(429),
            ..ProviderStreamError::default()
        },
        &event,
    );

    assert_eq!(error.response_body, Some(event.to_string()));
    assert_eq!(
        error
            .response_headers
            .as_ref()
            .and_then(|headers| headers.get("retry-after-ms"))
            .map(String::as_str),
        Some("1500")
    );
    assert_eq!(error.data, None);
}

/// aimux's own: the error says it was built from an event inside a stream,
/// so a host can tell it from an HTTP response with the same status (#400).
#[test]
fn records_a_stream_event_origin() {
    let error = build(
        ProviderStreamError {
            message: "Rate limit reached".into(),
            status_code: Some(429),
            ..ProviderStreamError::default()
        },
        &json!({ "error": { "message": "Rate limit reached" } }),
    );
    assert_eq!(error.origin, aimux_core::ApiCallErrorOrigin::StreamEvent);
}
