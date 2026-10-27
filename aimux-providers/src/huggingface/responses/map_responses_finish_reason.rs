//! `map-huggingface-responses-finish-reason.ts`.

use aimux_core::types::FinishReasonUnified;

/// Map a Hugging Face finish-reason string to the unified enum.
pub(super) fn map_finish_reason(reason: &str) -> FinishReasonUnified {
    match reason {
        "stop" => FinishReasonUnified::Stop,
        "length" => FinishReasonUnified::Length,
        "content_filter" => FinishReasonUnified::ContentFilter,
        "tool_calls" => FinishReasonUnified::ToolCalls,
        "error" => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    }
}
