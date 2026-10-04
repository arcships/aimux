//! The providerOptions namespace and the chat options of Groq.
//!
//! Mirrors `groq-chat-language-model-options.ts`. `@ai-sdk/groq` reads
//! `providerOptions.groq` and reports provider metadata under `groq`, whatever
//! the provider is named; this module is the one place that spells the key.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "groq";

/// The generic OpenAI-compatible namespace, read under the Groq one.
const GENERIC_NAMESPACE: &str = "openaiCompatible";

/// `{ "groq": payload }`.
pub(crate) fn groq_metadata(payload: Value) -> Value {
    json!({ NAMESPACE: payload })
}

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

/// The option names the schema above consumes; any other field of the `groq`
/// namespace goes to the request body as given.
const SCHEMA_KEYS: [&str; 7] = [
    "reasoningFormat",
    "reasoningEffort",
    "parallelToolCalls",
    "user",
    "structuredOutputs",
    "strictJsonSchema",
    "serviceTier",
];

fn namespace<'a>(options: &'a CallOptions, key: &str) -> Option<&'a Map<String, Value>> {
    options
        .provider_options
        .as_ref()
        .and_then(|all| all.get(key))
        .and_then(Value::as_object)
}

/// Parse the Groq options of a call (`parseProviderOptions`): the generic
/// `openaiCompatible` namespace, then the `groq` one over it. Also returns the
/// fields of the `groq` namespace the schema does not know.
///
/// # Errors
///
/// `InvalidArgument` when an option has the wrong type or value.
pub(crate) fn parse_groq_options(
    options: &CallOptions,
) -> Result<(GroqLanguageModelChatOptions, Map<String, Value>), AiMuxError> {
    let groq = namespace(options, NAMESPACE);
    let mut merged = Map::new();
    for object in [namespace(options, GENERIC_NAMESPACE), groq]
        .into_iter()
        .flatten()
    {
        merged.extend(object.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let parsed = serde_json::from_value(Value::Object(merged)).map_err(|error| {
        AiMuxError::InvalidArgument(format!(
            "invalid argument for parameter providerOptions: {error}"
        ))
    })?;
    let extra = groq
        .into_iter()
        .flatten()
        .filter(|(key, _)| !SCHEMA_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok((parsed, extra))
}
