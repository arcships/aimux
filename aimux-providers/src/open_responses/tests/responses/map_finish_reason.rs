//! Port of `src/responses/map-open-responses-finish-reason.test.ts`.
//!
//! The 8 cases came from the older integration tests in
//! `tests/open_responses_test.rs` and carry their markers.

use crate::open_responses::map_open_responses_finish_reason;
use aimux_core::types::FinishReasonUnified;

mod map_open_responses_finish_reason {
    use super::*;

    /// upstream: @ai-sdk/open-responses src/responses/map-open-responses-finish-reason.test.ts › mapOpenResponsesFinishReason › should return tool-calls when hasToolCalls is true and finishReason is undefined
    #[test]
    fn tool_calls_when_has_tool_calls_and_finish_reason_undefined() {
        assert_eq!(
            map_open_responses_finish_reason(None, true),
            FinishReasonUnified::ToolCalls
        );
    }

    /// upstream: @ai-sdk/open-responses src/responses/map-open-responses-finish-reason.test.ts › mapOpenResponsesFinishReason › should return tool-calls when hasToolCalls is true and finishReason is null
    #[test]
    fn tool_calls_when_has_tool_calls_and_finish_reason_null() {
        // In Rust, null and undefined are both represented as None.
        assert_eq!(
            map_open_responses_finish_reason(None, true),
            FinishReasonUnified::ToolCalls
        );
    }

    /// upstream: @ai-sdk/open-responses src/responses/map-open-responses-finish-reason.test.ts › mapOpenResponsesFinishReason › should return stop when hasToolCalls is false and finishReason is undefined
    #[test]
    fn stop_when_no_tool_calls_and_finish_reason_undefined() {
        assert_eq!(
            map_open_responses_finish_reason(None, false),
            FinishReasonUnified::Stop
        );
    }

    /// upstream: @ai-sdk/open-responses src/responses/map-open-responses-finish-reason.test.ts › mapOpenResponsesFinishReason › should return stop when hasToolCalls is false and finishReason is null
    #[test]
    fn stop_when_no_tool_calls_and_finish_reason_null() {
        assert_eq!(
            map_open_responses_finish_reason(None, false),
            FinishReasonUnified::Stop
        );
    }

    /// upstream: @ai-sdk/open-responses src/responses/map-open-responses-finish-reason.test.ts › mapOpenResponsesFinishReason › should return length when finishReason is max_output_tokens
    #[test]
    fn length_when_finish_reason_max_output_tokens() {
        assert_eq!(
            map_open_responses_finish_reason(Some("max_output_tokens"), false),
            FinishReasonUnified::Length
        );
    }

    /// upstream: @ai-sdk/open-responses src/responses/map-open-responses-finish-reason.test.ts › mapOpenResponsesFinishReason › should return content-filter when finishReason is content_filter
    #[test]
    fn content_filter_when_finish_reason_content_filter() {
        assert_eq!(
            map_open_responses_finish_reason(Some("content_filter"), false),
            FinishReasonUnified::ContentFilter
        );
    }

    /// upstream: @ai-sdk/open-responses src/responses/map-open-responses-finish-reason.test.ts › mapOpenResponsesFinishReason › should return tool-calls when hasToolCalls is true and finishReason is unknown
    #[test]
    fn tool_calls_when_has_tool_calls_and_finish_reason_unknown() {
        assert_eq!(
            map_open_responses_finish_reason(Some("completed"), true),
            FinishReasonUnified::ToolCalls
        );
    }

    /// upstream: @ai-sdk/open-responses src/responses/map-open-responses-finish-reason.test.ts › mapOpenResponsesFinishReason › should return other when hasToolCalls is false and finishReason is unknown
    #[test]
    fn other_when_no_tool_calls_and_finish_reason_unknown() {
        assert_eq!(
            map_open_responses_finish_reason(Some("completed"), false),
            FinishReasonUnified::Other
        );
    }
}
