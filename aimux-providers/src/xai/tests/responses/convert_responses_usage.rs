//! Upstream `responses/convert-xai-responses-usage.test.ts`.
//!
//! `undefined` in the upstream expectations is an absent key in the serialized
//! `Usage` (serde skips `None`), so the expected JSON omits it.

use aimux_core::types::Usage;
use serde_json::{Value, json};

use crate::xai::responses::convert::convert_xai_responses_usage;
use crate::xai::responses::types::XaiResponsesUsage;

/// `convertXaiResponsesUsage(usage)` for a usage object as the API returns it.
fn convert(usage: Value) -> Usage {
    let usage: XaiResponsesUsage = serde_json::from_value(usage).expect("valid xAI usage");
    convert_xai_responses_usage(&usage)
}

mod convert_xai_responses_usage {
    use super::*;

    /// upstream: @ai-sdk/xai src/responses/convert-xai-responses-usage.test.ts › convertXaiResponsesUsage › should convert basic usage without caching or reasoning
    #[test]
    fn should_convert_basic_usage_without_caching_or_reasoning() {
        let result = convert(json!({
            "input_tokens": 100,
            "output_tokens": 50,
        }));

        assert_eq!(
            serde_json::to_value(&result).expect("usage serializes"),
            json!({
                "inputTokens": {
                    "cacheRead": 0,
                    "noCache": 100,
                    "total": 100,
                },
                "outputTokens": {
                    "reasoning": 0,
                    "text": 50,
                    "total": 50,
                },
                "raw": {
                    "input_tokens": 100,
                    "output_tokens": 50,
                },
            })
        );
    }

    /// upstream: @ai-sdk/xai src/responses/convert-xai-responses-usage.test.ts › convertXaiResponsesUsage › should convert usage with reasoning tokens
    #[test]
    fn should_convert_usage_with_reasoning_tokens() {
        let result = convert(json!({
            "input_tokens": 1941,
            "output_tokens": 583,
            "total_tokens": 2524,
            "output_tokens_details": {
                "reasoning_tokens": 380,
            },
        }));

        assert_eq!(
            serde_json::to_value(&result.output_tokens).expect("usage serializes"),
            json!({
                "reasoning": 380,
                "text": 203,
                "total": 583,
            })
        );
    }

    /// upstream: @ai-sdk/xai src/responses/convert-xai-responses-usage.test.ts › convertXaiResponsesUsage › should convert usage with cached input tokens
    #[test]
    fn should_convert_usage_with_cached_input_tokens() {
        let result = convert(json!({
            "input_tokens": 200,
            "output_tokens": 50,
            "input_tokens_details": {
                "cached_tokens": 150,
            },
        }));

        assert_eq!(
            serde_json::to_value(&result.input_tokens).expect("usage serializes"),
            json!({
                "cacheRead": 150,
                "noCache": 50,
                "total": 200,
            })
        );
    }

    /// upstream: @ai-sdk/xai src/responses/convert-xai-responses-usage.test.ts › convertXaiResponsesUsage › should handle cached_tokens exceeding input_tokens (non-inclusive reporting)
    #[test]
    fn should_handle_cached_tokens_exceeding_input_tokens_non_inclusive_reporting() {
        let result = convert(json!({
            "input_tokens": 4142,
            "output_tokens": 254,
            "input_tokens_details": {
                "cached_tokens": 4328,
            },
        }));

        assert_eq!(
            serde_json::to_value(&result.input_tokens).expect("usage serializes"),
            json!({
                "cacheRead": 4328,
                "noCache": 4142,
                "total": 8470,
            })
        );
    }

    /// upstream: @ai-sdk/xai src/responses/convert-xai-responses-usage.test.ts › convertXaiResponsesUsage › should convert usage with both cached input and reasoning
    #[test]
    fn should_convert_usage_with_both_cached_input_and_reasoning() {
        let result = convert(json!({
            "input_tokens": 200,
            "output_tokens": 583,
            "input_tokens_details": {
                "cached_tokens": 150,
            },
            "output_tokens_details": {
                "reasoning_tokens": 380,
            },
        }));

        assert_eq!(
            serde_json::to_value(&result).expect("usage serializes"),
            json!({
                "inputTokens": {
                    "cacheRead": 150,
                    "noCache": 50,
                    "total": 200,
                },
                "outputTokens": {
                    "reasoning": 380,
                    "text": 203,
                    "total": 583,
                },
                "raw": {
                    "input_tokens": 200,
                    "input_tokens_details": {
                        "cached_tokens": 150,
                    },
                    "output_tokens": 583,
                    "output_tokens_details": {
                        "reasoning_tokens": 380,
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/xai src/responses/convert-xai-responses-usage.test.ts › convertXaiResponsesUsage › should preserve raw usage data
    #[test]
    fn should_preserve_raw_usage_data() {
        let raw_usage = json!({
            "input_tokens": 12,
            "output_tokens": 319,
            "total_tokens": 331,
            "input_tokens_details": {
                "cached_tokens": 2,
            },
            "output_tokens_details": {
                "reasoning_tokens": 317,
            },
        });

        let result = convert(raw_usage.clone());

        assert_eq!(result.raw, raw_usage.as_object().cloned());
    }
}
