//! DeepSeek usage (`convert-to-deepseek-usage.ts`).

use serde_json::Value;

use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};

fn count(usage: &Value, pointer: &str) -> u32 {
    usage
        .pointer(pointer)
        .and_then(Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// The usage of a DeepSeek `usage` object; no object is null usage. The object
/// is kept as `raw`.
pub(crate) fn convert_deepseek_usage(usage: Option<&Value>) -> Usage {
    let Some(usage) = usage.filter(|usage| !usage.is_null()) else {
        return Usage::default();
    };
    let prompt_tokens = count(usage, "/prompt_tokens");
    let completion_tokens = count(usage, "/completion_tokens");
    let cache_read_tokens = count(usage, "/prompt_cache_hit_tokens");
    let reasoning_tokens = count(usage, "/completion_tokens_details/reasoning_tokens");

    Usage {
        input_tokens: InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(prompt_tokens.saturating_sub(cache_read_tokens)),
            cache_read: Some(cache_read_tokens),
            cache_write: None,
        },
        output_tokens: OutputTokenUsage {
            total: Some(completion_tokens),
            text: Some(completion_tokens.saturating_sub(reasoning_tokens)),
            reasoning: Some(reasoning_tokens),
        },
        raw: usage.as_object().cloned(),
    }
}
