//! The providerOptions and providerMetadata namespace of Cohere.
//!
//! `@ai-sdk/cohere` reads `providerOptions.cohere` and writes response
//! metadata under `cohere`, whatever the provider is named. This module is
//! the one place that spells the key.

use aimux_core::shared::{SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "cohere";

/// `{ "cohere": payload }`.
pub(crate) fn cohere_metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(NAMESPACE, payload).expect("metadata payload is an object")
}

/// `parseProviderOptions({ provider: 'cohere', providerOptions, schema })`.
///
/// # Errors
///
/// `InvalidArgument` "invalid cohere provider options" when the namespace
/// does not match the schema, with the serde error as its cause.
pub(crate) fn parse<T: serde::de::DeserializeOwned>(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<T>, aimux_core::AiMuxError> {
    aimux_provider_utils::parse_provider_options(NAMESPACE, provider_options)
}

/// `cohereLanguageModelChatOptions`.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct CohereChatOptions {
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) thinking: Option<CohereThinking>,
}

/// `thinking` of `cohereLanguageModelChatOptions`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CohereThinking {
    #[serde(
        default,
        rename = "type",
        deserialize_with = "aimux_provider_utils::optional_field"
    )]
    pub(crate) kind: Option<CohereThinkingType>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) token_budget: Option<serde_json::Number>,
}

/// `z.enum(['enabled', 'disabled'])`.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CohereThinkingType {
    Enabled,
    Disabled,
}

impl CohereThinkingType {
    /// The wire value.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

/// `cohereImagePartProviderOptions`.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct CohereImagePartOptions {
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) detail: Option<CohereImageDetail>,
}

/// `z.enum(['auto', 'low', 'high'])`.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CohereImageDetail {
    Auto,
    Low,
    High,
}

impl CohereImageDetail {
    /// The wire value.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Low => "low",
            Self::High => "high",
        }
    }
}
