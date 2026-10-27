//! Map a Chat Completions finish reason to the core finish reason (map-openai-finish-reason.ts).

use aimux_core::types::{FinishReason, FinishReasonUnified};

/// Parse OpenAI finish reason string into `FinishReason`.
#[must_use]
pub fn parse_finish_reason(s: &str) -> FinishReason {
    let unified = match s {
        "stop" => FinishReasonUnified::Stop,
        "length" => FinishReasonUnified::Length,
        // Legacy `function_call` is a tool-call stop too
        // (upstream map-openai-finish-reason.ts:13-15).
        "function_call" | "tool_calls" => FinishReasonUnified::ToolCalls,
        "content_filter" => FinishReasonUnified::ContentFilter,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
