//! Ports of the `convertMistralUsage` cases of the upstream
//! `@ai-sdk/mistral` `convert-mistral-usage.test.ts`.
//!
//! The upstream `toEqual` ignores `undefined` properties, and `Usage`
//! serializes its `None` fields as absent, so the usage is compared as JSON.

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
