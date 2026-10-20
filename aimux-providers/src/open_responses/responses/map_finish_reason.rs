//! responses/map-open-responses-finish-reason.ts.

use aimux_core::types::FinishReasonUnified;

// == Finish reason mapping ==

/// Map an Open Responses finish reason to the unified enum.
///
/// Mirrors the TS `mapOpenResponsesFinishReason`.
#[must_use]
pub fn map_open_responses_finish_reason(
    finish_reason: Option<&str>,
    has_tool_calls: bool,
) -> FinishReasonUnified {
    match finish_reason {
        None => {
            if has_tool_calls {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Stop
            }
        }
        Some("max_output_tokens") => FinishReasonUnified::Length,
        Some("content_filter") => FinishReasonUnified::ContentFilter,
        Some(_) => {
            if has_tool_calls {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Other
            }
        }
    }
}
