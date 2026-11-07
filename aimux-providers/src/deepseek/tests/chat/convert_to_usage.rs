//! Port of `src/chat/convert-to-deepseek-usage.test.ts` (`@ai-sdk/deepseek` 3.0.56).

use serde_json::{Value, json};

use crate::deepseek::chat::convert_to_usage::convert_deepseek_usage;

mod convert_deep_seek_usage {
    use super::*;

    /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-usage.test.ts › convertDeepSeekUsage › clamps text tokens at 0 when reasoning exceeds completion
    #[test]
    fn clamps_text_tokens_at_0_when_reasoning_exceeds_completion() {
        let usage: Value = json!({
            "prompt_tokens": 951,
            "completion_tokens": 6000,
            "completion_tokens_details": { "reasoning_tokens": 6001 },
        });

        let converted =
            serde_json::to_value(convert_deepseek_usage(Some(&usage))).expect("usage serializes");

        // `cacheWrite` is undefined upstream, so it is absent from the JSON.
        assert_eq!(
            converted,
            json!({
                // #478: an unreported cache read count stays unknown (the AI SDK fills 0).
                "inputTokens": { "total": 951, "noCache": 951 },
                "outputTokens": { "total": 6000, "text": 0, "reasoning": 6001 },
                "raw": usage,
            })
        );
    }

    #[test]
    fn keeps_unreported_counts_unknown() {
        // #478 (beyond the AI SDK): a count the provider did not report, absent
        // or null, stays unknown; a reported zero stays zero.
        for (usage, mut expected) in [
            (json!({}), json!({ "inputTokens": {}, "outputTokens": {} })),
            (
                json!({ "prompt_tokens": 2, "completion_tokens": null }),
                json!({ "inputTokens": { "total": 2, "noCache": 2 }, "outputTokens": {} }),
            ),
            (
                json!({ "completion_tokens": 0 }),
                json!({ "inputTokens": {}, "outputTokens": { "total": 0, "text": 0 } }),
            ),
        ] {
            expected["raw"] = usage.clone();
            assert_eq!(
                serde_json::to_value(convert_deepseek_usage(Some(&usage))).unwrap(),
                expected,
                "{usage}"
            );
        }
    }
}
