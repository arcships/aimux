//! The providerOptions and providerMetadata namespace of Mistral.
//!
//! `@ai-sdk/mistral` reads `providerOptions.mistral` and writes response
//! metadata under `mistral`, whatever the provider is named. This module is
//! the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "mistral";

/// The Mistral options in a providerOptions map.
pub(crate) fn mistral_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// Validate only the options declared by the upstream Mistral schema.
pub(crate) fn validated_mistral_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<&JsonObject>, aimux_core::AiMuxError> {
    let Some(options) = mistral_options(provider_options) else {
        return Ok(None);
    };
    for (key, value) in options {
        let valid = match key.as_str() {
            "safePrompt" | "structuredOutputs" | "strictJsonSchema" | "parallelToolCalls" => {
                value.is_boolean()
            }
            "documentImageLimit" | "documentPageLimit" => value.is_number(),
            "promptCacheKey" => value.is_string(),
            "reasoningEffort" => matches!(value.as_str(), Some("high" | "none")),
            _ => true,
        };
        if !valid {
            return Err(aimux_core::AiMuxError::InvalidArgument(format!(
                "Invalid mistral provider option {key}"
            )));
        }
    }
    Ok(Some(options))
}
