//! `map-cohere-finish-reason.ts`.

use aimux_core::types::{FinishReason, FinishReasonUnified};

/// Parse Cohere finish reason string into `FinishReason`.
#[must_use]
pub fn parse_finish_reason(s: &str) -> FinishReason {
    let unified = match s {
        "COMPLETE" | "STOP_SEQUENCE" => FinishReasonUnified::Stop,
        "MAX_TOKENS" => FinishReasonUnified::Length,
        "ERROR" => FinishReasonUnified::Error,
        "TOOL_CALL" => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
