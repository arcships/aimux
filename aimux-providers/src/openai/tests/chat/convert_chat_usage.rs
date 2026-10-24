use serde_json::{Value, json};

use crate::openai::model::convert_usage;
use crate::openai::types::UsageResponse;

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
}
