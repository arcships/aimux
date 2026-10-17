//! Mirrors `groq-error.ts`: the `{ "error": { "message", "type" } }` shape of
//! a failed response.

use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    ProviderErrorParts, ResponseHandler, create_json_error_response_handler,
};

/// `groqFailedResponseHandler` (groq@4.0.52 `groq-error.ts:4-16`): the
/// message is `error.message`; `error.type` is the provider code.
pub(crate) fn groq_failed_response_handler() -> ResponseHandler<AiMuxError> {
    create_json_error_response_handler(|data| {
        let error = data.get("error");
        let field = |name: &str| {
            error
                .and_then(|error| error.get(name))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        ProviderErrorParts {
            message: field("message").unwrap_or_default(),
            provider_code: field("type"),
        }
    })
}

/// `groqErrorDataSchema`, also the shape of an error chunk of a stream.
#[derive(Debug, Deserialize)]
pub(crate) struct GroqErrorData {
    pub error: GroqError,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqError {
    pub message: String,
    pub r#type: String,
}
