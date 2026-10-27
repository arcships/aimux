//! `convert-huggingface-responses-usage.ts`.

use serde_json::Value;

use aimux_core::types::Usage;

/// Convert a Hugging Face Responses usage object to the unified [`Usage`].
///
/// Mirrors the TS `convertHuggingFaceResponsesUsage`. `None` / null usage
/// produces an all-`None` `Usage` (matching the TS undefined-fields case).
pub(super) fn convert_usage(usage: Option<&Value>) -> Usage {
    let u = match usage {
        None | Some(Value::Null) => return Usage::default(),
        Some(u) => u,
    };

    let input_tokens = u
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;
    let output_tokens = u
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;
    let cached_tokens = u
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;
    let reasoning_tokens = u
        .get("output_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(input_tokens),
            no_cache: Some(input_tokens - cached_tokens),
            cache_read: Some(cached_tokens),
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(output_tokens),
            text: Some(output_tokens - reasoning_tokens),
            reasoning: Some(reasoning_tokens),
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: u.as_object().cloned(),
    }
}
