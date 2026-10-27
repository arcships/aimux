//! The token usage of a completion response
//! (`convert-openai-compatible-completion-usage.ts`).

use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};
use serde_json::{Map, Value};

pub(super) fn usage(raw: Option<Map<String, Value>>, native: bool) -> Usage {
    let Some(mut raw) = raw else {
        return Usage::default();
    };
    if native {
        raw.retain(|key, _| {
            matches!(
                key.as_str(),
                "prompt_tokens" | "completion_tokens" | "total_tokens"
            )
        });
    }
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
            total: if native {
                input
            } else {
                Some(input.unwrap_or(0))
            },
            no_cache: Some(input.unwrap_or(0)),
            ..Default::default()
        },
        output_tokens: OutputTokenUsage {
            total: if native {
                output
            } else {
                Some(output.unwrap_or(0))
            },
            text: Some(output.unwrap_or(0)),
            ..Default::default()
        },
        raw: Some(raw),
    }
}
