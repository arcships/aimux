//! `map-xai-responses-finish-reason.ts`.

use aimux_core::types::FinishReasonUnified;

#[must_use]
pub fn map_xai_responses_finish_reason(s: &str) -> FinishReasonUnified {
    match s {
        "stop" | "completed" => FinishReasonUnified::Stop,
        "length" | "max_output_tokens" => FinishReasonUnified::Length,
        "tool_calls" | "function_call" => FinishReasonUnified::ToolCalls,
        "content_filter" => FinishReasonUnified::ContentFilter,
        _ => FinishReasonUnified::Other,
    }
}
