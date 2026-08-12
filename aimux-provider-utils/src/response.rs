//! Response handling helpers.

use aimux_core::{AiMuxError, ApiCallError};

/// Provider error structure (allows each provider to define its own error JSON shape).
pub struct ErrorStructure {
    /// JSON path to the error message (e.g. `["error", "message"]`).
    pub message_path: &'static [&'static str],
    /// JSON path to the error type (e.g. `["error", "type"]`).
    pub type_path: &'static [&'static str],
}

/// Default OpenAI-compatible error structure: `{ "error": { "message": "...", "type": "..." } }`.
pub const DEFAULT_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "type"],
};

/// Parse an HTTP error response into an `AiMuxError`.
///
/// Every status becomes an `ApiCall` error; the classification is the
/// `status_code` field (401 auth, 404 model, 429 rate limit, …), read by the
/// caller the way the AI SDK reads `APICallError.statusCode`.
/// Retryability (408/409/429/5xx) is stored in `is_retryable` at
/// construction, as the AI SDK stores `APICallError.isRetryable`.
pub fn parse_provider_error(status: u16, body: &str, structure: &ErrorStructure) -> AiMuxError {
    // Empty: `ApiCallError`'s Display shows the status on its own.
    let mut message = String::new();
    let mut provider_code = None;

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        // Try to extract message.
        let mut current = &val;
        for key in structure.message_path {
            if let Some(v) = current.get(key) {
                current = v;
            } else {
                current = &val;
                break;
            }
        }
        if let Some(msg) = current.as_str() {
            message = msg.to_string();
        } else if !body.is_empty() {
            // The configured message path didn't land on a string value (e.g.
            // xAI's flat responses-API shape `{"code","error"}` where `error`
            // is itself a string, not an object). Surface the raw body so the
            // provider error still carries the upstream payload instead of a
            // bare "HTTP {status}" placeholder.
            message = body.to_string();
        }

        // Try to extract type.
        let mut current = &val;
        for key in structure.type_path {
            if let Some(v) = current.get(key) {
                current = v;
            } else {
                current = &val;
                break;
            }
        }
        if let Some(t) = current.as_str() {
            provider_code = Some(t.to_string());
        }
    } else if !body.is_empty() {
        message = body.to_string();
    }

    // Raw evidence (`APICallError.responseBody`): the AI SDK keeps the body on
    // every error branch — parsed, unparseable or empty — so consumers never
    // have to ask whether it is there. Same here: whenever a body arrived, it
    // is kept verbatim, even when `message` is that same body.
    let response_body = (!body.is_empty()).then(|| body.to_string());
    error_for_status(status, provider_code, message, None, response_body)
}

/// Build the `ApiCall` error for a failed HTTP response.
///
/// There is no status → variant mapping any more: the classification *is*
/// the `status_code` field, read by consumers exactly as the AI SDK reads
/// `APICallError.statusCode`. The retry hint is stored only when the provider
/// actually sent one (`retry-after-ms`/`retry-after`) — no fallback is
/// fabricated; a hint-less 429 gets the retry loop's exponential backoff.
pub fn error_for_status(
    status: u16,
    provider_code: Option<String>,
    message: String,
    retry_after_ms: Option<u64>,
    response_body: Option<String>,
) -> AiMuxError {
    AiMuxError::ApiCall(ApiCallError {
        status_code: Some(status),
        provider_code,
        message,
        response_body,
        retry_after_ms,
        // Stored at construction (`APICallError.isRetryable`), aligned with
        // the AI SDK response policy: 408/409/429/5xx retry, other 4xx don't
        // (RFC-0016 alignment goal).
        is_retryable: matches!(status, 408 | 409 | 429) || status >= 500,
        ..Default::default()
    })
}

/// Convert a stream-error JSON object (the `{"message","type","code",…}` payload
/// carried by an SSE `error` event) into an `AiMuxError`.
///
/// Shares [`error_for_status`] with [`parse_provider_error`], so the same status
/// yields the same variant — and the same retryability — on both paths (M3).
///
/// `code` is a *machine-readable provider code*: OpenAI-shaped payloads put a
/// string there (`"rate_limit_exceeded"`), Google-shaped ones an HTTP status.
/// Only a number in the HTTP range is read as a status; a string code lands in
/// `provider_code`. Reading a string `code` as a status was the M3 bug, and so
/// was defaulting a missing one to 500 — a mid-stream error arrives on a
/// *successful* response, so "no status" is the truth.
pub fn parse_stream_error(err_obj: &serde_json::Value) -> AiMuxError {
    let str_field = |k: &str| {
        err_obj
            .get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let message = str_field("message").unwrap_or_else(|| "Unknown stream error".to_string());
    let code = err_obj.get("code");
    let status = code
        .and_then(|v| v.as_u64())
        .filter(|c| (100..=599).contains(c))
        .map(|c| c as u16);
    let provider_code = code
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| str_field("type"));
    // Retry hint from the payload when the provider sends one (ms wins over
    // whole seconds); absent means absent — nothing is fabricated.
    let retry_after_ms = err_obj
        .get("retry_after_ms")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            err_obj
                .get("retry_after")
                .and_then(|v| v.as_u64())
                .map(|s| s * 1000)
        });

    match status {
        Some(status) => error_for_status(
            status,
            provider_code,
            message,
            retry_after_ms,
            Some(err_obj.to_string()),
        ),
        None => AiMuxError::ApiCall(ApiCallError {
            provider_code,
            message,
            response_body: Some(err_obj.to_string()),
            retry_after_ms,
            ..Default::default()
        }),
    }
}
