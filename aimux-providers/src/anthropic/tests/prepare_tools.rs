//! Port of `@ai-sdk/anthropic` 4.0.71 `src/anthropic-prepare-tools.test.ts`.
//!
//! Upstream calls `prepareTools` directly; the Rust counterpart is
//! `prepare_tools_with_provider` (`prepare_tools_with_validator` when the
//! test passes its own `CacheControlValidator`). Tools are written as the V4
//! JSON they are in upstream and deserialized into `Tool`. The
//! `toMatchInlineSnapshot` results are compared as JSON with upstream's keys
//! (`betas`, `toolChoice`, `toolWarnings`, `tools`); `undefined` keys are
//! omitted and a `Set` of betas is compared regardless of insertion order.
//!
//! `rejectsForcedToolUse` has no `prepareTools` parameter in aimux: the
//! fallback is applied while building the request body from the model's
//! capabilities, so those cases go through `build_request_body_with_warnings`
//! with a model that rejects forced tool use (`claude-opus-5-5`).
//!
//! `anthropicResponseSchema` / `anthropicChunkSchema` are the serde types the
//! response and stream parsers decode with (`types::AnthropicResponse`,
//! `types::StreamEvent`); "validates" is "deserializes".

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::tool::{LanguageModelV4ToolChoice, Tool};
use aimux_core::types::Warning;
use serde_json::{Value, json};

use crate::anthropic::cache_control::CacheControlValidator;
use crate::anthropic::convert::build_request_body_with_warnings;
use crate::anthropic::options::CANONICAL;
use crate::anthropic::prepare_tools::{
    AnthropicTool, PreparedTools, prepare_tools_with_provider, prepare_tools_with_validator,
};
use crate::anthropic::types::{AnthropicResponse, StreamEvent};

/// V4 tools (`type: 'function' | 'provider'`) as `prepareTools` receives them.
fn tools(value: Value) -> Vec<AnthropicTool> {
    let tools: Vec<Tool> = serde_json::from_value(value).expect("valid tools");
    tools
        .into_iter()
        .map(|tool| match tool {
            Tool::Function(function) => AnthropicTool::Function(function),
            Tool::Provider(provider) => AnthropicTool::Provider {
                id: provider.id.to_string(),
                name: provider.name,
                args: provider.args,
            },
        })
        .collect()
}

fn tool_choice(value: Value) -> LanguageModelV4ToolChoice {
    serde_json::from_value(value).expect("valid tool choice")
}

/// `prepareTools({ tools, toolChoice, supportsStructuredOutput, supportsStrictTools })`.
fn try_prepare_with(
    tools_json: Option<Value>,
    choice: Option<Value>,
    supports_structured_output: bool,
    supports_strict_tools: bool,
) -> Result<PreparedTools, AiMuxError> {
    let tools = tools_json.map(tools);
    let choice = choice.map(tool_choice);
    prepare_tools_with_provider(
        tools.as_deref(),
        choice.as_ref(),
        false,
        supports_structured_output,
        supports_strict_tools,
        false,
    )
}

/// `prepareTools` with `supportsStructuredOutput: true, supportsStrictTools: true`.
fn try_prepare(tools_json: Value, choice: Option<Value>) -> Result<PreparedTools, AiMuxError> {
    try_prepare_with(Some(tools_json), choice, true, true)
}

fn prepare(tools_json: Value) -> PreparedTools {
    try_prepare(tools_json, None).expect("prepareTools succeeds")
}

fn prepare_choice(tools_json: Value, choice: Value) -> PreparedTools {
    try_prepare(tools_json, Some(choice)).expect("prepareTools succeeds")
}

/// A `Set` of betas, in insertion order as upstream's inline snapshots print it.
fn set(items: &[&str]) -> Value {
    json!(items)
}

/// The result as upstream's inline snapshots print it.
fn snapshot(result: &PreparedTools) -> Value {
    let mut value = json!({
        "betas": result.betas,
        "toolWarnings": result.tool_warnings,
    });
    if let Some(tools) = &result.tools {
        value["tools"] = json!(tools);
    }
    if let Some(choice) = &result.tool_choice {
        value["toolChoice"] = choice.clone();
    }
    value
}

/// `{ betas, toolChoice: undefined, toolWarnings: [], tools }`.
fn expected(betas: &[&str], tools: Value) -> Value {
    json!({ "betas": set(betas), "toolWarnings": [], "tools": tools })
}

fn function_tool(extra: Value) -> Value {
    let mut tool = json!({
        "type": "function",
        "name": "testFunction",
        "description": "A test function",
        "inputSchema": { "type": "object", "properties": {} },
    });
    tool.as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap());
    json!([tool])
}

fn provider_tool(id: &str, name: &str, args: Value) -> Value {
    json!([{ "type": "provider", "id": id, "name": name, "args": args }])
}

fn test_tool() -> Value {
    json!([{
        "type": "function",
        "name": "testFunction",
        "description": "Test",
        "inputSchema": {},
    }])
}

