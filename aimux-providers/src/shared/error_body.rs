//! Error bodies that several vendor packages share.
//!
//! Each vendor keeps its own `<vendor>_failed_response_handler`; the ones
//! whose error body has one of the shapes below build it from here.

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_provider_utils::{
    ProviderErrorParts, ResponseHandler, create_json_error_response_handler,
};

/// A JSON error code that is a string or a number, as a string.
pub(crate) fn json_error_code(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `{"error": {"message": "...", "code": "..." | 123}}`: the message, else
/// `fallback`, and the code (a string or a number).
pub(crate) fn nested_error_response_handler(fallback: &'static str) -> ResponseHandler<AiMuxError> {
    create_json_error_response_handler(move |data| {
        let error = data.get("error");
        ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or(fallback)
                .to_string(),
            provider_code: error
                .and_then(|value| value.get("code"))
                .and_then(json_error_code),
        }
    })
}

/// `{"error": {"message", "type" | "code"}}`, or the same fields at the top
/// level when there is no `error` object: the message (empty when absent) and
/// `type`, else `code`, as the provider code (strings only).
pub(crate) fn message_type_error_response_handler() -> ResponseHandler<AiMuxError> {
    create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("type")
                .or_else(|| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}
