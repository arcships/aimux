//! The token usage of a completion response
//! (`convert-openai-compatible-completion-usage.ts`).

use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};
use serde_json::{Map, Value};

pub(crate) fn usage(raw: Option<Map<String, Value>>) -> Usage {
    let Some(raw) = raw else {
        return Usage::default();
    };
    let input = raw
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    let output = raw
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    // #478: a count the provider did not report stays unknown (the AI SDK
    // fills 0); a reported zero stays zero.
    Usage {
        input_tokens: InputTokenUsage {
            total: input,
            no_cache: input,
            ..Default::default()
        },
        output_tokens: OutputTokenUsage {
            total: output,
            text: output,
            ..Default::default()
        },
        raw: Some(raw),
    }
}
