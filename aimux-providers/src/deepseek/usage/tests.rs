//! Port of `src/chat/convert-to-deepseek-usage.test.ts` (`@ai-sdk/deepseek` 3.0.56).

use super::*;
use serde_json::{Value, json};

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
                "inputTokens": {
                    "total": 951,
                    "noCache": 951,
                    "cacheRead": 0,
                },
                "outputTokens": { "total": 6000, "text": 0, "reasoning": 6001 },
                "raw": usage,
            })
        );
    }
}
