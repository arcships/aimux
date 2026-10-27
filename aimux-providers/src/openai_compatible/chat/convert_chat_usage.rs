//! The token usage of a chat response (`convert-openai-compatible-chat-usage.ts`).

use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};
use serde_json::Value;

use super::api_types::UsageResponse;

/// Core usage from a parsed OpenAI-shaped `usage` object.
///
/// `raw` is the provider's original object, kept verbatim in `Usage.raw`.
/// Cache reads come from `prompt_tokens_details`, as in the upstream converter.
pub(crate) fn convert_usage(usage: &UsageResponse, raw: Option<&Value>) -> Usage {
    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    let completion_tokens = usage.completion_tokens.unwrap_or(0);

    let cached = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);

    // Saturating: some servers report more cached or reasoning tokens than
    // prompt or completion tokens.
    let no_cache = prompt_tokens.saturating_sub(cached);
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);

    Usage {
        input_tokens: InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(no_cache),
            cache_read: Some(cached),
            cache_write: None,
        },
        output_tokens: OutputTokenUsage {
            total: Some(completion_tokens),
            text: Some(completion_tokens.saturating_sub(reasoning_tokens)),
            reasoning: Some(reasoning_tokens),
        },
        raw: raw.and_then(Value::as_object).cloned(),
    }
}

/// Usage from a raw `usage` value.
pub(crate) fn usage_from_raw(raw: Option<&Value>) -> Usage {
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Usage::default();
    };
    match serde_json::from_value::<UsageResponse>(raw.clone()) {
        Ok(parsed) => convert_usage(&parsed, Some(raw)),
        Err(_) => Usage {
            raw: raw.as_object().cloned(),
            ..Usage::default()
        },
    }
}
