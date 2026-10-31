//! The provider options of the chat model
//! (`openai-compatible-chat-language-model-options.ts`):
//! `openaiCompatibleLanguageModelChatOptions`.

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::{optional_field, parse_provider_options};
use serde::Deserialize;

/// Option keys of the generic compatible schema
/// (`openaiCompatibleLanguageModelChatOptions`): consumed by the model, never
/// passed through.
pub(super) const SCHEMA_KEYS: [&str; 4] = [
    "user",
    "reasoningEffort",
    "textVerbosity",
    "strictJsonSchema",
];

/// `openaiCompatibleLanguageModelChatOptions`: a zod object of `.optional()`
/// fields, so unknown keys are dropped and an explicit `null` is rejected.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatOptions {
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) user: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) reasoning_effort: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) text_verbosity: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) strict_json_schema: Option<bool>,
}

impl ChatOptions {
    /// `parseProviderOptions({ provider, providerOptions, schema })`.
    pub(super) fn parse(
        provider: &str,
        provider_options: Option<&SharedProviderOptions>,
    ) -> Result<Option<Self>, AiMuxError> {
        parse_provider_options(provider, provider_options)
    }

    /// `Object.assign(self, later)`: every field `later` sets wins.
    pub(super) fn assign(self, later: Self) -> Self {
        Self {
            user: later.user.or(self.user),
            reasoning_effort: later.reasoning_effort.or(self.reasoning_effort),
            text_verbosity: later.text_verbosity.or(self.text_verbosity),
            strict_json_schema: later.strict_json_schema.or(self.strict_json_schema),
        }
    }
}
