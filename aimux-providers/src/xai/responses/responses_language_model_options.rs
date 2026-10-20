//! The `providerOptions.xai` checks of `xai-responses-language-model-options.ts`.

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use serde_json::Value;

pub(super) fn xai_option(options: &Option<SharedProviderOptions>, key: &str) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| crate::xai::options::xai_options(Some(m)))
        .and_then(|o| o.get(key))
        .cloned()
}

pub(super) fn validate_value(
    key: &str,
    value: &Value,
    valid: impl FnOnce(&Value) -> bool,
) -> Result<(), AiMuxError> {
    if valid(value) {
        Ok(())
    } else {
        Err(AiMuxError::InvalidArgument(format!(
            "Invalid xai option {key}: {value}"
        )))
    }
}

pub(super) fn string_array(value: &Value, maximum: usize) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.len() <= maximum && items.iter().all(Value::is_string))
}

pub(super) fn validate_response_options(
    options: &Option<SharedProviderOptions>,
) -> Result<(), AiMuxError> {
    for (key, value) in options
        .as_ref()
        .and_then(|options| crate::xai::options::xai_options(Some(options)))
        .into_iter()
        .flatten()
    {
        validate_value(key, value, |value| match key.as_str() {
            "reasoningEffort" => value
                .as_str()
                .is_some_and(|v| ["none", "low", "medium", "high", "xhigh"].contains(&v)),
            "reasoningSummary" => value
                .as_str()
                .is_some_and(|v| ["auto", "concise", "detailed"].contains(&v)),
            "serviceTier" => value
                .as_str()
                .is_some_and(|v| ["default", "priority"].contains(&v)),
            "topLogprobs" => value
                .as_f64()
                .is_some_and(|v| v.fract() == 0.0 && (0.0..=8.0).contains(&v)),
            "minP" => value.as_f64().is_some_and(|v| (0.0..=1.0).contains(&v)),
            "maxTurns" => value
                .as_f64()
                .is_some_and(|v| v.fract() == 0.0 && v.abs() <= 9_007_199_254_740_991.0),
            "logprobs" | "store" | "parallelToolCalls" => value.is_boolean(),
            "promptCacheKey" | "safetyIdentifier" | "user" | "previousResponseId" => {
                value.is_string()
            }
            "include" => {
                value.is_null()
                    || value.as_array().is_some_and(|items| {
                        items.iter().all(|item| {
                            item.as_str().is_some_and(|v| {
                                [
                                    "file_search_call.results",
                                    "web_search_call.action.sources",
                                    "code_interpreter_call.outputs",
                                    "reasoning.encrypted_content",
                                    "no_inline_citations",
                                ]
                                .contains(&v)
                            })
                        })
                    })
            }
            _ => true,
        })?;
    }
    Ok(())
}
