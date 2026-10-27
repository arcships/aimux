//! `map-mistral-finish-reason.ts`.

use aimux_core::types::{FinishReason, FinishReasonUnified};

/// Parse Mistral finish reason string into `FinishReason`.
///
/// Differences from OpenAI: `model_length` is also mapped to `Length`, and
/// `error` to `Error` (`@ai-sdk/mistral@4.0.55` map-mistral-finish-reason.ts).
#[must_use]
pub fn parse_finish_reason(s: &str) -> FinishReason {
    let unified = match s {
        "stop" => FinishReasonUnified::Stop,
        "length" | "model_length" => FinishReasonUnified::Length,
        "tool_calls" => FinishReasonUnified::ToolCalls,
        "error" => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
