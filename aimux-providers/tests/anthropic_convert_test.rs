// Panic convert wrappers are #[deprecated]; these tests still use them.
#![allow(deprecated)]
//! Rust port of the Anthropic provider pure-function tests.
//!
//! Translated from the Vercel AI SDK TypeScript tests:
//! - `packages/anthropic/src/convert-anthropic-usage.test.ts` (15 cases)
//! - `packages/anthropic/src/anthropic-prepare-tools.test.ts` (function-tool +
//!   tool-choice subset)
//! - `packages/anthropic/src/convert-to-anthropic-prompt.test.ts` (subset
//!   supported by the current Rust data model)
//!
//! Tests that exercise features absent from the Rust `aimux-core` data model
//! (reasoning parts, cache control, URL images, provider-defined tools,
//! mid-conversation system messages, citations, ...) are intentionally omitted
//! and documented in the task summary rather than translated here.

use std::collections::HashMap;

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::tool::{FunctionTool, ToolChoice};
use aimux_core::types::Warning;
use aimux_providers::anthropic::convert::convert_prompt_to_anthropic;
use aimux_providers::anthropic::prepare_tools::prepare_tools;
use aimux_providers::anthropic::usage::{
    AnthropicInputTokens, AnthropicOutputTokens, convert_anthropic_usage,
};
use base64::Engine;
use serde_json::{Value, json};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn msg(role: Role, parts: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content: parts,
        provider_options: None,
    }
}

fn text_part(t: &str) -> ContentPart {
    ContentPart::text(t.to_string())
}

fn prompt(msgs: Vec<LanguageModelPromptMessage>) -> Vec<LanguageModelPromptMessage> {
    msgs
}

/// Build a function tool with the common defaults (no strict, no provider
/// options, no input examples).
fn ftool(name: &str, desc: &str, schema: Value) -> FunctionTool {
    FunctionTool::new(name.to_string(), schema).with_description(desc.to_string())
}

fn anthropic_opts(value: Value) -> Option<HashMap<String, Value>> {
    let mut map = HashMap::new();
    map.insert("anthropic".to_string(), value);
    Some(map)
}

/// Decode a base64 string into raw bytes (mirrors the TS `data: { type: 'data',
/// data: '<base64>' }` input form, which the Rust `ContentPart::Image` holds as
/// raw bytes).
fn decode_b64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .expect("valid base64")
}

