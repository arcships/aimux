//! `convert-google-usage.ts`.

/// Convert a `GoogleUsageMetadata` into the core `Usage` type.
///
/// Mirrors `convertGoogleUsage`:
/// - `input.total = promptTokenCount + toolUsePromptTokenCount`
/// - `input.noCache = input.total - cachedContentTokenCount`
/// - `input.cacheRead = cachedContentTokenCount`
/// - `output.total = candidatesTokenCount + thoughtsTokenCount`
#[must_use]
pub fn convert_usage(usage: &super::api_types::GoogleUsageMetadata) -> aimux_core::types::Usage {
    use aimux_core::types::Usage;

    let prompt =
        usage.prompt_token_count.unwrap_or(0) + usage.tool_use_prompt_token_count.unwrap_or(0);
    let candidates = usage.candidates_token_count.unwrap_or(0);
    let cached = usage.cached_content_token_count.unwrap_or(0);
    let thoughts = usage.thoughts_token_count.unwrap_or(0);

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(prompt),
            no_cache: Some(prompt - cached),
            cache_read: Some(cached),
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(candidates + thoughts),
            text: Some(candidates),
            reasoning: Some(thoughts),
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: serde_json::to_value(usage)
            .ok()
            .and_then(|value| value.as_object().cloned()),
    }
}
