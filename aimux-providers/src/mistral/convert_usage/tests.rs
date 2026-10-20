//! Ports of the upstream `@ai-sdk/mistral` `convert-mistral-usage.test.ts`:
//! the `mistralUsageSchema` and `convertMistralUsage` cases.
//!
//! The zod `safeParse` of the upstream schema is the `UsageResponse`
//! deserializer here: `raw` keeps the whole payload, and a wrong type in a
//! known field is a deserialization error. The upstream `toEqual` ignores
//! `undefined` properties, and `Usage` serializes its `None` fields as absent,
//! so the usage is compared as JSON.

use super::*;
use serde_json::json;

/// Parse a Mistral usage object, as the response body carries it.
fn usage_response(value: Value) -> UsageResponse {
    serde_json::from_value(value).expect("usage in the Mistral shape")
}

/// `convertMistralUsage(usage)` as a JSON object.
fn converted(value: Value) -> Value {
    serde_json::to_value(convert_usage(&usage_response(value))).expect("usage serializes")
}

mod convert_mistral_usage {
    use super::*;

    fn basic_usage() -> Value {
        json!({
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        })
    }

    /// upstream: @ai-sdk/mistral src/convert-mistral-usage.test.ts › convertMistralUsage › should map basic usage without cached tokens
    #[test]
    fn should_map_basic_usage_without_cached_tokens() {
        let usage = basic_usage();

        assert_eq!(
            converted(usage.clone()),
            json!({
                "inputTokens": {
                    "total": 100,
                    "noCache": 100
                },
                "outputTokens": {
                    "total": 50,
                    "text": 50
                },
                "raw": usage
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/convert-mistral-usage.test.ts › convertMistralUsage › should map cached tokens from num_cached_tokens
    #[test]
    fn should_map_cached_tokens_from_num_cached_tokens() {
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150,
            "num_cached_tokens": 60
        });

        assert_eq!(
            converted(usage.clone()),
            json!({
                "inputTokens": {
                    "total": 100,
                    "noCache": 40,
                    "cacheRead": 60
                },
                "outputTokens": {
                    "total": 50,
                    "text": 50
                },
                "raw": usage
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/convert-mistral-usage.test.ts › convertMistralUsage › should map cached tokens from prompt_tokens_details.cached_tokens
    #[test]
    fn should_map_cached_tokens_from_prompt_tokens_details_cached_tokens() {
        let usage = json!({
            "prompt_tokens": 200,
            "completion_tokens": 30,
            "total_tokens": 230,
            "prompt_tokens_details": { "cached_tokens": 80 }
        });

        assert_eq!(
            converted(usage.clone()),
            json!({
                "inputTokens": {
                    "total": 200,
                    "noCache": 120,
                    "cacheRead": 80
                },
                "outputTokens": {
                    "total": 30,
                    "text": 30
                },
                "raw": usage
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/convert-mistral-usage.test.ts › convertMistralUsage › should map cached tokens from prompt_token_details.cached_tokens
    #[test]
    fn should_map_cached_tokens_from_prompt_token_details_cached_tokens() {
        let usage = json!({
            "prompt_tokens": 150,
            "completion_tokens": 25,
            "total_tokens": 175,
            "prompt_token_details": { "cached_tokens": 50 }
        });

        assert_eq!(
            converted(usage.clone()),
            json!({
                "inputTokens": {
                    "total": 150,
                    "noCache": 100,
                    "cacheRead": 50
                },
                "outputTokens": {
                    "total": 25,
                    "text": 25
                },
                "raw": usage
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/convert-mistral-usage.test.ts › convertMistralUsage › should prefer num_cached_tokens over prompt_tokens_details
    #[test]
    fn should_prefer_num_cached_tokens_over_prompt_tokens_details() {
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "total_tokens": 120,
            "num_cached_tokens": 40,
            "prompt_tokens_details": { "cached_tokens": 30 }
        });

        assert_eq!(
            converted(usage.clone()),
            json!({
                "inputTokens": {
                    "total": 100,
                    "noCache": 60,
                    "cacheRead": 40
                },
                "outputTokens": {
                    "total": 20,
                    "text": 20
                },
                "raw": usage
            })
        );
    }
}

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
