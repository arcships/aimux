//! `map-google-finish-reason.ts`.

use aimux_core::types::{FinishReason, FinishReasonUnified};

/// Map a Gemini `finishReason` string to the unified `FinishReason`.
///
/// `STOP` maps to `ToolCalls` when `has_tool_calls` is true (mirroring the
/// TS `mapGoogleFinishReason`).
#[must_use]
pub fn parse_finish_reason(reason: &str, has_tool_calls: bool) -> FinishReason {
    let unified = match reason {
        "STOP" => {
            if has_tool_calls {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Stop
            }
        }
        "MAX_TOKENS" => FinishReasonUnified::Length,
        "IMAGE_SAFETY" | "RECITATION" | "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            FinishReasonUnified::ContentFilter
        }
        "MALFORMED_FUNCTION_CALL" => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(reason.to_string()),
    }
}
