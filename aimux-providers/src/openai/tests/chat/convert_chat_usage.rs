use serde_json::{Value, json};

use crate::openai::chat::chat_api::UsageResponse;
use crate::openai::chat::convert_chat_usage::convert_usage;

/// `convertOpenAIChatUsage(usage)`: the wire usage parsed as aimux parses it,
/// with the same object kept as `raw`.
fn convert_openai_chat_usage(usage: &Value) -> Value {
    let parsed: UsageResponse = serde_json::from_value(usage.clone()).unwrap();
    serde_json::to_value(convert_usage(&parsed, Some(usage))).unwrap()
}

mod convert_open_ai_chat_usage {
    use super::*;

    /// upstream: @ai-sdk/openai src/chat/convert-openai-chat-usage.test.ts › convertOpenAIChatUsage › clamps text tokens at 0 when reasoning exceeds completion
    #[test]
    fn clamps_text_tokens_at_0_when_reasoning_exceeds_completion() {
        let usage = json!({
            "prompt_tokens": 951,
            "completion_tokens": 6000,
            "total_tokens": 6952,
            "prompt_tokens_details": { "cached_tokens": 60 },
            "completion_tokens_details": { "reasoning_tokens": 6001 },
        });

        assert_eq!(
            convert_openai_chat_usage(&usage),
            json!({
                "inputTokens": {
                    "total": 951,
                    "noCache": 891,
                    "cacheRead": 60,
                },
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
            assert_eq!(convert_openai_chat_usage(&usage), expected, "{usage}");
        }
    }
}
