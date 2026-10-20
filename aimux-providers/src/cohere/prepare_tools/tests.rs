//! Ported `@ai-sdk/cohere` unit tests for `prepare_tools.rs`
//! (`cohere-prepare-tools.test.ts`).
//!
//! One module per upstream test file, one nested module per `describe`, one
//! `#[test]` per `it`. Each test carries its upstream chain in an `upstream:`
//! marker line.

use super::*;
use aimux_core::tool::{FunctionTool, ProviderTool};
use aimux_core::types::NamespacedKind;

mod cohere_prepare_tools {
    use super::*;

    /// upstream: @ai-sdk/cohere src/cohere-prepare-tools.test.ts › should return undefined tools when no tools are provided
    #[test]
    fn should_return_undefined_tools_when_no_tools_are_provided() {
        let result = prepare_tools(&Some(Vec::new()), None);

        assert_eq!(result.tools, None);
        assert_eq!(result.tool_choice, None);
        assert!(result.tool_warnings.is_empty());
    }

    /// upstream: @ai-sdk/cohere src/cohere-prepare-tools.test.ts › should process function tools correctly
    #[test]
    fn should_process_function_tools_correctly() {
        let function_tool = Tool::Function(
            FunctionTool::new(
                "testFunction",
                json!({ "type": "object", "properties": {} }),
            )
            .with_description("test description"),
        );

        let result = prepare_tools(&Some(vec![function_tool]), None);

        assert_eq!(
            result.tools,
            Some(vec![json!({
                "type": "function",
                "function": {
                    "name": "testFunction",
                    "description": "test description",
                    "parameters": { "type": "object", "properties": {} },
                },
            })])
        );
        assert_eq!(result.tool_choice, None);
        assert!(result.tool_warnings.is_empty());
    }

    /// upstream: @ai-sdk/cohere src/cohere-prepare-tools.test.ts › should add warnings for provider-defined tools
    #[test]
    fn should_add_warnings_for_provider_defined_tools() {
        let provider_tool = Tool::Provider(ProviderTool {
            id: NamespacedKind::new("provider.tool").unwrap(),
            name: "tool".into(),
            args: serde_json::Map::new(),
            supports_deferred_results: None,
        });

        let result = prepare_tools(&Some(vec![provider_tool]), None);

        // Inline snapshot: `toolChoice: undefined`, `tools: []`, and one
        // unsupported warning with no `details`.
        assert_eq!(result.tools, Some(Vec::new()));
        assert_eq!(result.tool_choice, None);
        assert_eq!(
            serde_json::to_value(&result.tool_warnings).unwrap(),
            json!([
                {
                    "type": "unsupported",
                    "feature": "provider-defined tool provider.tool",
                },
            ])
        );
    }

    mod tool_choice_handling {
        use super::*;

        fn basic_tool() -> Tool {
            Tool::Function(
                FunctionTool::new(
                    "testFunction",
                    json!({ "type": "object", "properties": {} }),
                )
                .with_description("test description"),
            )
        }

        fn basic_tool_json() -> Value {
            json!({
                "type": "function",
                "function": {
                    "name": "testFunction",
                    "description": "test description",
                    "parameters": { "type": "object", "properties": {} },
                },
            })
        }

        /// upstream: @ai-sdk/cohere src/cohere-prepare-tools.test.ts › tool choice handling › should handle auto tool choice
        #[test]
        fn should_handle_auto_tool_choice() {
            let result = prepare_tools(
                &Some(vec![basic_tool()]),
                Some(&LanguageModelV4ToolChoice::Auto),
            );

            assert_eq!(result.tool_choice, None);
        }

        /// upstream: @ai-sdk/cohere src/cohere-prepare-tools.test.ts › tool choice handling › should handle none tool choice
        #[test]
        fn should_handle_none_tool_choice() {
            let result = prepare_tools(
                &Some(vec![basic_tool()]),
                Some(&LanguageModelV4ToolChoice::None),
            );

            assert_eq!(result.tools, Some(vec![basic_tool_json()]));
            assert_eq!(result.tool_choice, Some(json!("NONE")));
            assert!(result.tool_warnings.is_empty());
        }

        /// upstream: @ai-sdk/cohere src/cohere-prepare-tools.test.ts › tool choice handling › should handle required tool choice
        #[test]
        fn should_handle_required_tool_choice() {
            let result = prepare_tools(
                &Some(vec![basic_tool()]),
                Some(&LanguageModelV4ToolChoice::Required),
            );

            assert_eq!(result.tools, Some(vec![basic_tool_json()]));
            assert_eq!(result.tool_choice, Some(json!("REQUIRED")));
            assert!(result.tool_warnings.is_empty());
        }

        /// upstream: @ai-sdk/cohere src/cohere-prepare-tools.test.ts › tool choice handling › should handle tool type tool choice by filtering tools
        #[test]
        fn should_handle_tool_type_tool_choice_by_filtering_tools() {
            let result = prepare_tools(
                &Some(vec![basic_tool()]),
                Some(&LanguageModelV4ToolChoice::Tool {
                    tool_name: "testFunction".into(),
                }),
            );

            assert_eq!(result.tools, Some(vec![basic_tool_json()]));
            assert_eq!(result.tool_choice, Some(json!("REQUIRED")));
            assert!(result.tool_warnings.is_empty());
        }
    }
}
