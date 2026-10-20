//! Upstream `groq-prepare-tools.test.ts` (the cases that are not in the
//! integration tests).

use aimux_core::options::CallOptions;
use aimux_core::tool::{FunctionTool, LanguageModelV4ToolChoice, ProviderTool, Tool};
use serde_json::{Value, json};

use crate::groq::prepare_tools::prepare_tools;

/// The `groq.browser_search` provider tool, as the caller passes it.
fn browser_search_tool() -> Tool {
    Tool::Provider(
        serde_json::from_value::<ProviderTool>(json!({
            "id": "groq.browser_search",
            "name": "browser_search",
            "args": {},
        }))
        .expect("valid provider tool"),
    )
}

/// The input schema of a tool with no parameters.
fn empty_object_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

#[allow(
    clippy::module_inception,
    reason = "the upstream describe level keeps its name"
)]
mod prepare_tools {
    use super::*;

    /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should return undefined tools and toolChoice when tools are null
    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_null() {
        let prepared = prepare_tools(&CallOptions::new(Vec::new()), "gemma2-9b-it");

        assert_eq!(prepared.tools, None);
        assert_eq!(prepared.tool_choice, None);
        assert!(prepared.tool_warnings.is_empty());
    }

    /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should return undefined tools and toolChoice when tools are empty
    #[test]
    fn should_return_undefined_tools_and_tool_choice_when_tools_are_empty() {
        let options = CallOptions {
            tools: Some(Vec::new()),
            ..CallOptions::new(Vec::new())
        };

        let prepared = prepare_tools(&options, "gemma2-9b-it");

        assert_eq!(prepared.tools, None);
        assert_eq!(prepared.tool_choice, None);
        assert!(prepared.tool_warnings.is_empty());
    }

    mod strict_mode_for_function_tools {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode for multiple tools with different strict settings
        #[test]
        fn should_pass_through_strict_mode_for_multiple_tools_with_different_strict_settings() {
            let options = CallOptions {
                tools: Some(vec![
                    Tool::Function(FunctionTool {
                        description: Some("A strict tool".to_string()),
                        strict: Some(true),
                        ..FunctionTool::new("strictTool", empty_object_schema())
                    }),
                    Tool::Function(FunctionTool {
                        description: Some("A non-strict tool".to_string()),
                        strict: Some(false),
                        ..FunctionTool::new("nonStrictTool", empty_object_schema())
                    }),
                    Tool::Function(FunctionTool {
                        description: Some("A tool without strict setting".to_string()),
                        ..FunctionTool::new("defaultTool", empty_object_schema())
                    }),
                ]),
                ..CallOptions::new(Vec::new())
            };

            let prepared = prepare_tools(&options, "gemma2-9b-it");

            assert_eq!(
                prepared.tools,
                Some(vec![
                    json!({
                        "type": "function",
                        "function": {
                            "name": "strictTool",
                            "description": "A strict tool",
                            "parameters": { "type": "object", "properties": {} },
                            "strict": true,
                        },
                    }),
                    json!({
                        "type": "function",
                        "function": {
                            "name": "nonStrictTool",
                            "description": "A non-strict tool",
                            "parameters": { "type": "object", "properties": {} },
                            "strict": false,
                        },
                    }),
                    json!({
                        "type": "function",
                        "function": {
                            "name": "defaultTool",
                            "description": "A tool without strict setting",
                            "parameters": { "type": "object", "properties": {} },
                        },
                    }),
                ])
            );
        }
    }

    mod browser_search_tool {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should handle browser search with tool choice
        #[test]
        fn should_handle_browser_search_with_tool_choice() {
            let options = CallOptions {
                tools: Some(vec![browser_search_tool()]),
                tool_choice: Some(LanguageModelV4ToolChoice::Required),
                ..CallOptions::new(Vec::new())
            };

            let prepared = prepare_tools(&options, "openai/gpt-oss-120b");

            assert_eq!(
                prepared.tools,
                Some(vec![json!({ "type": "browser_search" })])
            );
            assert_eq!(prepared.tool_choice, Some(json!("required")));
            assert!(prepared.tool_warnings.is_empty());
        }
    }
}
