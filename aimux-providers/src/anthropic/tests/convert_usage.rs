use serde_json::{Value, json};

use crate::anthropic::convert_usage::{
    AnthropicInputTokens, AnthropicOutputTokens, AnthropicUsageResult, convert_anthropic_usage,
};

/// `convertAnthropicUsage({ usage, rawUsage })`.
fn convert(usage: &Value, raw_usage: Option<&Value>) -> AnthropicUsageResult {
    convert_anthropic_usage(usage, raw_usage)
}

mod convert_anthropic_usage {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › should use usage as raw when rawUsage is not provided
    #[test]
    fn should_use_usage_as_raw_when_raw_usage_is_not_provided() {
        let usage = json!({
            "input_tokens": 10,
            "output_tokens": 20,
        });

        let result = convert(&usage, None);

        assert_eq!(result.raw, usage);
    }

    /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › should use rawUsage as raw when provided
    #[test]
    fn should_use_raw_usage_as_raw_when_provided() {
        let usage = json!({
            "input_tokens": 10,
            "output_tokens": 20,
        });
        let raw_usage = json!({
            "input_tokens": 10,
            "output_tokens": 20,
            "service_tier": "standard",
            "inference_geo": "not_available",
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 0,
            },
        });

        let result = convert(&usage, Some(&raw_usage));