#[allow(
    clippy::module_inception,
    reason = "the upstream describe is named after the file"
)]
mod prepare_tools {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should return undefined tools and tool_choice when tools are null
    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_null() {
        let result = try_prepare_with(None, None, true, true).unwrap();
        assert_eq!(
            snapshot(&result),
            json!({ "toolWarnings": [], "betas": set(&[]) })
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should return undefined tools and tool_choice when tools are empty
    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_empty() {
        let result = prepare(json!([]));
        assert_eq!(
            snapshot(&result),
            json!({ "toolWarnings": [], "betas": set(&[]) })
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should correctly prepare function tools
    #[test]
    fn should_correctly_prepare_function_tools() {
        let result = prepare(function_tool(json!({
            "providerOptions": { CANONICAL: { "eagerInputStreaming": true } },
        })));
        assert_eq!(
            result.tools,
            Some(vec![json!({
                "name": "testFunction",
                "description": "A test function",
                "input_schema": { "type": "object", "properties": {} },
                "eager_input_streaming": true,
            })])
        );
        assert_eq!(result.tool_choice, None);
        assert_eq!(result.tool_warnings, Vec::<Warning>::new());
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should correctly preserve tool input examples
    #[test]
    fn should_correctly_preserve_tool_input_examples() {
        let result = prepare(json!([{
            "type": "function",
            "name": "tool_with_examples",
            "description": "tool with examples",
            "inputSchema": { "type": "object", "properties": { "a": { "type": "number" } } },
            "inputExamples": [{ "input": { "a": 1 } }, { "input": { "a": 2 } }],
        }]));
        assert_eq!(
            snapshot(&result),
            expected(
                &[
                    "structured-outputs-2025-11-13",
                    "advanced-tool-use-2025-11-20"
                ],
                json!([{
                    "description": "tool with examples",
                    "input_examples": [{ "a": 1 }, { "a": 2 }],
                    "input_schema": { "properties": { "a": { "type": "number" } }, "type": "object" },
                    "name": "tool_with_examples",
                }]),
            )
        );
    }

    mod strict_mode_for_function_tools {
        use super::*;

        fn prepared(strict: Option<bool>) -> Value {
            json!([{
                "description": "A test function",
                "input_schema": { "properties": {}, "type": "object" },
                "name": "testFunction",
            }])
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| {
                let mut tool = tool.clone();
                if let Some(strict) = strict {
                    tool["strict"] = json!(strict);
                }
                tool
            })
            .collect()
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › strict mode for function tools › should include strict and structured-outputs beta when supportsStructuredOutput is true and strict is true
        #[test]
        fn should_include_strict_and_structured_outputs_beta_when_strict_is_true() {
            let result = prepare(function_tool(json!({ "strict": true })));
            assert_eq!(
                snapshot(&result),
                expected(&["structured-outputs-2025-11-13"], prepared(Some(true)))
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › strict mode for function tools › should include beta but not strict property when strict is undefined and supportsStructuredOutput is true
        #[test]
        fn should_include_beta_but_not_strict_property_when_strict_is_undefined() {
            let result = prepare(function_tool(json!({})));
            assert_eq!(
                snapshot(&result),
                expected(&["structured-outputs-2025-11-13"], prepared(None))
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › strict mode for function tools › should not include strict, emit warning, and not add beta when both supportsStructuredOutput and supportsStrictTools are false
        #[test]
        fn should_not_include_strict_emit_warning_and_not_add_beta_when_both_unsupported() {
            let result = try_prepare_with(
                Some(function_tool(json!({ "strict": true }))),
                None,
                false,
                false,
            )
            .unwrap();
            assert_eq!(
                snapshot(&result),
                json!({
                    "betas": set(&[]),
                    "toolWarnings": [{
                        "details": "Tool 'testFunction' has strict: true, but strict mode is not supported by this provider. The strict property will be ignored.",
                        "feature": "strict",
                        "type": "unsupported",
                    }],
                    "tools": prepared(None),
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › strict mode for function tools › should include strict but not beta when supportsStructuredOutput is false but supportsStrictTools is true
        #[test]
        fn should_include_strict_but_not_beta_when_only_strict_tools_are_supported() {
            let result = try_prepare_with(
                Some(function_tool(json!({ "strict": true }))),
                None,
                false,
                true,
            )
            .unwrap();
            assert_eq!(snapshot(&result), expected(&[], prepared(Some(true))));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › strict mode for function tools › should include beta when strict is false and supportsStructuredOutput is true
        #[test]
        fn should_include_beta_when_strict_is_false_and_supports_structured_output_is_true() {
            let result = prepare(function_tool(json!({ "strict": false })));
            assert_eq!(
                snapshot(&result),
                expected(&["structured-outputs-2025-11-13"], prepared(Some(false)))
            );
        }
    }

    mod provider_defined_tools {
        use super::*;

        mod computer_20241022 {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › computer_20241022 › should correctly prepare computer_20241022 tool
            #[test]
            fn should_correctly_prepare_computer_20241022_tool() {
                let result = prepare(provider_tool(
                    "anthropic.computer_20241022",
                    "computer",
                    json!({ "displayWidthPx": 800, "displayHeightPx": 600, "displayNumber": 1 }),
                ));
                assert_eq!(
                    snapshot(&result),
                    expected(
                        &["computer-use-2024-10-22"],
                        json!([{
                            "display_height_px": 600,
                            "display_number": 1,
                            "display_width_px": 800,
                            "name": "computer",
                            "type": "computer_20241022",
                        }]),
                    )
                );
            }
        }

        mod computer_20250124 {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › computer_20250124 › should correctly prepare computer_20250124 tool
            #[test]
            fn should_correctly_prepare_computer_20250124_tool() {
                let result = prepare(provider_tool(
                    "anthropic.computer_20250124",
                    "computer",
                    json!({ "displayWidthPx": 1024, "displayHeightPx": 768, "displayNumber": 1 }),
                ));
                assert_eq!(
                    snapshot(&result),
                    expected(
                        &["computer-use-2025-01-24"],
                        json!([{
                            "display_height_px": 768,
                            "display_number": 1,
                            "display_width_px": 1024,
                            "name": "computer",
                            "type": "computer_20250124",
                        }]),
                    )
                );
            }
        }

        mod computer_20251124 {
            use super::*;

            fn computer(enable_zoom: Option<bool>) -> (Value, Value) {
                let mut args =
                    json!({ "displayWidthPx": 1024, "displayHeightPx": 768, "displayNumber": 1 });
                let mut tool = json!({
                    "display_height_px": 768,
                    "display_number": 1,
                    "display_width_px": 1024,
                    "name": "computer",
                    "type": "computer_20251124",
                });
                if let Some(enable_zoom) = enable_zoom {
                    args["enableZoom"] = json!(enable_zoom);
                    tool["enable_zoom"] = json!(enable_zoom);
                }
                (
                    provider_tool("anthropic.computer_20251124", "computer", args),
                    expected(&["computer-use-2025-11-24"], json!([tool])),
                )
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › computer_20251124 › should correctly prepare computer_20251124 tool
            #[test]
            fn should_correctly_prepare_computer_20251124_tool() {
                let (tools, expected) = computer(None);
                assert_eq!(snapshot(&prepare(tools)), expected);
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › computer_20251124 › should correctly prepare computer_20251124 tool with enableZoom
            #[test]
            fn should_correctly_prepare_computer_20251124_tool_with_enable_zoom() {
                let (tools, expected) = computer(Some(true));
                assert_eq!(snapshot(&prepare(tools)), expected);
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › computer_20251124 › should correctly prepare computer_20251124 tool with enableZoom false
            #[test]
            fn should_correctly_prepare_computer_20251124_tool_with_enable_zoom_false() {
                let (tools, expected) = computer(Some(false));
                assert_eq!(snapshot(&prepare(tools)), expected);
            }
        }

        mod text_editor_20241022 {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › text_editor_20241022 › should correctly prepare text_editor_20241022 tool
            #[test]
            fn should_correctly_prepare_text_editor_20241022_tool() {
                let result = prepare(provider_tool(
                    "anthropic.text_editor_20241022",
                    "text_editor",
                    json!({}),
                ));
                assert_eq!(
                    snapshot(&result),
                    expected(
                        &["computer-use-2024-10-22"],
                        json!([{ "name": "str_replace_editor", "type": "text_editor_20241022" }]),
                    )
                );
            }
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare bash_20241022 tool
        #[test]
        fn should_correctly_prepare_bash_20241022_tool() {
            let result = prepare(provider_tool("anthropic.bash_20241022", "bash", json!({})));
            assert_eq!(
                snapshot(&result),
                expected(
                    &["computer-use-2024-10-22"],
                    json!([{ "name": "bash", "type": "bash_20241022" }]),
                )
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare text_editor_20250728 with max_characters
        #[test]
        fn should_correctly_prepare_text_editor_20250728_with_max_characters() {
            let result = prepare(provider_tool(
                "anthropic.text_editor_20250728",
                "str_replace_based_edit_tool",
                json!({ "maxCharacters": 10000 }),
            ));
            assert_eq!(
                snapshot(&result),
                expected(
                    &[],
                    json!([{
                        "max_characters": 10000,
                        "name": "str_replace_based_edit_tool",
                        "type": "text_editor_20250728",
                    }]),
                )
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare text_editor_20250728 without max_characters
        #[test]
        fn should_correctly_prepare_text_editor_20250728_without_max_characters() {
            let result = prepare(provider_tool(
                "anthropic.text_editor_20250728",
                "str_replace_based_edit_tool",
                json!({}),
            ));
            assert_eq!(
                snapshot(&result),
                expected(
                    &[],
                    json!([{ "name": "str_replace_based_edit_tool", "type": "text_editor_20250728" }]),
                )
            );
        }

        fn web_search(id: &str, args: Value, betas: &[&str], tool: Value) {
            let result = prepare(provider_tool(id, "web_search", args));
            assert_eq!(snapshot(&result), expected(betas, json!([tool])), "{id}");
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare web_search_20250305
        #[test]
        fn should_correctly_prepare_web_search_20250305() {
            web_search(
                "anthropic.web_search_20250305",
                json!({
                    "maxUses": 10,
                    "allowedDomains": ["https://www.google.com"],
                    "userLocation": { "type": "approximate", "city": "New York" },
                }),
                &[],
                json!({
                    "allowed_domains": ["https://www.google.com"],
                    "max_uses": 10,
                    "name": "web_search",
                    "type": "web_search_20250305",
                    "user_location": { "city": "New York", "type": "approximate" },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare web_search_20260209
        #[test]
        fn should_correctly_prepare_web_search_20260209() {
            web_search(
                "anthropic.web_search_20260209",
                json!({
                    "maxUses": 10,
                    "allowedDomains": ["https://www.google.com"],
                    "userLocation": { "type": "approximate", "city": "New York" },
                }),
                &["code-execution-web-tools-2026-02-09"],
                json!({
                    "allowed_domains": ["https://www.google.com"],
                    "max_uses": 10,
                    "name": "web_search",
                    "type": "web_search_20260209",
                    "user_location": { "city": "New York", "type": "approximate" },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare web_search_20260318 without a beta header
        #[test]
        fn should_correctly_prepare_web_search_20260318_without_a_beta_header() {
            web_search(
                "anthropic.web_search_20260318",
                json!({
                    "maxUses": 10,
                    "allowedDomains": ["google.com"],
                    "userLocation": { "type": "approximate", "city": "New York" },
                    "responseInclusion": "excluded",
                }),
                &[],
                json!({
                    "allowed_domains": ["google.com"],
                    "max_uses": 10,
                    "name": "web_search",
                    "response_inclusion": "excluded",
                    "type": "web_search_20260318",
                    "user_location": { "city": "New York", "type": "approximate" },
                }),
            );
        }

        fn web_fetch(id: &str, args: Value, betas: &[&str], tool: Value) {
            let result = prepare(provider_tool(id, "web_fetch", args));
            assert_eq!(snapshot(&result), expected(betas, json!([tool])), "{id}");
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare web_fetch_20250910
        #[test]
        fn should_correctly_prepare_web_fetch_20250910() {
            web_fetch(
                "anthropic.web_fetch_20250910",
                json!({
                    "maxUses": 10,
                    "allowedDomains": ["https://www.google.com"],
                    "citations": { "enabled": true },
                    "maxContentTokens": 1000,
                }),
                &["web-fetch-2025-09-10"],
                json!({
                    "allowed_domains": ["https://www.google.com"],
                    "citations": { "enabled": true },
                    "max_content_tokens": 1000,
                    "max_uses": 10,
                    "name": "web_fetch",
                    "type": "web_fetch_20250910",
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare web_fetch_20260209
        #[test]
        fn should_correctly_prepare_web_fetch_20260209() {
            web_fetch(
                "anthropic.web_fetch_20260209",
                json!({
                    "maxUses": 10,
                    "allowedDomains": ["https://www.google.com"],
                    "citations": { "enabled": true },
                    "maxContentTokens": 1000,
                }),
                &["code-execution-web-tools-2026-02-09"],
                json!({
                    "allowed_domains": ["https://www.google.com"],
                    "citations": { "enabled": true },
                    "max_content_tokens": 1000,
                    "max_uses": 10,
                    "name": "web_fetch",
                    "type": "web_fetch_20260209",
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare web_fetch_20260318 without a beta header
        #[test]
        fn should_correctly_prepare_web_fetch_20260318_without_a_beta_header() {
            web_fetch(
                "anthropic.web_fetch_20260318",
                json!({
                    "maxUses": 10,
                    "allowedDomains": ["google.com"],
                    "citations": { "enabled": true },
                    "maxContentTokens": 1000,
                    "useCache": false,
                    "responseInclusion": "excluded",
                }),
                &[],
                json!({
                    "allowed_domains": ["google.com"],
                    "citations": { "enabled": true },
                    "max_content_tokens": 1000,
                    "max_uses": 10,
                    "name": "web_fetch",
                    "response_inclusion": "excluded",
                    "type": "web_fetch_20260318",
                    "use_cache": false,
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare tool_search_regex_20251119
        #[test]
        fn should_correctly_prepare_tool_search_regex_20251119() {
            let result = prepare(provider_tool(
                "anthropic.tool_search_regex_20251119",
                "tool_search",
                json!({}),
            ));
            assert_eq!(
                snapshot(&result),
                expected(
                    &[],
                    json!([{ "name": "tool_search_tool_regex", "type": "tool_search_tool_regex_20251119" }]),
                )
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare code_execution_20260120 without beta header
        #[test]
        fn should_correctly_prepare_code_execution_20260120_without_beta_header() {
            let result = prepare(provider_tool(
                "anthropic.code_execution_20260120",
                "code_execution",
                json!({}),
            ));
            assert_eq!(
                snapshot(&result),
                expected(
                    &[],
                    json!([{ "name": "code_execution", "type": "code_execution_20260120" }]),
                )
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare tool_search_bm25_20251119
        #[test]
        fn should_correctly_prepare_tool_search_bm25_20251119() {
            let result = prepare(provider_tool(
                "anthropic.tool_search_bm25_20251119",
                "tool_search",
                json!({}),
            ));
            assert_eq!(
                snapshot(&result),
                expected(
                    &[],
                    json!([{ "name": "tool_search_tool_bm25", "type": "tool_search_tool_bm25_20251119" }]),
                )
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare advisor_20260301 with only the required model
        #[test]
        fn should_correctly_prepare_advisor_20260301_with_only_the_required_model() {
            let result = prepare(provider_tool(
                "anthropic.advisor_20260301",
                "advisor",
                json!({ "model": "claude-opus-4-7" }),
            ));
            assert_eq!(
                snapshot(&result),
                expected(
                    &["advisor-tool-2026-03-01"],
                    json!([{ "model": "claude-opus-4-7", "name": "advisor", "type": "advisor_20260301" }]),
                )
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should correctly prepare advisor_20260301 with all optional args
        #[test]
        fn should_correctly_prepare_advisor_20260301_with_all_optional_args() {
            let result = prepare(provider_tool(
                "anthropic.advisor_20260301",
                "advisor",
                json!({
                    "model": "claude-opus-4-7",
                    "maxUses": 5,
                    "maxTokens": 2048,
                    "caching": { "type": "ephemeral", "ttl": "1h" },
                }),
            ));
            assert_eq!(
                snapshot(&result),
                expected(
                    &["advisor-tool-2026-03-01"],
                    json!([{
                        "caching": { "ttl": "1h", "type": "ephemeral" },
                        "max_tokens": 2048,
                        "max_uses": 5,
                        "model": "claude-opus-4-7",
                        "name": "advisor",
                        "type": "advisor_20260301",
                    }]),
                )
            );
        }

        /// Upstream rejects with `AI_TypeValidationError`; aimux has no
        /// type-validation error kind and rejects with `InvalidArgument`.
        fn assert_rejected(args: Value) {
            let result = try_prepare(
                provider_tool("anthropic.advisor_20260301", "advisor", args.clone()),
                None,
            );
            assert!(
                matches!(result, Err(AiMuxError::InvalidArgument { .. })),
                "{args}: {result:?}"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should reject advisor_20260301 maxTokens below 1024
        #[test]
        fn should_reject_advisor_20260301_max_tokens_below_1024() {
            assert_rejected(json!({ "model": "claude-opus-4-7", "maxTokens": 1023 }));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › provider-defined tools › should reject non-integer advisor_20260301 maxTokens
        #[test]
        fn should_reject_non_integer_advisor_20260301_max_tokens() {
            assert_rejected(json!({ "model": "claude-opus-4-7", "maxTokens": 2048.5 }));
        }
    }

    mod defer_loading_for_function_tools {
        use super::*;

        fn deferred(defer_loading: bool) -> Value {
            expected(
                &["structured-outputs-2025-11-13"],
                json!([{
                    "defer_loading": defer_loading,
                    "description": "A test function",
                    "input_schema": { "properties": {}, "type": "object" },
                    "name": "testFunction",
                }]),
            )
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › deferLoading for function tools › should include defer_loading when set to true
        #[test]
        fn should_include_defer_loading_when_set_to_true() {
            let result = prepare(function_tool(json!({
                "providerOptions": { CANONICAL: { "deferLoading": true } },
            })));
            assert_eq!(snapshot(&result), deferred(true));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › deferLoading for function tools › should include defer_loading when set to false
        #[test]
        fn should_include_defer_loading_when_set_to_false() {
            let result = prepare(function_tool(json!({
                "providerOptions": { CANONICAL: { "deferLoading": false } },
            })));
            assert_eq!(snapshot(&result), deferred(false));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › deferLoading for function tools › should not include defer_loading when not specified
        #[test]
        fn should_not_include_defer_loading_when_not_specified() {
            let result = prepare(function_tool(json!({})));
            assert!(result.tools.unwrap()[0].get("defer_loading").is_none());
        }
    }

    mod allowed_callers_for_function_tools_programmatic_tool_calling {
        use super::*;

        fn query_database(anthropic: Value) -> Value {
            json!([{
                "type": "function",
                "name": "query_database",
                "description": "Query a database",
                "inputSchema": { "type": "object", "properties": { "sql": { "type": "string" } } },
                "providerOptions": { CANONICAL: anthropic },
            }])
        }

        fn with_callers(caller: &str) -> Value {
            expected(
                &[
                    "structured-outputs-2025-11-13",
                    "advanced-tool-use-2025-11-20",
                ],
                json!([{
                    "allowed_callers": [caller],
                    "description": "Query a database",
                    "input_schema": { "properties": { "sql": { "type": "string" } }, "type": "object" },
                    "name": "query_database",
                }]),
            )
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › allowedCallers for function tools (programmatic tool calling) › should include allowed_callers and advanced-tool-use beta when allowedCallers is set
        #[test]
        fn should_include_allowed_callers_and_advanced_tool_use_beta_when_set() {
            let result = prepare(query_database(
                json!({ "allowedCallers": ["code_execution_20250825"] }),
            ));
            assert_eq!(snapshot(&result), with_callers("code_execution_20250825"));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › allowedCallers for function tools (programmatic tool calling) › should not include allowed_callers when not specified
        #[test]
        fn should_not_include_allowed_callers_when_not_specified() {
            let result = prepare(function_tool(json!({})));
            assert!(result.tools.unwrap()[0].get("allowed_callers").is_none());
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › allowedCallers for function tools (programmatic tool calling) › should include both deferLoading and allowedCallers when both are set
        #[test]
        fn should_include_both_defer_loading_and_allowed_callers_when_both_are_set() {
            let result = prepare(query_database(json!({
                "deferLoading": true,
                "allowedCallers": ["code_execution_20250825"],
            })));
            let tool = &result.tools.as_ref().unwrap()[0];
            assert_eq!(tool.get("defer_loading"), Some(&json!(true)));
            assert_eq!(
                tool.get("allowed_callers"),
                Some(&json!(["code_execution_20250825"]))
            );
            assert!(result.betas.contains("advanced-tool-use-2025-11-20"));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › allowedCallers for function tools (programmatic tool calling) › should include allowed_callers with code_execution_20260120
        #[test]
        fn should_include_allowed_callers_with_code_execution_20260120() {
            let result = prepare(query_database(
                json!({ "allowedCallers": ["code_execution_20260120"] }),
            ));
            assert_eq!(snapshot(&result), with_callers("code_execution_20260120"));
        }
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should add warnings for unsupported tools
    #[test]
    fn should_add_warnings_for_unsupported_tools() {
        let result = prepare(provider_tool(
            "unsupported.tool",
            "unsupported_tool",
            json!({}),
        ));
        assert_eq!(result.tools, Some(vec![]));
        assert_eq!(result.tool_choice, None);
        assert_eq!(
            json!(result.tool_warnings),
            json!([{ "feature": "provider-defined tool unsupported.tool", "type": "unsupported" }])
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should handle tool choice "auto"
    #[test]
    fn should_handle_tool_choice_auto() {
        let result = prepare_choice(test_tool(), json!({ "type": "auto" }));
        assert_eq!(result.tool_choice, Some(json!({ "type": "auto" })));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should handle tool choice "required"
    #[test]
    fn should_handle_tool_choice_required() {
        let result = prepare_choice(test_tool(), json!({ "type": "required" }));
        assert_eq!(result.tool_choice, Some(json!({ "type": "any" })));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should handle tool choice "none"
    #[test]
    fn should_handle_tool_choice_none() {
        let result = prepare_choice(test_tool(), json!({ "type": "none" }));
        assert_eq!(result.tools, None);
        assert_eq!(result.tool_choice, None);
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should handle tool choice "tool"
    #[test]
    fn should_handle_tool_choice_tool() {
        let result = prepare_choice(
            test_tool(),
            json!({ "type": "tool", "toolName": "testFunction" }),
        );
        assert_eq!(
            result.tool_choice,
            Some(json!({ "type": "tool", "name": "testFunction" }))
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should set cache control
    #[test]
    fn should_set_cache_control() {
        let result = prepare(json!([{
            "type": "function",
            "name": "testFunction",
            "description": "Test",
            "inputSchema": {},
            "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
        }]));
        assert_eq!(
            json!(result.tools),
            json!([{
                "cache_control": { "type": "ephemeral" },
                "description": "Test",
                "input_schema": {},
                "name": "testFunction",
            }])
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › prepareTools › should limit cache breakpoints to 4
    #[test]
    fn should_limit_cache_breakpoints_to_4() {
        let cache = json!({ CANONICAL: { "cacheControl": { "type": "ephemeral" } } });
        let tools = tools(json!([
            { "type": "function", "name": "tool1", "description": "Test 1", "inputSchema": {}, "providerOptions": cache },
            { "type": "function", "name": "tool2", "description": "Test 2", "inputSchema": {}, "providerOptions": cache },
            { "type": "function", "name": "tool3", "description": "Test 3", "inputSchema": {}, "providerOptions": cache },
            { "type": "function", "name": "tool4", "description": "Test 4", "inputSchema": {}, "providerOptions": cache },
            {
                "type": "function",
                "name": "tool5",
                "description": "Test 5 (should be rejected)",
                "inputSchema": {},
                "providerOptions": cache,
            },
        ]));
        let mut cache_control_validator = CacheControlValidator::new();
        let result = prepare_tools_with_validator(
            Some(&tools),
            None,
            false,
            true,
            true,
            false,
            CANONICAL,
            &mut cache_control_validator,
        )
        .unwrap();

        let prepared = result.tools.unwrap();
        let ephemeral = json!({ "type": "ephemeral" });
        for tool in &prepared[..4] {
            assert_eq!(tool.get("cache_control"), Some(&ephemeral), "{tool}");
        }
        assert!(prepared[4].get("cache_control").is_none());
        assert_eq!(
            json!(cache_control_validator.take_warnings()),
            json!([{
                "details": "Maximum 4 cache breakpoints exceeded (found 5). This breakpoint will be ignored.",
                "feature": "cacheControl breakpoint limit",
                "type": "unsupported",
            }])
        );
    }
}

mod anthropic_response_schema_web_fetch_tool_result {
    use super::*;

    fn response(source: Value) -> Value {
        json!({
            "type": "message",
            "id": "123",
            "model": "claude-3-5-sonnet-20241022",
            "content": [{
                "type": "web_fetch_tool_result",
                "tool_use_id": "srvtoolu_01234567890abcdef",
                "content": {
                    "type": "web_fetch_result",
                    "url": "https://test.com",
                    "retrieved_at": "2025-12-08T20:46:31.114158",
                    "content": { "type": "document", "title": "Example Title", "source": source },
                },
            }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 100, "output_tokens": 50 },
        })
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › anthropicResponseSchema - web_fetch_tool_result › should accept PDF response with base64 source
    #[test]
    fn should_accept_pdf_response_with_base64_source() {
        let result = serde_json::from_value::<AnthropicResponse>(response(json!({
            "type": "base64",
            "media_type": "application/pdf",
            "data": "JVBERi0xLjcNJeLjz9MNC",
        })));
        assert!(result.is_ok(), "{result:?}");
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › anthropicResponseSchema - web_fetch_tool_result › should accept text source in response
    #[test]
    fn should_accept_text_source_in_response() {
        let result = serde_json::from_value::<AnthropicResponse>(response(json!({
            "type": "text",
            "media_type": "text/plain",
            "data": "content",
        })));
        assert!(result.is_ok(), "{result:?}");
    }
}

mod anthropic_chunk_schema_web_fetch_tool_result {
    use super::*;

    fn chunk(source: Value) -> Value {
        json!({
            "type": "content_block_start",
            "index": 10,
            "content_block": {
                "type": "web_fetch_tool_result",
                "tool_use_id": "srvtoolu_01234567890abcdef",
                "content": {
                    "type": "web_fetch_result",
                    "url": "https://test.com",
                    "retrieved_at": "2025-12-08T20:46:31.114158",
                    "content": { "type": "document", "title": null, "source": source },
                },
            },
        })
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › anthropicChunkSchema - web_fetch_tool_result › should accept base64 PDF source in streaming response
    #[test]
    fn should_accept_base64_pdf_source_in_streaming_response() {
        let result = serde_json::from_value::<StreamEvent>(chunk(json!({
            "type": "base64",
            "media_type": "application/pdf",
            "data": "JVBERi0xLjcNJeLjz9MNC",
        })));
        assert!(result.is_ok(), "{result:?}");
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › anthropicChunkSchema - web_fetch_tool_result › should accept text source in streaming response
    #[test]
    fn should_accept_text_source_in_streaming_response() {
        let result = serde_json::from_value::<StreamEvent>(chunk(json!({
            "type": "text",
            "media_type": "text/plain",
            "data": "content",
        })));
        assert!(result.is_ok(), "{result:?}");
    }
}

mod anthropic_chunk_schema_shared_batch_content_variants {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › anthropicChunkSchema - shared batch content variants › accepts a string MCP tool result in a streaming content block
    #[test]
    fn accepts_a_string_mcp_tool_result_in_a_streaming_content_block() {
        let result = serde_json::from_value::<StreamEvent>(json!({
            "content_block": {
                "content": "tool output",
                "is_error": false,
                "tool_use_id": "mcp_123",
                "type": "mcp_tool_result",
            },
            "index": 0,
            "type": "content_block_start",
        }));
        assert!(result.is_ok(), "{result:?}");
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › anthropicChunkSchema - shared batch content variants › accepts opaque MCP tool result citations
    #[test]
    fn accepts_opaque_mcp_tool_result_citations() {
        let result = serde_json::from_value::<StreamEvent>(json!({
            "content_block": {
                "content": [{
                    "citations": [{ "reference": "opaque-reference", "type": "future_citation_variant" }],
                    "text": "tool output",
                    "type": "text",
                }],
                "is_error": false,
                "tool_use_id": "mcp_123",
                "type": "mcp_tool_result",
            },
            "index": 0,
            "type": "content_block_start",
        }));
        assert!(result.is_ok(), "{result:?}");
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › anthropicChunkSchema - shared batch content variants › accepts $type citation deltas
    #[test]
    fn accepts_type_citation_deltas() {
        for citation in [
            json!({
                "cited_text": "block",
                "document_index": 0,
                "document_title": null,
                "end_block_index": 2,
                "file_id": null,
                "start_block_index": 1,
                "type": "content_block_location",
            }),
            json!({
                "cited_text": "result",
                "end_block_index": 2,
                "search_result_index": 0,
                "source": "https://example.com",
                "start_block_index": 1,
                "title": null,
                "type": "search_result_location",
            }),
        ] {
            let result = serde_json::from_value::<StreamEvent>(json!({
                "delta": { "citation": citation, "type": "citations_delta" },
                "index": 0,
                "type": "content_block_delta",
            }));
            assert!(result.is_ok(), "{citation}: {result:?}");
        }
    }
}

mod computer_toolset_20260801 {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › computer_toolset_20260801 › should prepare the toolset without a name or beta header
    #[test]
    fn should_prepare_the_toolset_without_a_name_or_beta_header() {
        let result = prepare(provider_tool(
            "anthropic.computer_toolset_20260801",
            "computer",
            json!({}),
        ));
        assert_eq!(
            snapshot(&result),
            expected(&[], json!([{ "type": "computer_toolset_20260801" }]))
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › computer_toolset_20260801 › should map member configs to snake_case
    #[test]
    fn should_map_member_configs_to_snake_case() {
        let result = prepare(provider_tool(
            "anthropic.computer_toolset_20260801",
            "computer",
            json!({
                "configs": {
                    "zoom": { "enabled": false },
                    "wait": { "enabled": true, "deferLoading": true },
                },
            }),
        ));
        assert_eq!(
            json!(result.tools),
            json!([{
                "configs": {
                    "wait": { "defer_loading": true, "enabled": true },
                    "zoom": { "enabled": false },
                },
                "type": "computer_toolset_20260801",
            }])
        );
    }
}

mod rejects_forced_tool_use {
    use super::*;

    /// A model whose capabilities reject forced tool use.
    const MODEL: &str = "claude-opus-5-5";

    fn rejecting_tools() -> Vec<Tool> {
        serde_json::from_value(json!([
            { "type": "function", "name": "testFunction", "description": "Test", "inputSchema": {} },
            { "type": "function", "name": "otherFunction", "description": "Other", "inputSchema": {} },
        ]))
        .unwrap()
    }

    /// The request body's `tools` / `tool_choice` and the call's warnings.
    fn body(choice: Value, provider_options: Option<Value>) -> (Value, Vec<Warning>) {
        let options = CallOptions {
            max_output_tokens: Some(100),
            tools: Some(rejecting_tools()),
            tool_choice: Some(tool_choice(choice)),
            provider_options: provider_options
                .map(|options| serde_json::from_value(options).expect("provider options")),
            ..CallOptions::new(vec![LanguageModelMessage::user_text("Hi")])
        };
        let result = build_request_body_with_warnings(MODEL, &options, false).unwrap();
        (result.body, result.warnings)
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › rejectsForcedToolUse › should fall back to auto for tool choice "required"
    #[test]
    fn should_fall_back_to_auto_for_tool_choice_required() {
        let (body, warnings) = body(json!({ "type": "required" }), None);
        assert_eq!(body["tool_choice"], json!({ "type": "auto" }));
        assert_eq!(body["tools"].as_array().unwrap().len(), 2);
        assert_eq!(
            json!(warnings),
            json!([{
                "details": "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.",
                "feature": "toolChoice",
                "type": "unsupported",
            }])
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › rejectsForcedToolUse › should only send the selected tool with auto for tool choice "tool"
    #[test]
    fn should_only_send_the_selected_tool_with_auto_for_tool_choice_tool() {
        let (body, warnings) = body(json!({ "type": "tool", "toolName": "otherFunction" }), None);
        assert_eq!(body["tool_choice"], json!({ "type": "auto" }));
        let names: Vec<&Value> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool.get("name").unwrap_or(&tool["type"]))
            .collect();
        assert_eq!(names, vec![&json!("otherFunction")]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › rejectsForcedToolUse › should preserve disableParallelToolUse in the auto fallback
    #[test]
    fn should_preserve_disable_parallel_tool_use_in_the_auto_fallback() {
        let (body, _) = body(
            json!({ "type": "required" }),
            Some(json!({ CANONICAL: { "disableParallelToolUse": true } })),
        );
        assert_eq!(
            body["tool_choice"],
            json!({ "type": "auto", "disable_parallel_tool_use": true })
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-prepare-tools.test.ts › rejectsForcedToolUse › should not affect auto and none tool choices
    #[test]
    fn should_not_affect_auto_and_none_tool_choices() {
        let (auto, auto_warnings) = body(json!({ "type": "auto" }), None);
        assert_eq!(auto["tool_choice"], json!({ "type": "auto" }));
        assert_eq!(auto_warnings, Vec::<Warning>::new());

        let (none, none_warnings) = body(json!({ "type": "none" }), None);
        assert!(none.get("tools").is_none(), "{none}");
        assert_eq!(none_warnings, Vec::<Warning>::new());
    }
}
