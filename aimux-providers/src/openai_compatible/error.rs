//! The error structure of compatible servers (`openai-compatible-error.ts`).

use std::sync::Arc;

use aimux_provider_utils::ProviderErrorParts;
use serde_json::Value;

/// Maps a provider error payload to the message and code of the API error
/// (the AI SDK's `errorStructure`).
pub(crate) type ErrorStructure = Arc<dyn Fn(&Value) -> ProviderErrorParts + Send + Sync>;

/// `{ error: { message, type?, param?, code? } }`, the structure nearly every
/// compatible server answers errors with (the AI SDK's default
/// `errorStructure`).
pub(crate) fn default_error_structure(data: &Value) -> ProviderErrorParts {
    let error = data.get("error").unwrap_or(data);
    ProviderErrorParts {
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        provider_code: error.get("code").or_else(|| error.get("type")).and_then(
            |value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            },
        ),
    }
}
