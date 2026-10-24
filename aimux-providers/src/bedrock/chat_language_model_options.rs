//! Settings and provider options of the Amazon Bedrock chat model
//! (amazon-bedrock-chat-language-model-options.ts).

use serde_json::Value;

/// The chat family override supported by the upstream provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmazonBedrockChatModelFamily {
    Anthropic,
}

impl AmazonBedrockChatModelFamily {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
        }
    }
}

/// Optional family override for opaque inference-profile IDs.
#[derive(Clone, Debug, Default)]
pub struct AmazonBedrockChatModelSettings {
    pub model_family: Option<AmazonBedrockChatModelFamily>,
}

pub(super) fn validate_options(
    options: Option<&aimux_core::shared::JsonObject>,
) -> Result<(), aimux_core::AiMuxError> {
    let Some(options) = options else {
        return Ok(());
    };
    for (key, choices) in [
        (
            "structuredOutputMode",
            &["outputFormat", "jsonTool", "auto"][..],
        ),
        (
            "serviceTier",
            &["reserved", "priority", "default", "flex"][..],
        ),
    ] {
        if let Some(value) = options.get(key)
            && !value.as_str().is_some_and(|v| choices.contains(&v))
        {
            return Err(aimux_core::AiMuxError::invalid_argument(format!(
                "Invalid Bedrock {key}"
            )));
        }
    }
    if let Some(value) = options.get("additionalModelRequestFields")
        && !value.is_object()
    {
        return Err(aimux_core::AiMuxError::invalid_argument(
            "Invalid Bedrock additionalModelRequestFields",
        ));
    }
    if let Some(value) = options.get("anthropicBeta")
        && !value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
    {
        return Err(aimux_core::AiMuxError::invalid_argument(
            "Invalid Bedrock anthropicBeta",
        ));
    }
    if let Some(reasoning) = options.get("reasoningConfig") {
        if !reasoning.is_object() {
            return Err(aimux_core::AiMuxError::invalid_argument(
                "Invalid Bedrock reasoningConfig",
            ));
        }
        for (key, choices) in [
            ("type", &["enabled", "disabled", "adaptive"][..]),
            ("display", &["omitted", "summarized"][..]),
            (
                "maxReasoningEffort",
                &["low", "medium", "high", "xhigh", "max"][..],
            ),
        ] {
            if let Some(value) = reasoning.get(key)
                && !value.as_str().is_some_and(|v| choices.contains(&v))
            {
                return Err(aimux_core::AiMuxError::invalid_argument(format!(
                    "Invalid Bedrock reasoningConfig.{key}"
                )));
            }
        }
        if let Some(budget) = reasoning.get("budgetTokens")
            && !budget.is_number()
        {
            return Err(aimux_core::AiMuxError::invalid_argument(
                "Invalid Bedrock reasoningConfig.budgetTokens",
            ));
        }
    }
    Ok(())
}
