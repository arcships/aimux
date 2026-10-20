//! Ports of the upstream `@ai-sdk/mistral` `mistral-prepare-tools.test.ts`.

use crate::mistral::prepare_tools::{PreparedTools, prepare_tools};
use aimux_core::tool::FunctionTool;
use serde_json::{Value, json};

/// The upstream `prepareTools` result as a TS object: an `undefined` field is absent.
fn prepared_json(prepared: &PreparedTools) -> Value {
    let mut out = json!({ "toolWarnings": prepared.tool_warnings });
    if let Some(tools) = &prepared.tools {
        out["tools"] = json!(tools);
    }
    if let Some(choice) = &prepared.tool_choice {
        out["toolChoice"] = choice.clone();
    }
    out
}

// The file and its describe block are both named prepareTools upstream.
#[allow(clippy::module_inception)]
mod prepare_tools {
    use super::*;

    fn object_schema() -> Value {
        json!({ "type": "object", "properties": {} })
    }

    /// upstream: @ai-sdk/mistral src/mistral-prepare-tools.test.ts › prepareTools › should pass through strict mode when strict is true
    #[test]
    fn should_pass_through_strict_mode_when_strict_is_true() {
        let tools = Some(vec![
            FunctionTool::new("testFunction", object_schema())
                .with_description("A test function")
                .with_strict(true),
        ]);

        let result = prepare_tools(&tools, None);

        assert_eq!(
            prepared_json(&result),
            json!({
                "toolWarnings": [],
                "tools": [
                    {
                        "function": {
                            "description": "A test function",
                            "name": "testFunction",
                            "parameters": { "properties": {}, "type": "object" },
                            "strict": true
                        },
                        "type": "function"
                    }
                ]
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-prepare-tools.test.ts › prepareTools › should pass through strict mode when strict is false
    #[test]
    fn should_pass_through_strict_mode_when_strict_is_false() {
        let tools = Some(vec![
            FunctionTool::new("testFunction", object_schema())
                .with_description("A test function")
                .with_strict(false),
        ]);

        let result = prepare_tools(&tools, None);

        assert_eq!(
            prepared_json(&result),
            json!({
                "toolWarnings": [],
                "tools": [
                    {
                        "function": {
                            "description": "A test function",
                            "name": "testFunction",
                            "parameters": { "properties": {}, "type": "object" },
                            "strict": false
                        },
                        "type": "function"
                    }
                ]
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-prepare-tools.test.ts › prepareTools › should not include strict mode when strict is undefined
    #[test]
    fn should_not_include_strict_mode_when_strict_is_undefined() {
        let tools = Some(vec![
            FunctionTool::new("testFunction", object_schema()).with_description("A test function"),
        ]);

        let result = prepare_tools(&tools, None);

        assert_eq!(
            prepared_json(&result),
            json!({
                "toolWarnings": [],
                "tools": [
                    {
                        "function": {
                            "description": "A test function",
                            "name": "testFunction",
                            "parameters": { "properties": {}, "type": "object" }
                        },
                        "type": "function"
                    }
                ]
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-prepare-tools.test.ts › prepareTools › should pass through strict mode for multiple tools with different strict settings
    #[test]
    fn should_pass_through_strict_mode_for_multiple_tools_with_different_strict_settings() {
        let tools = Some(vec![
            FunctionTool::new("strictTool", object_schema())
                .with_description("A strict tool")
                .with_strict(true),
            FunctionTool::new("nonStrictTool", object_schema())
                .with_description("A non-strict tool")
                .with_strict(false),
            FunctionTool::new("defaultTool", object_schema())
                .with_description("A tool without strict setting"),
        ]);

        let result = prepare_tools(&tools, None);

        assert_eq!(
            prepared_json(&result),
            json!({
                "toolWarnings": [],
                "tools": [
                    {
                        "function": {
                            "description": "A strict tool",
                            "name": "strictTool",
                            "parameters": { "properties": {}, "type": "object" },
                            "strict": true
                        },
                        "type": "function"
                    },
                    {
                        "function": {
                            "description": "A non-strict tool",
                            "name": "nonStrictTool",
                            "parameters": { "properties": {}, "type": "object" },
                            "strict": false
                        },
                        "type": "function"
                    },
                    {
                        "function": {
                            "description": "A tool without strict setting",
                            "name": "defaultTool",
                            "parameters": { "properties": {}, "type": "object" }
                        },
                        "type": "function"
                    }
                ]
            })
        );
    }
}
