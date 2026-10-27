//! The Amazon Bedrock error response handler (amazon-bedrock-error.ts).

use aimux_core::error::AiMuxError;
use serde_json::Value;

pub(crate) fn bedrock_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        // Converse errors may be a top-level AWS error object or be wrapped
        // as `{ "error": { ... } }` by compatible gateways.
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                match error.get("type").and_then(Value::as_str) {
                    Some(kind) => format!("{kind}: {message}"),
                    None => message,
                }
            },
            provider_code: error
                .get("type")
                .or_else(|| error.get("__type"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}
