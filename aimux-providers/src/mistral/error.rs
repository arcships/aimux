//! `mistral-error.ts`: the failed-response handler, plus the in-stream
//! error the chat model raises.

use serde_json::Value;

use aimux_core::error::AiMuxError;

pub(crate) fn mistral_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: data
                .get("code")
                .or_else(|| data.get("type"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}

pub(crate) fn mistral_stream_error(
    error: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let status_code = error
        .get("status_code")
        .or_else(|| error.get("status"))
        .or_else(|| error.get("code"))
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok())
        .filter(|status| (400..=599).contains(status));
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Mistral stream failed before any output was generated")
        .to_owned();
    let provider_code =
        error
            .get("code")
            .or_else(|| error.get("type"))
            .and_then(|value| match value {
                Value::String(code) => Some(code.clone()),
                Value::Number(code) => Some(code.to_string()),
                _ => None,
            });
    aimux_provider_utils::stream_error_api_call(
        message,
        provider_code,
        status_code,
        error,
        url,
        request_body_values,
        response_headers,
    )
}
