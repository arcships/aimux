//! Upstream `convert-groq-usage.test.ts` (the cases that are not in the
//! integration tests).
//!
//! `undefined` in the upstream expectations is an absent key in the serialized
//! `Usage`, since serde skips `None`; the expected JSON therefore omits it.

use serde_json::{Value, json};

use super::*;

/// `convertGroqUsage(usage)` for a Groq usage object, as its serialized form.
fn convert_json(usage: Value) -> Value {
    let usage: GroqUsage = serde_json::from_value(usage).expect("valid Groq usage");
    serde_json::to_value(convert_groq_usage(Some(&usage))).expect("usage serializes")
}

mod convert_groq_usage {
    use super::*;

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should return undefined values when usage is null
    #[test]
    fn should_return_undefined_values_when_usage_is_null() {
        let result = convert_groq_usage(None);

        assert_eq!(
            serde_json::to_value(result).expect("usage serializes"),
            json!({ "inputTokens": {}, "outputTokens": {} })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should return undefined values when usage is undefined
    #[test]
    fn should_return_undefined_values_when_usage_is_undefined() {
        // An absent usage object is `None`, the same as `null` above.
        let result = convert_groq_usage(None);

        assert_eq!(
            serde_json::to_value(result).expect("usage serializes"),
            json!({ "inputTokens": {}, "outputTokens": {} })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle null reasoning_tokens in completion_tokens_details
    #[test]
    fn should_handle_null_reasoning_tokens_in_completion_tokens_details() {
        let result = convert_json(json!({
            "prompt_tokens": 20,
            "completion_tokens": 10,
            "completion_tokens_details": {
                "reasoning_tokens": null,
            },
        }));

        assert_eq!(
            result,
            json!({
                "inputTokens": {
                    "total": 20,
                    "noCache": 20,
                },
                "outputTokens": {
                    "total": 10,
                    "text": 10,
                },
                "raw": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "completion_tokens_details": {
                        "reasoning_tokens": null,
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle null completion_tokens_details
    #[test]
    fn should_handle_null_completion_tokens_details() {
        let result = convert_json(json!({
            "prompt_tokens": 20,
            "completion_tokens": 10,
            "completion_tokens_details": null,
        }));

        assert_eq!(
            result,
            json!({
                "inputTokens": {
                    "total": 20,
                    "noCache": 20,
                },
                "outputTokens": {
                    "total": 10,
                    "text": 10,
                },
                "raw": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "completion_tokens_details": null,
                },
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › clamps text tokens at 0 when reasoning exceeds completion
    #[test]
    fn clamps_text_tokens_at_0_when_reasoning_exceeds_completion() {
        let result = convert_json(json!({
            "prompt_tokens": 951,
            "completion_tokens": 6000,
            "completion_tokens_details": {
                "reasoning_tokens": 6001,
            },
        }));

        assert_eq!(
            result["outputTokens"],
            json!({
                "total": 6000,
                "text": 0,
                "reasoning": 6001,
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should leave cacheRead undefined when cached_tokens is null
    #[test]
    fn should_leave_cache_read_undefined_when_cached_tokens_is_null() {
        let result = convert_json(json!({
            "prompt_tokens": 20,
            "completion_tokens": 10,
            "prompt_tokens_details": {
                "cached_tokens": null,
            },
        }));

        assert_eq!(
            result["inputTokens"],
            json!({
                "total": 20,
                "noCache": 20,
            })
        );
    }
}
