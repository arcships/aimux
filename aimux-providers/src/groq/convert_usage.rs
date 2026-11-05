//! Mirrors `convert-groq-usage.ts`.

use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};

use super::api_types::GroqUsage;

/// Convert Groq usage to the core `Usage`. A missing object is the null usage
/// (every count unknown).
pub(crate) fn convert_groq_usage(usage: Option<&GroqUsage>) -> Usage {
    let Some(usage) = usage else {
        return Usage::default();
    };

    // A count the provider did not report stays unknown; a reported zero is
    // reported as zero.
    let prompt_tokens = usage.prompt_tokens;
    let cache_read_tokens = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens);
    let completion_tokens = usage.completion_tokens;
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.reasoning_tokens);
    let text_tokens =
        completion_tokens.map(|total| total.saturating_sub(reasoning_tokens.unwrap_or(0)));

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
            text: text_tokens,
            reasoning: reasoning_tokens,
        },
        raw: Some(usage.raw.clone()),
    }
}
