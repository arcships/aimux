//! `convert-xai-responses-usage.ts`.

use super::responses_api::XaiResponsesUsage;

#[must_use]
pub fn convert_xai_responses_usage(usage: &XaiResponsesUsage) -> aimux_core::types::Usage {
    let cache_read = usage
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);
    let reasoning = usage
        .output_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);

    let input_includes_cached = cache_read <= usage.input_tokens;
    let input_total = if input_includes_cached {
        usage.input_tokens
    } else {
        usage.input_tokens + cache_read
    };
    let input_no_cache = if input_includes_cached {
        usage.input_tokens - cache_read
    } else {
        usage.input_tokens
    };

    aimux_core::types::Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(input_total as u32),
            no_cache: Some(input_no_cache as u32),
            cache_read: Some(cache_read as u32),
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(usage.output_tokens as u32),
            text: Some((usage.output_tokens - reasoning) as u32),
            reasoning: Some(reasoning as u32),
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: serde_json::to_value(usage)
            .ok()
            .and_then(|value| value.as_object().cloned()),
    }
}
