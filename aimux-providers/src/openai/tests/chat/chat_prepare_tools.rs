//! `prepareChatTools` is the function-tool part of aimux's chat request body
//! build: `build_request_body_with_warnings` normalizes each tool schema, warns
//! for provider tools and writes `tools` / `tool_choice`. The result is read
//! back as `{ tools, toolChoice, toolWarnings }` (absent keys = `undefined`).

use aimux_core::options::CallOptions;
use serde_json::{Map, Value, json};

use crate::openai::chat::chat_language_model::build_request_body_with_warnings;

/// `prepareChatTools({ tools, toolChoice })`: `input` holds the upstream
/// `tools` / `toolChoice` literals.
fn prepare_chat_tools(input: Value) -> Value {
    let mut options = json!({ "prompt": [] });
    for (key, value) in input.as_object().unwrap() {
        options[key] = value.clone();
    }
    let options: CallOptions = serde_json::from_value(options).unwrap();
    let result = build_request_body_with_warnings("gpt-4o", &options, false).unwrap();
    let mut prepared = Map::new();
    if let Some(tools) = result.body.get("tools") {
        prepared.insert("tools".into(), tools.clone());
    }
    if let Some(tool_choice) = result.body.get("tool_choice") {
        prepared.insert("toolChoice".into(), tool_choice.clone());
    }
    prepared.insert(
        "toolWarnings".into(),
        serde_json::to_value(result.warnings).unwrap(),
    );
    Value::Object(prepared)
}

fn test_function(tool_choice: Value) -> Value {
    prepare_chat_tools(json!({
        "tools": [{
            "type": "function",
            "name": "testFunction",
            "description": "Test",
            "inputSchema": {},
        }],
        "toolChoice": tool_choice,
    }))
}

