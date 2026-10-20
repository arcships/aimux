//! `anthropicFailedResponseHandler` (`anthropic-error.ts`).

use serde_json::Value;

use aimux_core::error::AiMuxError;

pub(crate) fn anthropic_failed_response_handler()
-> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error.get("type").and_then(Value::as_str).map(str::to_owned),
        }
    })
}
