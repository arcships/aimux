use serde_json::{Map, Value, json};

use aimux_core::options::CallOptions;

use crate::openai_compatible::chat::prepare_tools::{PreparedTools, prepare_function_tools};

/// `prepareTools({ tools, toolChoice })`: the Rust form reads both from the
/// call options, given here in their AI SDK JSON form.
fn prepare_tools(tools: Value, tool_choice: Value) -> PreparedTools {
    let mut options = Map::new();
    options.insert("prompt".into(), json!([]));
    if !tools.is_null() {
        options.insert("tools".into(), tools);
    }
    if !tool_choice.is_null() {
        options.insert("toolChoice".into(), tool_choice);
    }
    let options: CallOptions =
        serde_json::from_value(Value::Object(options)).expect("call options");
    prepare_function_tools(&options)
}

/// The result in its upstream shape; `undefined` fields are omitted.
fn result_json(result: &PreparedTools) -> Value {
    let mut out = Map::new();
    if let Some(choice) = &result.tool_choice {
        out.insert("toolChoice".into(), choice.clone());
    }
    out.insert(
        "toolWarnings".into(),
        serde_json::to_value(&result.warnings).expect("warnings serialize"),
    );
    if let Some(tools) = &result.tools {
        out.insert("tools".into(), Value::Array(tools.clone()));
    }
    Value::Object(out)
}

fn test_function_tool() -> Value {
    json!([
        {
            "type": "function",
            "name": "testFunction",
            "description": "Test",
            "inputSchema": {},
        },
    ])
}

// The `prepareTools` describe block shares the file's name.
#[allow(clippy::module_inception)]
mod prepare_tools {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › should return undefined tools and toolChoice when tools are null
    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_null() {
        let result = prepare_tools(Value::Null, Value::Null);

        assert_eq!(result_json(&result), json!({ "toolWarnings": [] }));
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › should return undefined tools and toolChoice when tools are empty
    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_empty() {
        let result = prepare_tools(json!([]), Value::Null);

        assert_eq!(result_json(&result), json!({ "toolWarnings": [] }));
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › should correctly prepare function tools
    #[test]
    fn should_correctly_prepare_function_tools() {
        let result = prepare_tools(
            json!([
                {
                    "type": "function",
                    "name": "testFunction",
                    "description": "A test function",
                    "inputSchema": { "type": "object", "properties": {} },
                },
            ]),
            Value::Null,
        );

        assert_eq!(
            result_json(&result),
            json!({
                "toolWarnings": [],
                "tools": [
                    {
                        "function": {
                            "description": "A test function",
                            "name": "testFunction",
                            "parameters": {
                                "properties": {},
                                "type": "object",
                            },
                        },
                        "type": "function",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › should add warnings for unsupported provider-defined tools
    #[test]
    fn should_add_warnings_for_unsupported_provider_defined_tools() {
        let result = prepare_tools(
            json!([
                {
                    "type": "provider",
                    "id": "some.unsupported_tool",
                    "name": "unsupported_tool",
                    "args": {},
                },
            ]),
            Value::Null,
        );

        assert_eq!(result.tools, Some(vec![]));
        assert_eq!(result.tool_choice, None);
        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([
                {
                    "feature": "provider-defined tool some.unsupported_tool",
                    "type": "unsupported",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › should handle tool choice "auto"
    #[test]
    fn should_handle_tool_choice_auto() {
        let result = prepare_tools(test_function_tool(), json!({ "type": "auto" }));

        assert_eq!(result.tool_choice, Some(json!("auto")));
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › should handle tool choice "required"
    #[test]
    fn should_handle_tool_choice_required() {
        let result = prepare_tools(test_function_tool(), json!({ "type": "required" }));

        assert_eq!(result.tool_choice, Some(json!("required")));
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › should handle tool choice "none"
    #[test]
    fn should_handle_tool_choice_none() {
        let result = prepare_tools(test_function_tool(), json!({ "type": "none" }));

        assert_eq!(result.tool_choice, Some(json!("none")));
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › should handle tool choice "tool"
    #[test]
    fn should_handle_tool_choice_tool() {
        let result = prepare_tools(
            test_function_tool(),
            json!({ "type": "tool", "toolName": "testFunction" }),
        );

        assert_eq!(
            result.tool_choice,
            Some(json!({
                "type": "function",
                "function": { "name": "testFunction" },
            }))
        );
    }

    mod strict_mode {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › strict mode › should pass through strict mode when strict is true
        #[test]
        fn should_pass_through_strict_mode_when_strict_is_true() {
            let result = prepare_tools(
                json!([
                    {
                        "type": "function",
                        "name": "testFunction",
                        "description": "A test function",
                        "inputSchema": { "type": "object", "properties": {} },
                        "strict": true,
                    },
                ]),
                Value::Null,
            );

            assert_eq!(
                result_json(&result),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {
                            "function": {
                                "description": "A test function",
                                "name": "testFunction",
                                "parameters": {
                                    "properties": {},
                                    "type": "object",
                                },
                                "strict": true,
                            },
                            "type": "function",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › strict mode › should pass through strict mode when strict is false
        #[test]
        fn should_pass_through_strict_mode_when_strict_is_false() {
            let result = prepare_tools(
                json!([
                    {
                        "type": "function",
                        "name": "testFunction",
                        "description": "A test function",
                        "inputSchema": { "type": "object", "properties": {} },
                        "strict": false,
                    },
                ]),
                Value::Null,
            );

            assert_eq!(
                result_json(&result),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {
                            "function": {
                                "description": "A test function",
                                "name": "testFunction",
                                "parameters": {
                                    "properties": {},
                                    "type": "object",
                                },
                                "strict": false,
                            },
                            "type": "function",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › strict mode › should not include strict mode when strict is undefined
        #[test]
        fn should_not_include_strict_mode_when_strict_is_undefined() {
            let result = prepare_tools(
                json!([
                    {
                        "type": "function",
                        "name": "testFunction",
                        "description": "A test function",
                        "inputSchema": { "type": "object", "properties": {} },
                    },
                ]),
                Value::Null,
            );

            assert_eq!(
                result_json(&result),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {
                            "function": {
                                "description": "A test function",
                                "name": "testFunction",
                                "parameters": {
                                    "properties": {},
                                    "type": "object",
                                },
                            },
                            "type": "function",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-prepare-tools.test.ts › prepareTools › strict mode › should pass through strict mode for multiple tools with different strict settings
        #[test]
        fn should_pass_through_strict_mode_for_multiple_tools_with_different_strict() {
            let result = prepare_tools(
                json!([
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
                ]),
                Value::Null,
            );

            assert_eq!(
                result_json(&result),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {
                            "function": {
                                "description": "A strict tool",
                                "name": "strictTool",
                                "parameters": {
                                    "properties": {},
                                    "type": "object",
                                },
                                "strict": true,
                            },
                            "type": "function",
                        },
                        {
                            "function": {
                                "description": "A non-strict tool",
                                "name": "nonStrictTool",
                                "parameters": {
                                    "properties": {},
                                    "type": "object",
                                },
                                "strict": false,
                            },
                            "type": "function",
                        },
                        {
                            "function": {
                                "description": "A tool without strict setting",
                                "name": "defaultTool",
                                "parameters": {
                                    "properties": {},
                                    "type": "object",
                                },
                            },
                            "type": "function",
                        },
                    ],
                })
            );
        }
    }
}
