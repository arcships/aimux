//! DeepSeek usage (`convert-to-deepseek-usage.ts`).

use serde_json::Value;

use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};

fn count(usage: &Value, pointer: &str) -> Option<u32> {
    usage
        .pointer(pointer)
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
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

    // #478: a count the provider did not report stays unknown (the AI SDK
    // fills 0); a reported zero stays zero.
    Usage {
        input_tokens: InputTokenUsage {
            total: prompt_tokens,
            no_cache: prompt_tokens
                .map(|total| total.saturating_sub(cache_read_tokens.unwrap_or(0))),
            cache_read: cache_read_tokens,
            cache_write: None,
        },
        output_tokens: OutputTokenUsage {
            total: completion_tokens,
            text: completion_tokens
                .map(|total| total.saturating_sub(reasoning_tokens.unwrap_or(0))),
            reasoning: reasoning_tokens,
        },
        raw: usage.as_object().cloned(),
    }
}
