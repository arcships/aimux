//! Mirrors `map-groq-finish-reason.ts`.

use aimux_core::types::FinishReasonUnified;

/// Map Groq's `finish_reason` to the unified finish reason.
pub(crate) fn map_groq_finish_reason(finish_reason: Option<&str>) -> FinishReasonUnified {
    match finish_reason {
        Some("stop") => FinishReasonUnified::Stop,
        Some("length") => FinishReasonUnified::Length,
        Some("content_filter") => FinishReasonUnified::ContentFilter,
        Some("function_call" | "tool_calls") => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    }
}
