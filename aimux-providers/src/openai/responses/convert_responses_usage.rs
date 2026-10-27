//! Convert Responses usage to the core usage (convert-openai-responses-usage.ts).

use aimux_core::types::Usage;
use serde_json::Value;

use crate::openai::responses::responses_api::ResponsesUsage;

/// Convert a Responses API usage into the core `Usage` type.
///
/// Mirrors TS `convertOpenAIResponsesUsage`:
/// - `input.total = input_tokens`
/// - `input.noCache = input_tokens - cached_tokens - cache_write_tokens`
/// - `input.cacheRead = cached_tokens`
/// - `input.cacheWrite = cache_write_tokens`
/// - `output.total = output_tokens`
/// - `output.text = output_tokens - reasoning_tokens`
/// - `output.reasoning = reasoning_tokens`
///
/// `raw` is the untouched wire `usage` object; it is stored on `Usage.raw`
/// (RFC-0015 P0-3). Re-serializing the typed struct instead would drop the
/// `orchestration_*` counters and add explicit nulls, so the caller passes the
/// original value through — matching the TS `raw: usage`.
#[must_use]
pub fn convert_responses_usage(usage: Option<&ResponsesUsage>, raw: Option<Value>) -> Usage {
    let Some(usage) = usage else {
        return Usage {
            raw: raw.and_then(|value| value.as_object().cloned()),
            ..Default::default()
        };
    };

    let input_tokens = usage.input_tokens;
    let output_tokens = usage.output_tokens;

    let cached_tokens = usage
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);
    let cache_write = usage
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.cache_write_tokens);
    let reasoning_tokens = usage
        .output_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);

    let no_cache = input_tokens - cached_tokens - cache_write.unwrap_or(0);
    let text_tokens = output_tokens - reasoning_tokens;

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(input_tokens),
            no_cache: Some(no_cache),
            cache_read: Some(cached_tokens),
            cache_write,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(output_tokens),
            text: Some(text_tokens),
            reasoning: Some(reasoning_tokens),
        },
        raw: raw.and_then(|value| value.as_object().cloned()),
    }
}

/// Helper: build a `ResponsesUsage` from a raw JSON `usage` object.
#[must_use]
pub fn parse_usage(raw: &Value) -> Option<ResponsesUsage> {
    serde_json::from_value(raw.clone()).ok()
}
