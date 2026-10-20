//! DeepSeek finish reasons (`map-deepseek-finish-reason.ts`).

use aimux_core::types::FinishReasonUnified;

/// The unified finish reason of a DeepSeek `finish_reason`.
pub(crate) fn map_deepseek_finish_reason(finish_reason: Option<&str>) -> FinishReasonUnified {
    match finish_reason {
        Some("stop") => FinishReasonUnified::Stop,
        Some("length") => FinishReasonUnified::Length,
        Some("content_filter") => FinishReasonUnified::ContentFilter,
        Some("tool_calls") => FinishReasonUnified::ToolCalls,
        Some("insufficient_system_resource") => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    }
}
