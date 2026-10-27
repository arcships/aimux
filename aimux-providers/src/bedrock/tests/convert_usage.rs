//! `convertAmazonBedrockUsage(usage)` is `convert_usage`: the upstream usage
//! literal is parsed as the Converse `usage` object (`BedrockUsage`) and the
//! result is compared as JSON (keys whose value is `undefined` omitted). The
//! two `raw`-only cases are ported in
//! `aimux-providers/tests/bedrock_remaining_test.rs`.

use serde_json::{Value, json};

use crate::bedrock::convert::{BedrockUsage, convert_usage};

fn convert(usage: Option<Value>) -> Value {
    let usage: Option<BedrockUsage> =
        usage.map(|usage| serde_json::from_value(usage).expect("valid Bedrock usage"));
    serde_json::to_value(convert_usage(usage.as_ref())).unwrap()
}

mod convert_amazon_bedrock_usage {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should convert basic usage without cache tokens
    #[test]
    fn should_convert_basic_usage_without_cache_tokens() {
        let result = convert(Some(json!({ "inputTokens": 100, "outputTokens": 50 })));

        assert_eq!(
            result,
            json!({
                "inputTokens": { "total": 100, "noCache": 100, "cacheRead": 0, "cacheWrite": 0 },
                "outputTokens": { "total": 50, "text": 50 },
                "raw": { "inputTokens": 100, "outputTokens": 50 },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should convert usage with cache read tokens
    #[test]
    fn should_convert_usage_with_cache_read_tokens() {
        let result = convert(Some(json!({
            "inputTokens": 100,
            "outputTokens": 50,
            "cacheReadInputTokens": 80,
        })));

        assert_eq!(
            result,
            json!({
                // 100 + 80 + 0
                "inputTokens": { "total": 180, "noCache": 100, "cacheRead": 80, "cacheWrite": 0 },
                "outputTokens": { "total": 50, "text": 50 },
                "raw": { "inputTokens": 100, "outputTokens": 50, "cacheReadInputTokens": 80 },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should convert usage with cache write tokens
    #[test]
    fn should_convert_usage_with_cache_write_tokens() {
        let result = convert(Some(json!({
            "inputTokens": 100,
            "outputTokens": 50,
            "cacheWriteInputTokens": 60,
        })));

        assert_eq!(
            result,
            json!({
                // 100 + 0 + 60
                "inputTokens": { "total": 160, "noCache": 100, "cacheRead": 0, "cacheWrite": 60 },
                "outputTokens": { "total": 50, "text": 50 },
                "raw": { "inputTokens": 100, "outputTokens": 50, "cacheWriteInputTokens": 60 },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should convert usage with both cache read and write tokens
    #[test]
    fn should_convert_usage_with_both_cache_read_and_write_tokens() {
        let result = convert(Some(json!({
            "inputTokens": 100,
            "outputTokens": 50,
            "cacheReadInputTokens": 80,
            "cacheWriteInputTokens": 60,
        })));

        assert_eq!(
            result,
            json!({
                // 100 + 80 + 60
                "inputTokens": { "total": 240, "noCache": 100, "cacheRead": 80, "cacheWrite": 60 },
                "outputTokens": { "total": 50, "text": 50 },
                "raw": {
                    "inputTokens": 100,
                    "outputTokens": 50,
                    "cacheReadInputTokens": 80,
                    "cacheWriteInputTokens": 60,
                },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should handle null cache tokens
    #[test]
    fn should_handle_null_cache_tokens() {
        let result = convert(Some(json!({
            "inputTokens": 100,
            "outputTokens": 50,
            "cacheReadInputTokens": null,
            "cacheWriteInputTokens": null,
        })));

        assert_eq!(
            result,
            json!({
                "inputTokens": { "total": 100, "noCache": 100, "cacheRead": 0, "cacheWrite": 0 },
                "outputTokens": { "total": 50, "text": 50 },
                "raw": {
                    "inputTokens": 100,
                    "outputTokens": 50,
                    "cacheReadInputTokens": null,
                    "cacheWriteInputTokens": null,
                },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should handle null usage
    #[test]
    fn should_handle_null_usage() {
        // `null` and `undefined` are both `None` here.
        let result = convert(None);

        assert_eq!(result, json!({ "inputTokens": {}, "outputTokens": {} }));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should handle undefined usage
    #[test]
    fn should_handle_undefined_usage() {
        let result = convert(None);

        assert_eq!(result, json!({ "inputTokens": {}, "outputTokens": {} }));
    }
}
