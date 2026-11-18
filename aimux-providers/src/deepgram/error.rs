//! Deepgram error bodies (`deepgram-error.ts`).

use serde_json::Value;

use aimux_core::error::AiMuxError;

/// Deepgram errors: `{"err_code": "...", "err_msg": "...", "request_id": ...}`
/// on most endpoints; some return `{"category": "...", "message": "...",
/// "details": ...}` instead (https://developers.deepgram.com/docs/errors).
pub(super) fn deepgram_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let message = data
        .get("err_msg")
        .and_then(Value::as_str)
        .or_else(|| data.get("message").and_then(Value::as_str))
        .or_else(|| {
            data.get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| data.get("error").and_then(Value::as_str))
        .unwrap_or("Deepgram request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: data
            .get("err_code")
            .or_else(|| data.get("category"))
            .and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
    }
}

pub(super) fn deepgram_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(deepgram_error_parts)
}
