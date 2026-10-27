//! Convert Chat Completions usage to the core usage (convert-openai-chat-usage.ts).

use aimux_core::types::Usage;
use serde_json::Value;

use crate::openai::chat::chat_api::UsageResponse;

/// Convert an OpenAI `UsageResponse` into the core `Usage` type.
///
/// Mirrors the TS `convertOpenAIChatUsage`:
/// - `input.total = prompt_tokens`
/// - `input.noCache = prompt_tokens - cached_tokens - cache_write_tokens`
/// - `input.cacheRead = cached_tokens`
/// - `input.cacheWrite = cache_write_tokens`
///
/// `usage_raw` is the provider's original `usage` JSON object, preserved
/// verbatim in `Usage.raw` (M10, RFC-0016). Vendor-specific fields not part
/// of `UsageResponse` (e.g. prompt-cache hit counters) survive only
/// through this raw value.
pub(crate) fn convert_usage(usage: &UsageResponse, usage_raw: Option<&Value>) -> Usage {
    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    let completion_tokens = usage.completion_tokens.unwrap_or(0);

    // Cache-read tokens: only the nested `prompt_tokens_details.cached_tokens`
    // (upstream convert-openai-chat-usage.ts:28). A top-level `cached_tokens`
    // (Moonshot's wire format) stays in `usage.raw`; Moonshot is an
    // OpenAI-compatible preset with its own usage converter.
    let cached = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);

    let cache_write = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|d| d.cache_write_tokens);

    // Use saturating subtraction: some OpenAI-compatible servers (e.g. vLLM
    // serving Qwen3 reasoning models via Doubleword) report
    // `reasoning_tokens > completion_tokens`, and cached + cache-write tokens
    // can exceed `prompt_tokens`. Saturating to 0 avoids a panic on these
    // real-world responses; the non-underflow path is unchanged.
    let no_cache = prompt_tokens
        .saturating_sub(cached)
        .saturating_sub(cache_write.unwrap_or(0));

    // Reasoning tokens from completion_tokens_details.
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);
    let text_tokens = completion_tokens.saturating_sub(reasoning_tokens);

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(no_cache),
            cache_read: Some(cached),
            cache_write,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(completion_tokens),
            text: Some(text_tokens),
            reasoning: Some(reasoning_tokens),
        },
        // M10 (RFC-0016): keep the provider's original usage JSON verbatim —
        // vendor-specific fields (e.g. a top-level `cached_tokens`, prompt-cache
        // `prompt_cache_hit_tokens`) are otherwise lost for audit/billing.
        raw: usage_raw.and_then(|value| value.as_object().cloned()),
    }
}
