//! `mapAnthropicStopReason` (`map-anthropic-stop-reason.ts`).

use aimux_core::types::{FinishReason, FinishReasonUnified};

/// Parse Anthropic stop_reason into `FinishReason`.
#[must_use]
pub fn parse_stop_reason(s: &str) -> FinishReason {
    let unified = match s {
        "end_turn" | "stop_sequence" | "pause_turn" => FinishReasonUnified::Stop,
        "max_tokens" | "model_context_window_exceeded" => FinishReasonUnified::Length,
        "refusal" => FinishReasonUnified::ContentFilter,
        "tool_use" => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