// ═════════════════════════════════════════════════════════════════════════════
// convertAnthropicUsage (15 cases)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod convert_anthropic_usage_tests {
    use super::*;

    #[test]
    fn should_use_usage_as_raw_when_raw_usage_is_not_provided() {
        let usage = json!({ "input_tokens": 10, "output_tokens": 20 });
        let result = convert_anthropic_usage(&usage, None);
        assert_eq!(result.raw, usage);
    }

    #[test]
    fn should_use_raw_usage_as_raw_when_provided() {
        let usage = json!({ "input_tokens": 10, "output_tokens": 20 });
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
        let result = convert_anthropic_usage(&usage, Some(&raw_usage));
        assert_eq!(result.raw, raw_usage);
    }

    #[test]
    fn should_compute_token_totals_correctly_with_cache_tokens() {
        let usage = json!({
            "input_tokens": 10,
            "output_tokens": 20,
            "cache_creation_input_tokens": 5,
            "cache_read_input_tokens": 3,
        });
        let result = convert_anthropic_usage(&usage, None);
        assert_eq!(
            result.input_tokens,
            AnthropicInputTokens {
                total: 18,
                no_cache: 10,
                cache_read: 3,
                cache_write: 5
            }
        );
        assert_eq!(
            result.output_tokens,
            AnthropicOutputTokens {
                total: 20,
                text: None,
                reasoning: None
            }
        );
    }

    #[test]
    fn should_handle_null_cache_tokens() {
        let usage = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_creation_input_tokens": null,
            "cache_read_input_tokens": null,
        });
        let result = convert_anthropic_usage(&usage, None);
        assert_eq!(result.input_tokens.total, 100);
        assert_eq!(result.input_tokens.cache_read, 0);
        assert_eq!(result.input_tokens.cache_write, 0);
    }

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
        let result = convert_anthropic_usage(&usage, None);
        // Total should be sum of iterations, not top-level values.
        assert_eq!(result.input_tokens.total, 203000); // 180000 + 23000
        assert_eq!(result.input_tokens.no_cache, 203000);
        assert_eq!(result.output_tokens.total, 4500); // 3500 + 1000
        assert_eq!(result.raw, usage);
    }

    #[test]
    fn should_handle_single_iteration_message_only_no_compaction_triggered() {
        let usage = json!({
            "input_tokens": 5000,
            "output_tokens": 500,
            "iterations": [
                { "type": "message", "input_tokens": 5000, "output_tokens": 500 },
            ],
        });
        let result = convert_anthropic_usage(&usage, None);
        assert_eq!(result.input_tokens.total, 5000);
        assert_eq!(result.output_tokens.total, 500);
    }

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
        let result = convert_anthropic_usage(&usage, None);
        // Total = 200000 + 50000 + 180000 + 30000 = 460000 input
        assert_eq!(result.input_tokens.total, 460000);
        // Total = 4000 + 2000 + 3500 + 1500 = 11000 output
        assert_eq!(result.output_tokens.total, 11000);
    }

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
        let result = convert_anthropic_usage(&usage, None);
        // noCache = sum of iterations only
        assert_eq!(result.input_tokens.no_cache, 203000); // 180000 + 23000
        assert_eq!(result.input_tokens.cache_write, 1000);
        assert_eq!(result.input_tokens.cache_read, 500);
        assert_eq!(result.input_tokens.total, 204500); // 203000 + 1000 + 500
        assert_eq!(result.output_tokens.total, 4500); // 3500 + 1000
    }

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
        let result = convert_anthropic_usage(&usage, Some(&raw_usage));
        assert_eq!(result.raw, raw_usage);
        assert_eq!(result.input_tokens.total, 203000);
    }

    #[test]
    fn should_use_top_level_values_when_iterations_is_null() {
        let usage = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "iterations": null,
        });
        let result = convert_anthropic_usage(&usage, None);
        assert_eq!(result.input_tokens.total, 100);
        assert_eq!(result.output_tokens.total, 50);
    }

    #[test]
    fn should_use_top_level_values_when_iterations_is_undefined() {
        let usage = json!({ "input_tokens": 100, "output_tokens": 50 });
        let result = convert_anthropic_usage(&usage, None);
        assert_eq!(result.input_tokens.total, 100);
        assert_eq!(result.output_tokens.total, 50);
    }

    #[test]
    fn should_use_top_level_values_when_iterations_array_is_empty() {
        let usage = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "iterations": [],
        });
        let result = convert_anthropic_usage(&usage, None);
        assert_eq!(result.input_tokens.total, 100);
        assert_eq!(result.output_tokens.total, 50);
    }

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
        let result = convert_anthropic_usage(&usage, None);
        assert_eq!(result.input_tokens.total, 0);
        assert_eq!(result.output_tokens.total, 0);
    }

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
        let result = convert_anthropic_usage(&usage, None);
        let expected_total_input = 180000 + 23000; // 203000
        let expected_total_output = 3500 + 1000; // 4500
        assert_eq!(result.input_tokens.total, expected_total_input);
        assert_eq!(result.output_tokens.total, expected_total_output);
        // The top-level values (45000, 1234) are NOT the billed amounts when
        // iterations is present.
        assert_ne!(
            result.input_tokens.total,
            usage["input_tokens"].as_u64().unwrap()
        );
        assert_ne!(
            result.output_tokens.total,
            usage["output_tokens"].as_u64().unwrap()
        );
    }

    #[test]
    fn should_handle_re_applying_previous_compaction_block_no_new_compaction() {
        let usage = json!({
            "input_tokens": 15000,
            "output_tokens": 800,
        });
        let result = convert_anthropic_usage(&usage, None);
        // Top-level values are accurate when no new compaction triggered.
        assert_eq!(result.input_tokens.total, 15000);
        assert_eq!(result.output_tokens.total, 800);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// prepareTools (function-tool + tool-choice subset)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod prepare_tools_tests {
    use super::*;

    fn assert_strict_warning(warnings: &[Warning], tool_name: &str, strict_val: bool) {
        assert_eq!(warnings.len(), 1, "expected exactly one warning");
        match &warnings[0] {
            Warning::Unsupported { feature, details } => {
                assert_eq!(feature, "strict");
                let expected = format!(
                    "Tool '{}' has strict: {}, but strict mode is not supported by this provider. The strict property will be ignored.",
                    tool_name, strict_val
                );
                assert_eq!(details.as_ref().unwrap(), &expected);
            }
            other => panic!("expected Unsupported warning, got {:?}", other),
        }
    }

    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_null() {
        let result = prepare_tools(None, None, false, true, true, false);
        assert!(result.tools.is_none());
        assert!(result.tool_choice.is_none());
        assert!(result.tool_warnings.is_empty());
        assert!(result.betas.is_empty());
    }

    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_empty() {
        let tools: Vec<FunctionTool> = vec![];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);
        assert!(result.tools.is_none());
        assert!(result.tool_choice.is_none());
        assert!(result.tool_warnings.is_empty());
        assert!(result.betas.is_empty());
    }

    #[test]
    fn should_correctly_prepare_function_tools() {
        let mut tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        tool.provider_options = anthropic_opts(json!({ "eagerInputStreaming": true }));
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        let expected = json!({
            "name": "testFunction",
            "description": "A test function",
            "input_schema": { "type": "object", "properties": {} },
            "eager_input_streaming": true,
        });
        assert_eq!(result.tools.unwrap()[0], expected);
        assert!(result.tool_choice.is_none());
        assert!(result.tool_warnings.is_empty());
    }

    #[test]
    fn should_correctly_preserve_tool_input_examples() {
        let mut tool = ftool(
            "tool_with_examples",
            "tool with examples",
            json!({ "type": "object", "properties": { "a": { "type": "number" } } }),
        );
        tool.input_examples = Some(vec![
            json!({ "input": { "a": 1 } }),
            json!({ "input": { "a": 2 } }),
        ]);
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        assert!(result.betas.contains("structured-outputs-2025-11-13"));
        assert!(result.betas.contains("advanced-tool-use-2025-11-20"));
        assert!(result.tool_choice.is_none());
        assert!(result.tool_warnings.is_empty());

        let expected = json!({
            "name": "tool_with_examples",
            "description": "tool with examples",
            "input_schema": { "type": "object", "properties": { "a": { "type": "number" } } },
            "input_examples": [{ "a": 1 }, { "a": 2 }],
        });
        assert_eq!(result.tools.unwrap()[0], expected);
    }

    // ── strict mode for function tools ──

    #[test]
    fn strict_included_and_beta_when_supports_structured_output_and_strict_true() {
        let mut tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        tool.strict = Some(true);
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        let expected = json!({
            "name": "testFunction",
            "description": "A test function",
            "input_schema": { "type": "object", "properties": {} },
            "strict": true,
        });
        assert_eq!(result.tools.unwrap()[0], expected);
        assert!(result.betas.contains("structured-outputs-2025-11-13"));
        assert_eq!(result.betas.len(), 1);
        assert!(result.tool_warnings.is_empty());
    }

    #[test]
    fn beta_but_not_strict_when_strict_undefined_and_supports_structured_output() {
        let tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        let expected = json!({
            "name": "testFunction",
            "description": "A test function",
            "input_schema": { "type": "object", "properties": {} },
        });
        assert_eq!(result.tools.unwrap()[0], expected);
        assert!(result.betas.contains("structured-outputs-2025-11-13"));
        assert!(result.tool_warnings.is_empty());
    }

    #[test]
    fn no_strict_emit_warning_no_beta_when_both_supports_flags_false() {
        let mut tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        tool.strict = Some(true);
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, false, false, false);

        let expected = json!({
            "name": "testFunction",
            "description": "A test function",
            "input_schema": { "type": "object", "properties": {} },
        });
        assert_eq!(result.tools.unwrap()[0], expected);
        assert!(result.betas.is_empty());
        assert_strict_warning(&result.tool_warnings, "testFunction", true);
    }

    #[test]
    fn strict_but_no_beta_when_supports_structured_output_false_but_strict_tools_true() {
        let mut tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        tool.strict = Some(true);
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, false, true, false);

        let expected = json!({
            "name": "testFunction",
            "description": "A test function",
            "input_schema": { "type": "object", "properties": {} },
            "strict": true,
        });
        assert_eq!(result.tools.unwrap()[0], expected);
        assert!(result.betas.is_empty());
        assert!(result.tool_warnings.is_empty());
    }

    #[test]
    fn beta_when_strict_false_and_supports_structured_output_true() {
        let mut tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        tool.strict = Some(false);
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        let expected = json!({
            "name": "testFunction",
            "description": "A test function",
            "input_schema": { "type": "object", "properties": {} },
            "strict": false,
        });
        assert_eq!(result.tools.unwrap()[0], expected);
        assert!(result.betas.contains("structured-outputs-2025-11-13"));
        assert!(result.tool_warnings.is_empty());
    }

    // ── deferLoading for function tools ──

    #[test]
    fn should_include_defer_loading_when_set_to_true() {
        let mut tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        tool.provider_options = anthropic_opts(json!({ "deferLoading": true }));
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        let expected = json!({
            "name": "testFunction",
            "description": "A test function",
            "input_schema": { "type": "object", "properties": {} },
            "defer_loading": true,
        });
        assert_eq!(result.tools.unwrap()[0], expected);
        assert!(result.betas.contains("structured-outputs-2025-11-13"));
        assert_eq!(result.betas.len(), 1);
    }

    #[test]
    fn should_include_defer_loading_when_set_to_false() {
        let mut tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        tool.provider_options = anthropic_opts(json!({ "deferLoading": false }));
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        let expected = json!({
            "name": "testFunction",
            "description": "A test function",
            "input_schema": { "type": "object", "properties": {} },
            "defer_loading": false,
        });
        assert_eq!(result.tools.unwrap()[0], expected);
    }

    #[test]
    fn should_not_include_defer_loading_when_not_specified() {
        let tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);
        let tool_def = result.tools.unwrap()[0].as_object().unwrap().clone();
        assert!(!tool_def.contains_key("defer_loading"));
    }

    // ── allowedCallers for function tools ──

    #[test]
    fn should_include_allowed_callers_and_advanced_tool_use_beta_when_set() {
        let mut tool = ftool(
            "query_database",
            "Query a database",
            json!({ "type": "object", "properties": { "sql": { "type": "string" } } }),
        );
        tool.provider_options =
            anthropic_opts(json!({ "allowedCallers": ["code_execution_20250825"] }));
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        assert!(result.betas.contains("structured-outputs-2025-11-13"));
        assert!(result.betas.contains("advanced-tool-use-2025-11-20"));

        let expected = json!({
            "name": "query_database",
            "description": "Query a database",
            "input_schema": { "type": "object", "properties": { "sql": { "type": "string" } } },
            "allowed_callers": ["code_execution_20250825"],
        });
        assert_eq!(result.tools.unwrap()[0], expected);
    }

    #[test]
    fn should_not_include_allowed_callers_when_not_specified() {
        let tool = ftool(
            "testFunction",
            "A test function",
            json!({ "type": "object", "properties": {} }),
        );
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);
        let tool_def = result.tools.unwrap()[0].as_object().unwrap().clone();
        assert!(!tool_def.contains_key("allowed_callers"));
    }

    #[test]
    fn should_include_both_defer_loading_and_allowed_callers_when_both_set() {
        let mut tool = ftool(
            "query_database",
            "Query a database",
            json!({ "type": "object", "properties": { "sql": { "type": "string" } } }),
        );
        tool.provider_options = anthropic_opts(json!({
            "deferLoading": true,
            "allowedCallers": ["code_execution_20250825"],
        }));
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        let tool_def = result.tools.unwrap()[0].as_object().unwrap().clone();
        assert_eq!(tool_def.get("defer_loading"), Some(&json!(true)));
        assert_eq!(
            tool_def.get("allowed_callers"),
            Some(&json!(["code_execution_20250825"]))
        );
        assert!(result.betas.contains("advanced-tool-use-2025-11-20"));
    }

    #[test]
    fn should_include_allowed_callers_with_code_execution_20260120() {
        let mut tool = ftool(
            "query_database",
            "Query a database",
            json!({ "type": "object", "properties": { "sql": { "type": "string" } } }),
        );
        tool.provider_options =
            anthropic_opts(json!({ "allowedCallers": ["code_execution_20260120"] }));
        let tools = vec![tool];
        let result = prepare_tools(Some(&tools), None, false, true, true, false);

        assert!(result.betas.contains("structured-outputs-2025-11-13"));
        assert!(result.betas.contains("advanced-tool-use-2025-11-20"));

        let expected = json!({
            "name": "query_database",
            "description": "Query a database",
            "input_schema": { "type": "object", "properties": { "sql": { "type": "string" } } },
            "allowed_callers": ["code_execution_20260120"],
        });
        assert_eq!(result.tools.unwrap()[0], expected);
    }

    // ── tool choice ──

    #[test]
    fn should_handle_tool_choice_auto() {
        let tool = ftool("testFunction", "Test", json!({}));
        let tools = vec![tool];
        let tc = ToolChoice::Auto;
        let result = prepare_tools(Some(&tools), Some(&tc), false, true, true, false);
        assert_eq!(result.tool_choice.unwrap(), json!({ "type": "auto" }));
    }

    #[test]
    fn should_handle_tool_choice_required() {
        let tool = ftool("testFunction", "Test", json!({}));
        let tools = vec![tool];
        let tc = ToolChoice::Required;
        let result = prepare_tools(Some(&tools), Some(&tc), false, true, true, false);
        assert_eq!(result.tool_choice.unwrap(), json!({ "type": "any" }));
    }

    #[test]
    fn should_handle_tool_choice_none() {
        let tool = ftool("testFunction", "Test", json!({}));
        let tools = vec![tool];
        let tc = ToolChoice::None;
        let result = prepare_tools(Some(&tools), Some(&tc), false, true, true, false);
        assert!(result.tools.is_none());
        assert!(result.tool_choice.is_none());
    }

    #[test]
    fn should_handle_tool_choice_tool() {
        let tool = ftool("testFunction", "Test", json!({}));
        let tools = vec![tool];
        let tc = ToolChoice::Tool {
            tool_name: "testFunction".to_string(),
        };
        let result = prepare_tools(Some(&tools), Some(&tc), false, true, true, false);
        assert_eq!(
            result.tool_choice.unwrap(),
            json!({ "type": "tool", "name": "testFunction" })
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// convertToAnthropicPrompt (subset supported by the Rust data model)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod convert_prompt_tests {
    use super::*;

    #[test]
    fn should_convert_a_single_system_message_into_an_anthropic_system_message() {
        let p = prompt(vec![msg(
            Role::System,
            vec![text_part("This is a system message")],
        )]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert_eq!(
            system,
            Some(vec![
                json!({ "type": "text", "text": "This is a system message" })
            ])
        );
        assert!(messages.is_empty());
    }

    #[test]
    fn should_convert_multiple_system_messages_into_an_anthropic_system_message() {
        let p = prompt(vec![
            msg(Role::System, vec![text_part("This is a system message")]),
            msg(
                Role::System,
                vec![text_part("This is another system message")],
            ),
        ]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert_eq!(
            system,
            Some(vec![
                json!({ "type": "text", "text": "This is a system message" }),
                json!({ "type": "text", "text": "This is another system message" }),
            ])
        );
        assert!(messages.is_empty());
    }

    #[test]
    fn should_add_image_parts_for_uint8array_images() {
        // TS input uses `data: { type: 'data', data: 'AAECAw==' }`; the Rust
        // `ContentPart::Image` holds the decoded raw bytes and re-encodes them.
        let p = prompt(vec![msg(
            Role::User,
            vec![ContentPart::image(
                decode_b64("AAECAw=="),
                "image/png".to_string(),
            )],
        )]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert!(system.is_none());
        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": "AAECAw==",
                    },
                }],
            })]
        );
    }

    #[test]
    fn should_convert_a_single_tool_result_into_an_anthropic_user_message() {
        let p = prompt(vec![msg(
            Role::Tool,
            vec![ContentPart::tool_result(
                "tool-call-1".to_string(),
                json!({ "type": "json", "value": { "test": "This is a tool message" } }),
            )],
        )]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert!(system.is_none());
        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tool-call-1",
                    "content": "{\"test\":\"This is a tool message\"}",
                }],
            })]
        );
    }

    #[test]
    fn should_convert_multiple_tool_results_into_an_anthropic_user_message() {
        let p = prompt(vec![msg(
            Role::Tool,
            vec![
                ContentPart::tool_result(
                    "tool-call-1".to_string(),
                    json!({ "type": "json", "value": { "test": "This is a tool message" } }),
                ),
                ContentPart::tool_result(
                    "tool-call-2".to_string(),
                    json!({ "type": "json", "value": { "something": "else" } }),
                ),
            ],
        )]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert!(system.is_none());
        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool-call-1",
                        "content": "{\"test\":\"This is a tool message\"}",
                    },
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool-call-2",
                        "content": "{\"something\":\"else\"}",
                    },
                ],
            })]
        );
    }

    #[test]
    fn should_combine_user_and_tool_messages() {
        let p = prompt(vec![
            msg(
                Role::Tool,
                vec![ContentPart::tool_result(
                    "tool-call-1".to_string(),
                    json!({ "type": "json", "value": { "test": "This is a tool message" } }),
                )],
            ),
            msg(Role::User, vec![text_part("This is a user message")]),
        ]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert!(system.is_none());
        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool-call-1",
                        "content": "{\"test\":\"This is a tool message\"}",
                    },
                    { "type": "text", "text": "This is a user message" },
                ],
            })]
        );
    }

    #[test]
    fn should_combine_multiple_sequential_assistant_messages_into_a_single_message() {
        let p = prompt(vec![
            msg(Role::User, vec![text_part("Hi!")]),
            msg(Role::Assistant, vec![text_part("Hello")]),
            msg(Role::Assistant, vec![text_part("World")]),
            msg(Role::Assistant, vec![text_part("!")]),
        ]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert!(system.is_none());
        assert_eq!(
            messages,
            vec![
                json!({ "role": "user", "content": [{ "type": "text", "text": "Hi!" }] }),
                json!({
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "Hello" },
                        { "type": "text", "text": "World" },
                        { "type": "text", "text": "!" },
                    ],
                }),
            ]
        );
    }

    #[test]
    fn should_wrap_non_object_invalid_tool_call_input_in_an_object() {
        let p = prompt(vec![msg(
            Role::Assistant,
            vec![ContentPart::tool_call(
                "call-1".to_string(),
                "cityAttractions".to_string(),
                // malformed JSON the model produced, kept as a raw string
                json!("{ \"city\": \"San Francisco\", }"),
            )],
        )]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert!(system.is_none());
        assert_eq!(
            messages,
            vec![json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "cityAttractions",
                    "id": "call-1",
                    "input": { "rawInvalidInput": "{ \"city\": \"San Francisco\", }" },
                }],
            })]
        );
    }

    #[test]
    fn should_convert_user_assistant_tool_assistant_user_message_sequence_with_multiple_tool_calls()
    {
        let p = prompt(vec![
            msg(
                Role::User,
                vec![text_part("weather for berlin, london and paris")],
            ),
            msg(
                Role::Assistant,
                vec![
                    text_part(
                        "I will use the weather tool to get the weather for berlin, london and paris",
                    ),
                    ContentPart::tool_call(
                        "weather-call-1".to_string(),
                        "weather".to_string(),
                        json!({ "location": "berlin" }),
                    ),
                    ContentPart::tool_call(
                        "weather-call-2".to_string(),
                        "weather".to_string(),
                        json!({ "location": "london" }),
                    ),
                    ContentPart::tool_call(
                        "weather-call-3".to_string(),
                        "weather".to_string(),
                        json!({ "location": "paris" }),
                    ),
                ],
            ),
            msg(
                Role::Tool,
                vec![
                    ContentPart::tool_result(
                        "weather-call-1".to_string(),
                        json!({ "type": "json", "value": { "weather": "sunny" } }),
                    ),
                    ContentPart::tool_result(
                        "weather-call-2".to_string(),
                        json!({ "type": "json", "value": { "weather": "cloudy" } }),
                    ),
                    ContentPart::tool_result(
                        "weather-call-3".to_string(),
                        json!({ "type": "json", "value": { "weather": "rainy" } }),
                    ),
                ],
            ),
            msg(
                Role::Assistant,
                vec![text_part(
                    "The weather for berlin is sunny, the weather for london is cloudy, and the weather for paris is rainy",
                )],
            ),
            msg(Role::User, vec![text_part("and for new york?")]),
        ]);
        let (system, messages) = convert_prompt_to_anthropic(&p);
        assert!(system.is_none());
        assert_eq!(
            messages,
            vec![
                json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": "weather for berlin, london and paris" }],
                }),
                json!({
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "I will use the weather tool to get the weather for berlin, london and paris" },
                        { "type": "tool_use", "id": "weather-call-1", "name": "weather", "input": { "location": "berlin" } },
                        { "type": "tool_use", "id": "weather-call-2", "name": "weather", "input": { "location": "london" } },
                        { "type": "tool_use", "id": "weather-call-3", "name": "weather", "input": { "location": "paris" } },
                    ],
                }),
                json!({
                    "role": "user",
                    "content": [
                        { "type": "tool_result", "tool_use_id": "weather-call-1", "content": "{\"weather\":\"sunny\"}" },
                        { "type": "tool_result", "tool_use_id": "weather-call-2", "content": "{\"weather\":\"cloudy\"}" },
                        { "type": "tool_result", "tool_use_id": "weather-call-3", "content": "{\"weather\":\"rainy\"}" },
                    ],
                }),
                json!({
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "The weather for berlin is sunny, the weather for london is cloudy, and the weather for paris is rainy" }],
                }),
                json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": "and for new york?" }],
                }),
            ]
        );
    }
}
