use serde_json::{Value, json};

use crate::openai_compatible::chat::usage_from_raw;

/// `convertOpenAICompatibleChatUsage`: the Rust form takes the raw `usage`
/// value (absent for `undefined`); the result is compared in its JSON form,
/// where an absent count is an `undefined` key.
fn convert_open_ai_compatible_chat_usage(usage: Option<&Value>) -> Value {
    serde_json::to_value(usage_from_raw(usage)).expect("usage serializes")
}

mod convert_open_ai_compatible_chat_usage {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-openai-compatible-chat-usage.test.ts › convertOpenAICompatibleChatUsage › returns null usage when usage is missing
    #[test]
    fn returns_null_usage_when_usage_is_missing() {
        // createNullLanguageModelUsage(): every count undefined, no raw.
        assert_eq!(
            convert_open_ai_compatible_chat_usage(None),
            json!({ "inputTokens": {}, "outputTokens": {} })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-openai-compatible-chat-usage.test.ts › convertOpenAICompatibleChatUsage › splits completion tokens into text and reasoning
    #[test]
    fn splits_completion_tokens_into_text_and_reasoning() {
        let usage = json!({
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "completion_tokens_details": { "reasoning_tokens": 5 },
        });
        assert_eq!(
            convert_open_ai_compatible_chat_usage(Some(&usage)),
            json!({
                "inputTokens": {
                    "total": 10,
                    "noCache": 10,
                    "cacheRead": 0,
                },
                "outputTokens": { "total": 20, "text": 15, "reasoning": 5 },
                "raw": {
                    "prompt_tokens": 10,
                    "completion_tokens": 20,
                    "completion_tokens_details": { "reasoning_tokens": 5 },
                },
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-openai-compatible-chat-usage.test.ts › convertOpenAICompatibleChatUsage › clamps text tokens at 0 when reasoning exceeds completion
    #[test]
    fn clamps_text_tokens_at_0_when_reasoning_exceeds_completion() {
        // Provider-inconsistent usage (Baseten Kimi-K3, finish_reason 'length'):
        // completion_tokens undercounts the actual generation, so
        // completion_tokens_details.reasoning_tokens > completion_tokens.
        let usage = json!({
            "prompt_tokens": 951,
            "completion_tokens": 6000,
            "total_tokens": 6952,
            "prompt_tokens_details": { "cached_tokens": 60 },
            "completion_tokens_details": { "reasoning_tokens": 6001 },
        });
        assert_eq!(
            convert_open_ai_compatible_chat_usage(Some(&usage)),
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
