//! The providerOptions namespace and the chat options of Groq.
//!
//! Mirrors `groq-chat-language-model-options.ts`. `@ai-sdk/groq` reads
//! `providerOptions.groq`, whatever the provider is named; this module is the
//! one place that spells the key.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "groq";

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ReasoningFormat {
    Parsed,
    Raw,
    Hidden,
}

/// Reasoning effort level for model inference.
/// See <https://console.groq.com/docs/reasoning#reasoning-effort>.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ReasoningEffort {
    None,
    Default,
    Low,
    Medium,
    High,
}

/// Service tier for the request.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceTier {
    OnDemand,
    Performance,
    Flex,
    Auto,
}

/// `groqLanguageModelChatOptions`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroqLanguageModelChatOptions {
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub reasoning_format: Option<ReasoningFormat>,
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Whether to enable parallel function calling during tool use. Default to true.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub parallel_tool_calls: Option<bool>,
    /// A unique identifier representing the end-user.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub user: Option<String>,
    /// Whether to use structured outputs. Default true.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub structured_outputs: Option<bool>,
    /// Whether to use strict JSON schema validation. Only used when structured
    /// outputs are enabled and a schema is provided. Default true.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub strict_json_schema: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub service_tier: Option<ServiceTier>,
}

fn deserialize_optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Parse the Groq options of a call (`parseProviderOptions` with provider
/// `groq`): only the `groq` namespace is read.
///
/// # Errors
///
/// `InvalidArgument` when an option has the wrong type or value.
pub(crate) fn parse_groq_options(
    options: &CallOptions,
) -> Result<GroqLanguageModelChatOptions, AiMuxError> {
    let Some(groq) = options
        .provider_options
        .as_ref()
        .and_then(|all| all.get(NAMESPACE))
        .filter(|value| !value.is_null())
    else {
        return Ok(GroqLanguageModelChatOptions::default());
    };
    let groq = groq.as_object().ok_or_else(|| {
        AiMuxError::InvalidArgument(
            "invalid argument for parameter providerOptions: invalid groq provider options"
                .to_string(),
        )
    })?;
    serde_json::from_value(Value::Object(groq.clone())).map_err(|error| {
        AiMuxError::InvalidArgument(format!(
            "invalid argument for parameter providerOptions: {error}"
        ))
    })
}
