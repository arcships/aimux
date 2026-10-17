//! Mirrors `convert-groq-usage.ts`.

use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};

use super::types::GroqUsage;

/// Convert Groq usage to the core `Usage`. A missing object is the null usage
/// (every count unknown).
pub(crate) fn convert_groq_usage(usage: Option<&GroqUsage>) -> Usage {
    let Some(usage) = usage else {
        return Usage::default();
    };

    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    let cache_read_tokens = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens);
    let completion_tokens = usage.completion_tokens.unwrap_or(0);
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.reasoning_tokens);
    let text_tokens = match reasoning_tokens {
        Some(reasoning) => completion_tokens.saturating_sub(reasoning),
        None => completion_tokens,
    };

    Usage {
        input_tokens: InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(match cache_read_tokens {
                Some(cached) => prompt_tokens.saturating_sub(cached),
                None => prompt_tokens,
            }),
            cache_read: cache_read_tokens,
            cache_write: None,
        },
        output_tokens: OutputTokenUsage {
            total: Some(completion_tokens),
            text: Some(text_tokens),
            reasoning: reasoning_tokens,
        },
        raw: serde_json::to_value(usage)
            .ok()
            .and_then(|value| value.as_object().cloned()),
    }
}
