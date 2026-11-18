//! The providerOptions and providerMetadata namespace of Mistral, and the
//! chat options (`mistral-chat-language-model-options.ts`).
//!
//! `@ai-sdk/mistral` reads `providerOptions.mistral` and writes response
//! metadata under `mistral`, whatever the provider is named. This module is
//! the one place that spells the key, and holds the zod refinements the
//! other option schemas share as serde types.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use aimux_core::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::{optional_field, parse_provider_options};

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "mistral";

/// `parseProviderOptions({ provider: 'mistral', providerOptions, schema })`.
pub(super) fn parse<T: serde::de::DeserializeOwned>(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<T>, AiMuxError> {
    parse_provider_options(NAMESPACE, provider_options)
}

/// `mistralLanguageModelChatOptions`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MistralChatOptions {
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) safe_prompt: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) document_image_limit: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) document_page_limit: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) structured_outputs: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) strict_json_schema: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) parallel_tool_calls: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) prompt_cache_key: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) reasoning_effort: Option<MistralReasoningEffort>,
}

/// `z.enum(['high', 'none'])`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MistralReasoningEffort {
    High,
    None,
}

impl MistralReasoningEffort {
    /// The wire value.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::None => "none",
        }
    }
}

/// The chat options, parsed (`None` without a `mistral` namespace).
///
/// # Errors
///
/// `InvalidArgument` "invalid mistral provider options" when the namespace
/// does not match the schema.
pub(crate) fn chat_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<MistralChatOptions>, AiMuxError> {
    parse(provider_options)
}

/// `z.string().min(1)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NonEmptyString(pub(super) String);

impl<'de> Deserialize<'de> for NonEmptyString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.is_empty() {
            return Err(D::Error::custom(
                "too small: expected string to have >=1 characters",
            ));
        }
        Ok(Self(text))
    }
}
