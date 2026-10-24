//! Map a Bedrock stop reason to the finish reason (map-amazon-bedrock-finish-reason.ts).

use aimux_core::types::{FinishReason, FinishReasonUnified};

/// Map a Bedrock `stopReason` to the unified `FinishReason`.
#[must_use]
pub fn map_finish_reason(reason: &str) -> FinishReason {
    let unified = match reason {
        "stop_sequence" | "end_turn" | "stop" => FinishReasonUnified::Stop,
        "max_tokens" | "length" => FinishReasonUnified::Length,
        "content_filtered" | "content-filter" | "guardrail_intervened" => {
            FinishReasonUnified::ContentFilter
        }
        "tool_use" | "tool-calls" => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(reason.to_string()),
    }
}
