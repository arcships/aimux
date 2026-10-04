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
    pub reasoning_format: Option<ReasoningFormat>,
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Whether to enable parallel function calling during tool use. Default to true.
    pub parallel_tool_calls: Option<bool>,
    /// A unique identifier representing the end-user.
    pub user: Option<String>,
    /// Whether to use structured outputs. Default true.
    pub structured_outputs: Option<bool>,
    /// Whether to use strict JSON schema validation. Only used when structured
    /// outputs are enabled and a schema is provided. Default true.
    pub strict_json_schema: Option<bool>,
    pub service_tier: Option<ServiceTier>,
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
        .and_then(Value::as_object)
    else {
        return Ok(GroqLanguageModelChatOptions::default());
    };
    serde_json::from_value(Value::Object(groq.clone())).map_err(|error| {
        AiMuxError::InvalidArgument(format!(
            "invalid argument for parameter providerOptions: {error}"
        ))
    })
}
