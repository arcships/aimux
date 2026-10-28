//! The token usage of a completion response
//! (`convert-openai-compatible-completion-usage.ts`).

use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};
use serde_json::{Map, Value};

pub(super) fn usage(raw: Option<Map<String, Value>>) -> Usage {
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
    Usage {
        input_tokens: InputTokenUsage {
            total: Some(input.unwrap_or(0)),
            no_cache: Some(input.unwrap_or(0)),
            ..Default::default()
        },
        output_tokens: OutputTokenUsage {
            total: Some(output.unwrap_or(0)),
            text: Some(output.unwrap_or(0)),
            ..Default::default()
        },
        raw: Some(raw),
    }
}
