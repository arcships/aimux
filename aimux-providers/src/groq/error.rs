//! Mirrors `groq-error.ts`: the `{ "error": { "message", "type" } }` shape of
//! a failed response.

use serde::Deserialize;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{ResponseHandler, create_standard_json_error_response_handler};

pub(crate) fn groq_failed_response_handler() -> ResponseHandler<AiMuxError> {
    create_standard_json_error_response_handler()
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
