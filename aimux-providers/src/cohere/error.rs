//! `cohere-error.ts`: the failed-response handler.

use serde_json::Value;

use aimux_core::error::AiMuxError;

pub(crate) fn cohere_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: None,
        }
    })
}