        assert_eq!(result.raw, raw_usage);
    }

    /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › should compute token totals correctly with cache tokens
    #[test]
    fn should_compute_token_totals_correctly_with_cache_tokens() {
        let result = convert(
            &json!({
                "input_tokens": 10,
                "output_tokens": 20,
                "cache_creation_input_tokens": 5,
                "cache_read_input_tokens": 3,
            }),
            None,
        );

        assert_eq!(
            result.input_tokens,
            AnthropicInputTokens {
                total: 18,
                no_cache: 10,
                cache_read: 3,
                cache_write: 5,
            }
        );
        assert_eq!(
            result.output_tokens,
            AnthropicOutputTokens {
                total: 20,
                text: None,
                reasoning: None,
            }
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › should handle null cache tokens
    #[test]
    fn should_handle_null_cache_tokens() {
        let usage = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_creation_input_tokens": null,
            "cache_read_input_tokens": null,
        });

        let result = convert(&usage, None);

        assert_eq!(result.input_tokens.total, 100);
        assert_eq!(result.input_tokens.cache_read, 0);
        assert_eq!(result.input_tokens.cache_write, 0);
    }

    mod compaction_usage_with_iterations {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › compaction usage with iterations › should sum across all iterations when iterations array is present
        #[test]
        fn should_sum_across_all_iterations_when_iterations_array_is_present() {
            let usage = json!({
                "input_tokens": 45000,
                "output_tokens": 1234,
                "iterations": [
                    { "type": "compaction", "input_tokens": 180000, "output_tokens": 3500 },
                    { "type": "message", "input_tokens": 23000, "output_tokens": 1000 },
                ],
            });

            let result = convert(&usage, None);

            // Total should be sum of iterations, not top-level values
            assert_eq!(result.input_tokens.total, 203000); // 180000 + 23000
            assert_eq!(result.input_tokens.no_cache, 203000);
            assert_eq!(result.output_tokens.total, 4500); // 3500 + 1000
            assert_eq!(result.raw, usage);
        }

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › compaction usage with iterations › should handle single iteration (message only, no compaction triggered)
        #[test]
        fn should_handle_single_iteration_message_only_no_compaction_triggered() {
            let usage = json!({
                "input_tokens": 5000,
                "output_tokens": 500,
                "iterations": [
                    { "type": "message", "input_tokens": 5000, "output_tokens": 500 },
                ],
            });

            let result = convert(&usage, None);

            assert_eq!(result.input_tokens.total, 5000);
            assert_eq!(result.output_tokens.total, 500);
        }

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › compaction usage with iterations › should handle multiple compaction iterations (long-running task)
        #[test]
        fn should_handle_multiple_compaction_iterations_long_running_task() {
            let usage = json!({
                "input_tokens": 10000,
                "output_tokens": 500,
                "iterations": [
                    { "type": "compaction", "input_tokens": 200000, "output_tokens": 4000 },
                    { "type": "message", "input_tokens": 50000, "output_tokens": 2000 },
                    { "type": "compaction", "input_tokens": 180000, "output_tokens": 3500 },
                    { "type": "message", "input_tokens": 30000, "output_tokens": 1500 },
                ],
            });

            let result = convert(&usage, None);

            // Total = 200000 + 50000 + 180000 + 30000 = 460000 input
            // Total = 4000 + 2000 + 3500 + 1500 = 11000 output
            assert_eq!(result.input_tokens.total, 460000);
            assert_eq!(result.output_tokens.total, 11000);
        }

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › compaction usage with iterations › should combine iterations with cache tokens
        #[test]
        fn should_combine_iterations_with_cache_tokens() {
            let usage = json!({
                "input_tokens": 45000,
                "output_tokens": 1234,
                "cache_creation_input_tokens": 1000,
                "cache_read_input_tokens": 500,
                "iterations": [
                    { "type": "compaction", "input_tokens": 180000, "output_tokens": 3500 },
                    { "type": "message", "input_tokens": 23000, "output_tokens": 1000 },
                ],
            });

            let result = convert(&usage, None);

            // Total input = sum of iterations + cache tokens
            // noCache = sum of iterations only
            assert_eq!(result.input_tokens.no_cache, 203000); // 180000 + 23000
            assert_eq!(result.input_tokens.cache_write, 1000);
            assert_eq!(result.input_tokens.cache_read, 500);
            assert_eq!(result.input_tokens.total, 204500); // 203000 + 1000 + 500
            assert_eq!(result.output_tokens.total, 4500); // 3500 + 1000
        }

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › compaction usage with iterations › should use rawUsage as raw even when iterations are present
        #[test]
        fn should_use_raw_usage_as_raw_even_when_iterations_are_present() {
            let usage = json!({
                "input_tokens": 45000,
                "output_tokens": 1234,
                "iterations": [
                    { "type": "compaction", "input_tokens": 180000, "output_tokens": 3500 },
                    { "type": "message", "input_tokens": 23000, "output_tokens": 1000 },
                ],
            });
            let raw_usage = json!({
                "input_tokens": 45000,
                "output_tokens": 1234,
                "service_tier": "standard",
            });

            let result = convert(&usage, Some(&raw_usage));

            assert_eq!(result.raw, raw_usage);
            assert_eq!(result.input_tokens.total, 203000);
        }
    }

    mod edge_cases {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › edge cases › should use top-level values when iterations is null
        #[test]
        fn should_use_top_level_values_when_iterations_is_null() {
            let usage = json!({
                "input_tokens": 100,
                "output_tokens": 50,
                "iterations": null,
            });

            let result = convert(&usage, None);

            assert_eq!(result.input_tokens.total, 100);
            assert_eq!(result.output_tokens.total, 50);
        }

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › edge cases › should use top-level values when iterations is undefined
        #[test]
        fn should_use_top_level_values_when_iterations_is_undefined() {
            let usage = json!({
                "input_tokens": 100,
                "output_tokens": 50,
            });

            let result = convert(&usage, None);

            assert_eq!(result.input_tokens.total, 100);
            assert_eq!(result.output_tokens.total, 50);
        }

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › edge cases › should use top-level values when iterations array is empty
        #[test]
        fn should_use_top_level_values_when_iterations_array_is_empty() {
            let usage = json!({
                "input_tokens": 100,
                "output_tokens": 50,
                "iterations": [],
            });

            let result = convert(&usage, None);

            assert_eq!(result.input_tokens.total, 100);
            assert_eq!(result.output_tokens.total, 50);
        }

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › edge cases › should handle zero tokens in iterations
        #[test]
        fn should_handle_zero_tokens_in_iterations() {
            let usage = json!({
                "input_tokens": 0,
                "output_tokens": 0,
                "iterations": [
                    { "type": "compaction", "input_tokens": 0, "output_tokens": 0 },
                    { "type": "message", "input_tokens": 0, "output_tokens": 0 },
                ],
            });

            let result = convert(&usage, None);

            assert_eq!(result.input_tokens.total, 0);
            assert_eq!(result.output_tokens.total, 0);
        }
    }

    mod real_world_scenarios_from_documentation {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › real-world scenarios from documentation › should match documentation example exactly
        #[test]
        fn should_match_documentation_example_exactly() {
            let usage = json!({
                "input_tokens": 45000,
                "output_tokens": 1234,
                "iterations": [
                    { "type": "compaction", "input_tokens": 180000, "output_tokens": 3500 },
                    { "type": "message", "input_tokens": 23000, "output_tokens": 1000 },
                ],
            });

            let result = convert(&usage, None);

            let expected_total_input = 180000 + 23000; // 203000
            let expected_total_output = 3500 + 1000; // 4500

            assert_eq!(result.input_tokens.total, expected_total_input);
            assert_eq!(result.output_tokens.total, expected_total_output);

            // The top-level values (45000, 1234) are NOT the billed amounts
            // when iterations is present
            assert_ne!(
                Some(result.input_tokens.total),
                usage["input_tokens"].as_u64()
            );
            assert_ne!(
                Some(result.output_tokens.total),
                usage["output_tokens"].as_u64()
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-anthropic-usage.test.ts › convertAnthropicUsage › real-world scenarios from documentation › should handle re-applying previous compaction block (no new compaction)
        #[test]
        fn should_handle_re_applying_previous_compaction_block_no_new_compaction() {
            // No iterations - previous compaction block was re-applied
            let usage = json!({
                "input_tokens": 15000,
                "output_tokens": 800,
            });

            let result = convert(&usage, None);

            // Top-level values are accurate when no new compaction triggered
            assert_eq!(result.input_tokens.total, 15000);
            assert_eq!(result.output_tokens.total, 800);
        }
    }
}
