//! The finish reason of a chat or completion response
//! (`map-openai-compatible-finish-reason.ts`; the completion package's copy is
//! the same function).

use aimux_core::types::{FinishReason, FinishReasonUnified};

/// Map an OpenAI finish reason string to the unified one, keeping the raw.
#[must_use]
pub(crate) fn parse_finish_reason(raw: &str) -> FinishReason {
    let unified = match raw {
        "stop" => FinishReasonUnified::Stop,
        "length" => FinishReasonUnified::Length,
        "tool_calls" | "function_call" => FinishReasonUnified::ToolCalls,
        "content_filter" => FinishReasonUnified::ContentFilter,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(raw.to_string()),
    }
}
