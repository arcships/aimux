//! The provider options of the chat model
//! (`openai-compatible-chat-language-model-options.ts`): the keys of
//! `openaiCompatibleLanguageModelChatOptions` and their checks.

use aimux_core::error::AiMuxError;
use serde_json::{Map, Value};

/// Option keys of the generic compatible schema
/// (`openaiCompatibleLanguageModelChatOptions`): consumed by the model, never
/// passed through.
pub(super) const SCHEMA_KEYS: [&str; 4] = [
    "user",
    "reasoningEffort",
    "textVerbosity",
    "strictJsonSchema",
];

pub(super) fn option_string(
    merged: &Map<String, Value>,
    namespace_name: &str,
    key: &str,
) -> Result<Option<String>, AiMuxError> {
    match merged.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(AiMuxError::InvalidArgument(format!(
            "invalid provider options for \"{namespace_name}\": `{key}` must be a string"
        ))),
    }
}

pub(super) fn validate_options(
    options: &Map<String, Value>,
    namespace_name: &str,
) -> Result<(), AiMuxError> {
    for key in ["user", "reasoningEffort", "textVerbosity"] {
        option_string(options, namespace_name, key)?;
    }
    if options
        .get("strictJsonSchema")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "invalid provider options for \"{namespace_name}\": `strictJsonSchema` must be a boolean"
        )));
    }
    Ok(())
}