mod prepare_chat_tools {
    use super::*;

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should return undefined tools and toolChoice when tools are null
    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_null() {
        assert_eq!(prepare_chat_tools(json!({})), json!({ "toolWarnings": [] }));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should return undefined tools and toolChoice when tools are empty
    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_empty() {
        assert_eq!(
            prepare_chat_tools(json!({ "tools": [] })),
            json!({ "toolWarnings": [] })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should correctly prepare function tools
    #[test]
    fn should_correctly_prepare_function_tools() {
        let result = prepare_chat_tools(json!({
            "tools": [{
                "type": "function",
                "name": "testFunction",
                "description": "A test function",
                "inputSchema": { "type": "object", "properties": {} },
            }],
        }));

        assert_eq!(
            result,
            json!({
                "toolWarnings": [],
                "tools": [{
                    "function": {
                        "description": "A test function",
                        "name": "testFunction",
                        "parameters": { "properties": {}, "type": "object" },
                    },
                    "type": "function",
                }],
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should remove string propertyNames from function tools and warn
    #[test]
    fn should_remove_string_property_names_from_function_tools_and_warn() {
        let result = prepare_chat_tools(json!({
            "tools": [{
                "type": "function",
                "name": "testFunction",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "values": {
                            "type": "object",
                            "propertyNames": { "type": "string", "pattern": "^[A-Z_]+$" },
                        },
                    },
                },
            }],
        }));

        assert_eq!(
            result,
            json!({
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "testFunction",
                        "parameters": {
                            "type": "object",
                            "properties": { "values": { "type": "object" } },
                        },
                    },
                }],
                "toolWarnings": [{
                    "type": "compatibility",
                    "feature": "JSON Schema propertyNames",
                    "details": "OpenAI does not support JSON Schema propertyNames. It was removed before sending the schema, so OpenAI will not enforce property-name constraints.",
                }],
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should remove regex lookaround patterns from function tools and warn
    #[test]
    fn should_remove_regex_lookaround_patterns_from_function_tools_and_warn() {
        let result = prepare_chat_tools(json!({
            "tools": [{
                "type": "function",
                "name": "createContact",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "email": { "type": "string", "format": "email", "pattern": "^(?!\\.).+@.+$" },
                        "username": { "type": "string", "pattern": "^@[a-zA-Z0-9_]+$" },
                    },
                },
            }],
        }));

        assert_eq!(
            result,
            json!({
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "createContact",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "email": { "type": "string", "format": "email" },
                                "username": { "type": "string", "pattern": "^@[a-zA-Z0-9_]+$" },
                            },
                        },
                    },
                }],
                "toolWarnings": [{
                    "type": "compatibility",
                    "feature": "JSON Schema pattern with regex lookaround",
                    "details": "OpenAI does not support regex lookaround in JSON Schema patterns. The pattern was removed before sending the schema, so OpenAI will not enforce that constraint.",
                }],
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should add warnings for unsupported tools
    #[test]
    fn should_add_warnings_for_unsupported_tools() {
        let result = prepare_chat_tools(json!({
            "tools": [{
                "type": "provider",
                "id": "openai.unsupported_tool",
                "name": "unsupported_tool",
                "args": {},
            }],
        }));

        assert_eq!(result["tools"], json!([]));
        assert_eq!(result.get("toolChoice"), None);
        assert_eq!(
            result["toolWarnings"],
            json!([{ "feature": "tool type: provider", "type": "unsupported" }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should handle tool choice "auto"
    #[test]
    fn should_handle_tool_choice_auto() {
        assert_eq!(
            test_function(json!({ "type": "auto" }))["toolChoice"],
            json!("auto")
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should handle tool choice "required"
    #[test]
    fn should_handle_tool_choice_required() {
        assert_eq!(
            test_function(json!({ "type": "required" }))["toolChoice"],
            json!("required")
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should handle tool choice "none"
    #[test]
    fn should_handle_tool_choice_none() {
        assert_eq!(
            test_function(json!({ "type": "none" }))["toolChoice"],
            json!("none")
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should handle tool choice "tool"
    #[test]
    fn should_handle_tool_choice_tool() {
        assert_eq!(
            test_function(json!({ "type": "tool", "toolName": "testFunction" }))["toolChoice"],
            json!({ "type": "function", "function": { "name": "testFunction" } })
        );
    }

    fn strict_tool(strict: Option<bool>) -> Value {
        let mut tool = json!({
            "type": "function",
            "name": "testFunction",
            "description": "A test function",
            "inputSchema": { "type": "object", "properties": {} },
        });
        if let Some(strict) = strict {
            tool["strict"] = json!(strict);
        }
        prepare_chat_tools(json!({ "tools": [tool] }))
    }

    fn expected_strict_tool(strict: Option<bool>) -> Value {
        let mut function = json!({
            "description": "A test function",
            "name": "testFunction",
            "parameters": { "properties": {}, "type": "object" },
        });
        if let Some(strict) = strict {
            function["strict"] = json!(strict);
        }
        json!({
            "toolWarnings": [],
            "tools": [{ "function": function, "type": "function" }],
        })
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should pass through strict mode when strict is true
    #[test]
    fn should_pass_through_strict_mode_when_strict_is_true() {
        assert_eq!(strict_tool(Some(true)), expected_strict_tool(Some(true)));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should pass through strict mode when strict is false
    #[test]
    fn should_pass_through_strict_mode_when_strict_is_false() {
        assert_eq!(strict_tool(Some(false)), expected_strict_tool(Some(false)));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should not include strict mode when strict is undefined
    #[test]
    fn should_not_include_strict_mode_when_strict_is_undefined() {
        assert_eq!(strict_tool(None), expected_strict_tool(None));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-prepare-tools.test.ts › prepareChatTools › should pass through strict mode for multiple tools with different strict settings
    #[test]
    fn should_pass_through_strict_mode_for_multiple_tools_with_different_strict_settings() {
        let result = prepare_chat_tools(json!({
            "tools": [
                {
                    "type": "function",
                    "name": "strictTool",
                    "description": "A strict tool",
                    "inputSchema": { "type": "object", "properties": {} },
                    "strict": true,
                },
                {
                    "type": "function",
                    "name": "nonStrictTool",
                    "description": "A non-strict tool",
                    "inputSchema": { "type": "object", "properties": {} },
                    "strict": false,
                },
                {
                    "type": "function",
                    "name": "defaultTool",
                    "description": "A tool without strict setting",
                    "inputSchema": { "type": "object", "properties": {} },
                },
            ],
        }));

        assert_eq!(
            result,
            json!({
                "toolWarnings": [],
                "tools": [
                    {
                        "function": {
                            "description": "A strict tool",
                            "name": "strictTool",
                            "parameters": { "properties": {}, "type": "object" },
                            "strict": true,
                        },
                        "type": "function",
                    },
                    {
                        "function": {
                            "description": "A non-strict tool",
                            "name": "nonStrictTool",
                            "parameters": { "properties": {}, "type": "object" },
                            "strict": false,
                        },
                        "type": "function",
                    },
                    {
                        "function": {
                            "description": "A tool without strict setting",
                            "name": "defaultTool",
                            "parameters": { "properties": {}, "type": "object" },
                        },
                        "type": "function",
                    },
                ],
            })
        );
    }
}
