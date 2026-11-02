//! ElevenLabs error bodies (`elevenlabs-error.ts`).

use serde_json::Value;

use aimux_core::error::AiMuxError;

/// ElevenLabs errors: `{"detail": {"status": "invalid_api_key", "message": ...}}`
/// where `detail.status` is the machine code. FastAPI validation errors carry
/// `detail` as a plain string or a `[{loc, msg, type}]` list instead. The
/// `{"error": {"message", "code"}}` shape of `elevenlabsErrorDataSchema` gives
/// the code as `error.code`, a number or a string.
pub(super) fn elevenlabs_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let detail = data.get("detail");
    let message = detail
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .or_else(|| detail.and_then(Value::as_str))
        .or_else(|| {
            detail
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("msg"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            data.get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| data.get("error").and_then(Value::as_str))
        .or_else(|| data.get("message").and_then(Value::as_str))
        .unwrap_or("ElevenLabs request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: detail
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(
                || match data.get("error").and_then(|error| error.get("code"))? {
                    Value::String(code) => Some(code.clone()),
                    Value::Number(code) => Some(code.to_string()),
                    _ => None,
                },
            ),
    }
}

pub(super) fn elevenlabs_failed_response_handler()
-> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(elevenlabs_error_parts)
}
