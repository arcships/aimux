//! The `ApiCallError` of an OpenAI stream error frame (openai-stream-error.ts).

use aimux_core::error::AiMuxError;
use serde_json::Value;

/// The `ApiCallError` of an OpenAI stream error frame, as upstream's
/// `createOpenAIProviderStreamError` builds it (@ai-sdk/openai 4.0.83
/// src/openai-stream-error.ts:18-37): an HTTP status in `code` is the status,
/// otherwise one is inferred from `code`/`type` (`getStatusCode`, :235-267);
/// `insufficient_quota` is never retryable (:303-315).
pub(crate) fn openai_stream_error(
    error: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let response_failed = error.get("type").and_then(Value::as_str) == Some("response.failed");
    let payload = if response_failed {
        error
            .get("response")
            .and_then(|response| response.get("error"))
            .unwrap_or(error)
    } else {
        error.get("error").unwrap_or(error)
    };
    let message = payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("OpenAI stream failed before any output was generated")
        .to_owned();
    let provider_code = payload.get("code").and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    });
    // Upstream types a `response.failed` frame as `response.failed`, not by
    // its inner error (`parseStreamError`, openai-stream-error.ts:207-217).
    let error_type = if response_failed {
        "response.failed"
    } else {
        payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let discriminator = format!(
        "{} {error_type}",
        provider_code.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    let explicit_status = payload
        .get("code")
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .or_else(|| {
                    value
                        .as_str()
                        .filter(|value| {
                            value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit())
                        })
                        .and_then(|value| value.parse::<u16>().ok())
                })
        })
        .filter(|value| (400..=599).contains(value));
    let status = explicit_status.unwrap_or_else(|| {
        if ["insufficient_quota", "rate_limit"]
            .iter()
            .any(|term| discriminator.contains(term))
        {
            429
        } else if discriminator.contains("authentication") {
            401
        } else if discriminator.contains("permission") {
            403
        } else if discriminator.contains("not_found") {
            404
        } else if ["invalid", "bad_request", "context_length"]
            .iter()
            .any(|term| discriminator.contains(term))
        {
            400
        } else if discriminator.contains("overload") {
            503
        } else if discriminator.contains("timeout") {
            504
        } else {
            500
        }
    });
    let quota = provider_code.as_deref() == Some("insufficient_quota")
        || error_type == "insufficient_quota";
    aimux_provider_utils::create_provider_stream_error(
        aimux_provider_utils::ProviderStreamError {
            message,
            provider_code,
            status_code: Some(status),
            is_retryable: quota.then_some(false),
            data: error.clone(),
        },
        error,
        url,
        request_body_values,
        response_headers,
    )
}
