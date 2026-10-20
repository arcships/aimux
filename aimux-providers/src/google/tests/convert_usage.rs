use serde_json::{Value, json};

use crate::google::convert::convert_usage;
use crate::google::types::GoogleUsageMetadata;

/// `convertGoogleUsage(usageMetadata)` as the upstream JSON object.
fn convert_usage_json(usage_metadata: Value) -> Value {
    let usage: GoogleUsageMetadata =
        serde_json::from_value(usage_metadata).expect("usage metadata deserializes");
    serde_json::to_value(convert_usage(&usage)).expect("usage serializes")
}

mod convert_google_usage {
    use super::*;

    /// upstream: @ai-sdk/google src/convert-google-usage.test.ts › convertGoogleUsage › includes tool-use prompt tokens in input usage
    #[test]
    fn includes_tool_use_prompt_tokens_in_input_usage() {
        assert_eq!(
            convert_usage_json(json!({
                "promptTokenCount": 55,
                "toolUsePromptTokenCount": 89,
                "candidatesTokenCount": 51,
                "thoughtsTokenCount": 56,
                "totalTokenCount": 251,
            })),
            json!({
                "inputTokens": {
                    "total": 144,
                    "noCache": 144,
                    "cacheRead": 0,
                },
                "outputTokens": {
                    "total": 107,
                    "text": 51,
                    "reasoning": 56,
                },
                "raw": {
                    "promptTokenCount": 55,
                    "toolUsePromptTokenCount": 89,
                    "candidatesTokenCount": 51,
                    "thoughtsTokenCount": 56,
                    "totalTokenCount": 251,
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/convert-google-usage.test.ts › convertGoogleUsage › includes tool-use prompt tokens when calculating uncached input
    #[test]
    fn includes_tool_use_prompt_tokens_when_calculating_uncached_input() {
        assert_eq!(
            convert_usage_json(json!({
                "promptTokenCount": 55,
                "toolUsePromptTokenCount": 89,
                "cachedContentTokenCount": 100,
            }))["inputTokens"],
            json!({
                "total": 144,
                "noCache": 44,
                "cacheRead": 100,
            })
        );
    }
}
