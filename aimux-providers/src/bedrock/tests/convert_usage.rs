//! `convertAmazonBedrockUsage(usage)` is `convert_usage`: the upstream usage
//! literal is parsed as the Converse `usage` object (`BedrockUsage`) and the
//! result is compared as JSON (keys whose value is `undefined` omitted). The
//! two `raw`-only cases are ported in
//! `aimux-providers/tests/bedrock_remaining_test.rs`.

use serde_json::{Value, json};

use crate::bedrock::api_types::BedrockUsage;
use crate::bedrock::convert_usage::convert_usage;

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

// ── Older ports (previously `tests/bedrock_convert_test.rs`) ──
//
// Pure-function tests for the Amazon Bedrock provider's conversion layer.
//
// Translated from the TS test files:
// - `convert-to-amazon-bedrock-chat-messages.test.ts` (64 cases)
// - `amazon-bedrock-prepare-tools.test.ts` (23 cases incl. `it.each`)
// - `convert-amazon-bedrock-usage.test.ts` (9 cases)

mod bedrock_convert {

    use crate::bedrock::api_types::BedrockUsage;

    use crate::bedrock::convert_usage::convert_usage;

    // ════════════════════════════════════════════════════════════════════════════
    // convert-amazon-bedrock-usage
    // ════════════════════════════════════════════════════════════════════════════
    //
    // The TS `convertAmazonBedrockUsage` echoes the input as `result.raw`.
    // The assertions below cover the `inputTokens`/`outputTokens` breakdown.

    fn bedrock_usage(
        input: u32,
        output: u32,
        cache_read: Option<u32>,
        cache_write: Option<u32>,
    ) -> BedrockUsage {
        BedrockUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
            total_tokens: None,
            cache_read_input_tokens: cache_read,
            cache_write_input_tokens: cache_write,
            ..Default::default()
        }
    }

    /// partial: `should convert basic usage without cache tokens`; full port in src/bedrock/tests/convert_usage.rs.
    #[test]
    fn usage_basic_without_cache() {
        let u = convert_usage(Some(&bedrock_usage(100, 50, None, None)));
        assert_eq!(u.input_tokens.total, Some(100));
        assert_eq!(u.input_tokens.no_cache, Some(100));
        assert_eq!(u.input_tokens.cache_read, Some(0));
        assert_eq!(u.input_tokens.cache_write, Some(0));
        assert_eq!(u.output_tokens.total, Some(50));
        assert_eq!(u.output_tokens.text, Some(50));
    }

    /// partial: `should convert usage with cache read tokens`; full port in src/bedrock/tests/convert_usage.rs.
    #[test]
    fn usage_with_cache_read() {
        let u = convert_usage(Some(&bedrock_usage(100, 50, Some(80), None)));
        assert_eq!(u.input_tokens.total, Some(180));
        assert_eq!(u.input_tokens.no_cache, Some(100));
        assert_eq!(u.input_tokens.cache_read, Some(80));
        assert_eq!(u.input_tokens.cache_write, Some(0));
        assert_eq!(u.output_tokens.total, Some(50));
        assert_eq!(u.output_tokens.text, Some(50));
    }

    /// partial: `should convert usage with cache write tokens`; full port in src/bedrock/tests/convert_usage.rs.
    #[test]
    fn usage_with_cache_write() {
        let u = convert_usage(Some(&bedrock_usage(100, 50, None, Some(60))));
        assert_eq!(u.input_tokens.total, Some(160));
        assert_eq!(u.input_tokens.no_cache, Some(100));
        assert_eq!(u.input_tokens.cache_read, Some(0));
        assert_eq!(u.input_tokens.cache_write, Some(60));
        assert_eq!(u.output_tokens.total, Some(50));
        assert_eq!(u.output_tokens.text, Some(50));
    }

    /// partial: `should convert usage with both cache read and write tokens`; full port in src/bedrock/tests/convert_usage.rs.
    #[test]
    fn usage_with_both_cache() {
        let u = convert_usage(Some(&bedrock_usage(100, 50, Some(80), Some(60))));
        assert_eq!(u.input_tokens.total, Some(240));
        assert_eq!(u.input_tokens.no_cache, Some(100));
        assert_eq!(u.input_tokens.cache_read, Some(80));
        assert_eq!(u.input_tokens.cache_write, Some(60));
        assert_eq!(u.output_tokens.total, Some(50));
        assert_eq!(u.output_tokens.text, Some(50));
    }

    /// partial: `should handle null cache tokens`; full port in src/bedrock/tests/convert_usage.rs.
    #[test]
    fn usage_null_cache_tokens() {
        let u = convert_usage(Some(&bedrock_usage(100, 50, None, None)));
        assert_eq!(u.input_tokens.total, Some(100));
        assert_eq!(u.input_tokens.no_cache, Some(100));
        assert_eq!(u.input_tokens.cache_read, Some(0));
        assert_eq!(u.input_tokens.cache_write, Some(0));
        assert_eq!(u.output_tokens.total, Some(50));
        assert_eq!(u.output_tokens.text, Some(50));
    }

    /// partial: `should handle null usage`; full port in src/bedrock/tests/convert_usage.rs.
    #[test]
    fn usage_null() {
        let u = convert_usage(None);
        assert_eq!(u.input_tokens.total, None);
        assert_eq!(u.input_tokens.no_cache, None);
        assert_eq!(u.input_tokens.cache_read, None);
        assert_eq!(u.input_tokens.cache_write, None);
        assert_eq!(u.output_tokens.total, None);
        assert_eq!(u.output_tokens.text, None);
    }

    /// partial: `should handle undefined usage`; full port in src/bedrock/tests/convert_usage.rs.
    #[test]
    fn usage_undefined() {
        let u = convert_usage(None);
        assert_eq!(u.input_tokens.total, None);
        assert_eq!(u.input_tokens.no_cache, None);
        assert_eq!(u.input_tokens.cache_read, None);
        assert_eq!(u.input_tokens.cache_write, None);
        assert_eq!(u.output_tokens.total, None);
        assert_eq!(u.output_tokens.text, None);
    }
}

