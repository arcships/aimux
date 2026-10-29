//! `createProviderStreamError` (create-provider-stream-error.ts): the error of
//! an error event delivered inside a stream.

use std::collections::HashMap;

use aimux_core::{AiMuxError, ApiCallError};
use serde_json::Value;

/// What a provider reads from an in-stream error event, as upstream's
/// `ProviderStreamError` carries it.
///
/// `ApiCallError` has a single code field, so the provider picks the
/// upstream `code` or `type` it reports as `provider_code`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderStreamError {
    pub message: String,
    pub provider_code: Option<String>,
    /// The status the provider maps the event to: a numeric HTTP status the
    /// payload carried, or the one its status table infers. The event rides
    /// on an already-successful response, so that 2xx status is not the
    /// error's.
    pub status_code: Option<u16>,
    /// `None` derives retryability from `status_code`.
    pub is_retryable: Option<bool>,
    /// The provider's raw error data; `null` reports none.
    pub data: Value,
}

/// Build the `ApiCallError` for an error event delivered inside a stream.
///
/// `event` is the frame as received: it becomes the response body, and a
/// `retry_after_ms` / `retry_after` field in it is surfaced through the
/// response-headers channel so `AiMuxError::retry_after_hint()` observes it.
///
/// Callers reach this from inside a live stream holding the *raw* request
/// body and URL they built — unlike the exchange path, where `post_*_to_api`
/// redacts before any handler runs. Redaction and URL sanitization therefore
/// happen here, so credentials and data URLs never leak into the public
/// error (or the recordings that persist it).
#[must_use]
pub fn create_provider_stream_error(
    error: ProviderStreamError,
    event: &Value,
    url: impl Into<String>,
    request_body_values: Value,
    mut response_headers: HashMap<String, String>,
) -> AiMuxError {
    // Real headers win; the payload hint fills in only when absent (an
    // in-stream rate-limit error rides on a 200 whose headers carry none).
    if !response_headers.contains_key("retry-after-ms")
        && !response_headers.contains_key("retry-after")
    {
        if let Some(ms) = event.get("retry_after_ms").and_then(Value::as_f64) {
            response_headers.insert("retry-after-ms".into(), ms.to_string());
        } else if let Some(seconds) = event.get("retry_after").and_then(Value::as_f64) {
            response_headers.insert("retry-after".into(), seconds.to_string());
        }
    }
    let ProviderStreamError {
        message,
        provider_code,
        status_code,
        is_retryable,
        data,
    } = error;
    AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code,
        provider_code,
        response_body: Some(event.to_string()),
        response_headers: Some(response_headers),
        data: (!data.is_null()).then(|| crate::logging::redact_error_context(data)),
        is_retryable: is_retryable
            .unwrap_or_else(|| status_code.is_some_and(aimux_core::error::is_retryable_status)),
        ..ApiCallError::new(
            message,
            crate::http::sanitized_request_url(&url.into()),
            crate::logging::redact_error_context(request_body_values),
        )
    }))
}
