//! Ports of the `mistralUsageSchema` cases of the upstream
//! `@ai-sdk/mistral` `convert-mistral-usage.test.ts`.
//!
//! The zod `safeParse` of the upstream schema is the `UsageResponse`
//! deserializer here: `raw` keeps the whole payload, and a wrong type in a
//! known field is a deserialization error.

use super::*;
use serde_json::json;

mod mistral_usage_schema {
    use super::*;

    fn basic_usage() -> Value {
        json!({
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        })
    }

    /// `basicUsage` with one field set to `value`, as the upstream spreads it.
    fn with_field(key: &str, value: Value) -> Value {
        let mut usage = basic_usage();
        usage[key] = value;
        usage
    }

    /// upstream: @ai-sdk/mistral src/convert-mistral-usage.test.ts › mistralUsageSchema › should preserve additional usage and token detail fields
    #[test]
    fn should_preserve_additional_usage_and_token_detail_fields() {
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150,
            "additional_usage_field": { "value": true },
            "prompt_tokens_details": {
                "cached_tokens": 20,
                "additional_prompt_field": ["value"]
            },
            "prompt_token_details": {
                "audio_tokens": 5,
                "additional_prompt_token_field": 1
            },
            "completion_tokens_details": {
                "reasoning_tokens": 10,
                "additional_completion_field": null
            }
        });

        let parsed: UsageResponse =
            serde_json::from_value(usage.clone()).expect("usage with extra fields parses");

        assert_eq!(parsed.raw, usage);
    }

    /// upstream: @ai-sdk/mistral src/convert-mistral-usage.test.ts › mistralUsageSchema › should reject an invalid %s value
    #[test]
    fn should_reject_an_invalid_s_value() {
        let cases: Vec<(&str, Value)> = vec![
            ("prompt_tokens", with_field("prompt_tokens", json!("100"))),
            (
                "completion_tokens",
                with_field("completion_tokens", json!("50")),
            ),
            ("total_tokens", with_field("total_tokens", json!("150"))),
            (
                "prompt_audio_seconds",
                with_field("prompt_audio_seconds", json!("10")),
            ),
            ("request_count", with_field("request_count", json!("1"))),
            ("service_tier", with_field("service_tier", json!(1))),
            (
                "num_cached_tokens",
                with_field("num_cached_tokens", json!("20")),
            ),
            (
                "prompt_tokens_details.cached_tokens",
                with_field("prompt_tokens_details", json!({ "cached_tokens": "20" })),
            ),
            (
                "prompt_tokens_details.audio_tokens",
                with_field("prompt_tokens_details", json!({ "audio_tokens": "5" })),
            ),
            (
                "prompt_tokens_details.messages",
                with_field("prompt_tokens_details", json!({ "messages": {} })),
            ),
            (
                "prompt_token_details.cached_tokens",
                with_field("prompt_token_details", json!({ "cached_tokens": "20" })),
            ),
            (
                "prompt_token_details.audio_tokens",
                with_field("prompt_token_details", json!({ "audio_tokens": "5" })),
            ),
            (
                "prompt_token_details.messages",
                with_field("prompt_token_details", json!({ "messages": {} })),
            ),
            (
                "completion_tokens_details.reasoning_tokens",
                with_field(
                    "completion_tokens_details",
                    json!({ "reasoning_tokens": "10" }),
                ),
            ),
        ];

        for (field, usage) in cases {
            assert!(
                serde_json::from_value::<UsageResponse>(usage).is_err(),
                "should reject an invalid {field} value"
            );
        }
    }
}