// ── Older ports (previously `tests/bedrock_remaining_test.rs`) ──
//
// Remaining Amazon Bedrock upstream tests, restored from the tool-types branch.
//
// Pinned translation sources (`amazon-bedrock/src/`):
// - `normalize-tool-call-id.test.ts`
// - `convert-amazon-bedrock-usage.test.ts` (raw echo)
// - `amazon-bedrock-provider.test.ts` (factory/auth variants)
// - `convert-to-amazon-bedrock-chat-messages.test.ts` (Mistral IDs)
// - `amazon-bedrock-chat-language-model.test.ts` (request/response cases)

mod bedrock_remaining {
    use crate::bedrock::tests::remaining_support::*;

    use serde_json::{Value, json};

    use crate::bedrock::api_types::BedrockUsage;
    use crate::bedrock::convert_usage::convert_usage;

    mod convert_amazon_bedrock_usage {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should include totalTokens in raw when provided
        #[test]
        fn usage_raw_includes_total_tokens() {
            let raw = bedrock_usage_with_total(100, 50, Some(150), None, None);
            let u = convert_usage(Some(&raw));
            assert_eq!(
                u.raw,
                Some(
                    json!({"inputTokens":100,"outputTokens":50,"totalTokens":150})
                        .as_object()
                        .unwrap()
                        .clone()
                )
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/convert-amazon-bedrock-usage.test.ts › convertAmazonBedrockUsage › should preserve raw usage data
        #[test]
        fn usage_raw_preserved() {
            let raw = bedrock_usage_with_total(100, 50, Some(150), Some(80), Some(60));
            let u = convert_usage(Some(&raw));
            assert_eq!(u.raw, Some(json!({"inputTokens":100,"outputTokens":50,"totalTokens":150,"cacheReadInputTokens":80,"cacheWriteInputTokens":60}).as_object().unwrap().clone()));
        }
    }

    /// TS: "should handle null cache tokens" — `raw` is the usage object as
    /// sent, explicit `null`s and unknown keys included.
    #[test]
    fn usage_raw_keeps_null_and_unknown_keys() {
        let sent = json!({
            "inputTokens": 100,
            "outputTokens": 50,
            "cacheReadInputTokens": null,
            "cacheWriteInputTokens": null,
            "serverToolUsage": null
        });
        let usage: BedrockUsage = serde_json::from_value(sent.clone()).unwrap();
        let u = convert_usage(Some(&usage));
        assert_eq!(u.raw.map(Value::Object), Some(sent));
        assert_eq!(u.input_tokens.cache_read, Some(0));
    }
}
