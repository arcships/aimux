//! Upstream `responses/xai-responses-prepare-tools.test.ts`.
//!
//! The Rust `prepare_responses_tools` is synchronous, so the upstream `await`
//! has no counterpart here.

use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::{FunctionTool, ProviderTool, Tool};
use aimux_core::types::Warning;
use serde_json::{Value, json};

use crate::xai::responses::convert::{PreparedResponsesTools, prepare_responses_tools};

/// A provider-defined tool as the caller passes it: `{ type: 'provider', id, name, args }`.
fn provider_tool(id: &str, name: &str, args: Value) -> Tool {
    Tool::Provider(
        serde_json::from_value::<ProviderTool>(json!({
            "id": id,
            "name": name,
            "args": args,
        }))
        .expect("valid provider tool"),
    )
}

/// A function tool with a description and an input schema.
fn function_tool(name: &str, description: &str, input_schema: Value) -> Tool {
    Tool::Function(FunctionTool {
        description: Some(description.to_string()),
        ..FunctionTool::new(name, input_schema)
    })
}

/// `prepareResponsesTools({ tools, toolChoice })`.
fn prepare(
    tools: Option<Vec<Tool>>,
    tool_choice: Option<LanguageModelV4ToolChoice>,
) -> PreparedResponsesTools {
    prepare_responses_tools(&tools, tool_choice.as_ref())
        .expect("upstream prepareResponsesTools does not reject these tools")
}

/// The upstream whole-result snapshot `{ toolChoice, toolWarnings, tools }`.
/// A key that is `undefined` upstream is absent here.
fn snapshot(result: &PreparedResponsesTools) -> Value {
    let mut object = serde_json::Map::new();
    if let Some(tool_choice) = &result.tool_choice {
        object.insert("toolChoice".to_string(), tool_choice.clone());
    }
    object.insert(
        "toolWarnings".to_string(),
        serde_json::to_value(&result.tool_warnings).expect("warnings serialize"),
    );
    if let Some(tools) = &result.tools {
        object.insert("tools".to_string(), Value::Array(tools.clone()));
    }
    Value::Object(object)
}

mod prepare_responses_tools {
    use super::*;

    mod web_search {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › web_search › should prepare web_search tool with no args
        #[test]
        fn should_prepare_web_search_tool_with_no_args() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]),
                None,
            );

            assert_eq!(
                snapshot(&result),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {
                            "type": "web_search",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › web_search › should prepare web_search tool with allowed domains
        #[test]
        fn should_prepare_web_search_tool_with_allowed_domains() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({ "allowedDomains": ["wikipedia.org", "example.com"] }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "allowed_domains": ["wikipedia.org", "example.com"],
                    "type": "web_search",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › web_search › should prepare web_search tool with excluded domains
        #[test]
        fn should_prepare_web_search_tool_with_excluded_domains() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({ "excludedDomains": ["spam.com"] }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "excluded_domains": ["spam.com"],
                    "type": "web_search",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › web_search › should prepare web_search tool with image search
        #[test]
        fn should_prepare_web_search_tool_with_image_search() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({ "enableImageSearch": true }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "enable_image_search": true,
                    "type": "web_search",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › web_search › should prepare web_search tool with image understanding
        #[test]
        fn should_prepare_web_search_tool_with_image_understanding() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({ "enableImageUnderstanding": true }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "enable_image_understanding": true,
                    "type": "web_search",
                })])
            );
        }
    }

    mod x_search {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › x_search › should prepare x_search tool with no args
        #[test]
        fn should_prepare_x_search_tool_with_no_args() {
            let result = prepare(
                Some(vec![provider_tool("xai.x_search", "x_search", json!({}))]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "type": "x_search",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › x_search › should prepare x_search tool with allowed handles
        #[test]
        fn should_prepare_x_search_tool_with_allowed_handles() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.x_search",
                    "x_search",
                    json!({ "allowedXHandles": ["elonmusk", "xai"] }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "allowed_x_handles": ["elonmusk", "xai"],
                    "type": "x_search",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › x_search › should prepare x_search tool with date range
        #[test]
        fn should_prepare_x_search_tool_with_date_range() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.x_search",
                    "x_search",
                    json!({ "fromDate": "2025-01-01", "toDate": "2025-12-31" }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "from_date": "2025-01-01",
                    "to_date": "2025-12-31",
                    "type": "x_search",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › x_search › should prepare x_search tool with video understanding
        #[test]
        fn should_prepare_x_search_tool_with_video_understanding() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.x_search",
                    "x_search",
                    json!({
                        "enableVideoUnderstanding": true,
                        "enableImageUnderstanding": true,
                    }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "enable_image_understanding": true,
                    "enable_video_understanding": true,
                    "type": "x_search",
                })])
            );
        }
    }

    mod code_execution {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › code_execution › should prepare code_execution tool as code_interpreter
        #[test]
        fn should_prepare_code_execution_tool_as_code_interpreter() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.code_execution",
                    "code_execution",
                    json!({}),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "type": "code_interpreter",
                })])
            );
        }
    }

    mod view_image {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › view_image › should prepare view_image tool
        #[test]
        fn should_prepare_view_image_tool() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.view_image",
                    "view_image",
                    json!({}),
                )]),
                None,
            );

            assert_eq!(result.tools, Some(vec![json!({ "type": "view_image" })]));
        }
    }

    mod view_x_video {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › view_x_video › should prepare view_x_video tool
        #[test]
        fn should_prepare_view_x_video_tool() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.view_x_video",
                    "view_x_video",
                    json!({}),
                )]),
                None,
            );

            assert_eq!(result.tools, Some(vec![json!({ "type": "view_x_video" })]));
        }
    }

    mod image_generation {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › image_generation › should prepare image_generation tool with no args
        #[test]
        fn should_prepare_image_generation_tool_with_no_args() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({}),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "type": "image_generation",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › image_generation › should prepare image_generation tool with action
        #[test]
        fn should_prepare_image_generation_tool_with_action() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({ "action": "generate" }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "action": "generate",
                    "type": "image_generation",
                })])
            );
        }
    }

    mod file_search {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › file_search › should prepare file_search tool with vector store IDs
        #[test]
        fn should_prepare_file_search_tool_with_vector_store_ids() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.file_search",
                    "file_search",
                    json!({ "vectorStoreIds": ["collection_1", "collection_2"] }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "type": "file_search",
                    "vector_store_ids": ["collection_1", "collection_2"],
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › file_search › should prepare file_search tool with max num results
        #[test]
        fn should_prepare_file_search_tool_with_max_num_results() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.file_search",
                    "file_search",
                    json!({
                        "vectorStoreIds": ["collection_1"],
                        "maxNumResults": 10,
                    }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "max_num_results": 10,
                    "type": "file_search",
                    "vector_store_ids": ["collection_1"],
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › file_search › should handle multiple tools including file_search
        #[test]
        fn should_handle_multiple_tools_including_file_search() {
            let result = prepare(
                Some(vec![
                    provider_tool("xai.web_search", "web_search", json!({})),
                    provider_tool(
                        "xai.file_search",
                        "file_search",
                        json!({ "vectorStoreIds": ["collection_1"] }),
                    ),
                    function_tool(
                        "calculator",
                        "calculate numbers",
                        json!({ "type": "object", "properties": {} }),
                    ),
                ]),
                None,
            );

            let tools = result.tools.expect("tools are prepared");
            assert_eq!(tools.len(), 3);
            assert_eq!(tools[0]["type"], "web_search");
            assert_eq!(tools[1]["type"], "file_search");
            assert_eq!(tools[2]["type"], "function");
        }
    }

    mod function_tools {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools › should prepare function tools
        #[test]
        fn should_prepare_function_tools() {
            let result = prepare(
                Some(vec![Tool::Function(FunctionTool {
                    description: Some("get weather information".to_string()),
                    ..FunctionTool::new(
                        "weather",
                        json!({
                            "type": "object",
                            "properties": {
                                "location": { "type": "string" },
                            },
                            "required": ["location"],
                            "additionalProperties": false,
                        }),
                    )
                })]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "description": "get weather information",
                    "name": "weather",
                    "parameters": {
                        "additionalProperties": false,
                        "properties": {
                            "location": {
                                "type": "string",
                            },
                        },
                        "required": ["location"],
                        "type": "object",
                    },
                    "type": "function",
                })])
            );
        }
    }

    mod function_tools_strict_mode {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools strict mode › should pass through strict mode when strict is true
        #[test]
        fn should_pass_through_strict_mode_when_strict_is_true() {
            let result = prepare(
                Some(vec![Tool::Function(FunctionTool {
                    description: Some("A test function".to_string()),
                    strict: Some(true),
                    ..FunctionTool::new(
                        "testFunction",
                        json!({ "type": "object", "properties": {} }),
                    )
                })]),
                None,
            );

            assert_eq!(
                snapshot(&result),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {
                            "description": "A test function",
                            "name": "testFunction",
                            "parameters": {
                                "properties": {},
                                "type": "object",
                            },
                            "strict": true,
                            "type": "function",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools strict mode › should pass through strict mode when strict is false
        #[test]
        fn should_pass_through_strict_mode_when_strict_is_false() {
            let result = prepare(
                Some(vec![Tool::Function(FunctionTool {
                    description: Some("A test function".to_string()),
                    strict: Some(false),
                    ..FunctionTool::new(
                        "testFunction",
                        json!({ "type": "object", "properties": {} }),
                    )
                })]),
                None,
            );

            assert_eq!(
                snapshot(&result),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {
                            "description": "A test function",
                            "name": "testFunction",
                            "parameters": {
                                "properties": {},
                                "type": "object",
                            },
                            "strict": false,
                            "type": "function",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools strict mode › should not include strict when strict is undefined
        #[test]
        fn should_not_include_strict_when_strict_is_undefined() {
            let result = prepare(
                Some(vec![function_tool(
                    "testFunction",
                    "A test function",
                    json!({ "type": "object", "properties": {} }),
                )]),
                None,
            );

            let tools = result.tools.expect("tools are prepared");
            assert!(tools[0].get("strict").is_none());
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools strict mode › should pass through strict mode for multiple tools with different strict settings
        #[test]
        fn should_pass_through_strict_mode_for_multiple_tools_with_different_strict_settings() {
            let result = prepare(
                Some(vec![
                    Tool::Function(FunctionTool {
                        description: Some("A strict tool".to_string()),
                        strict: Some(true),
                        ..FunctionTool::new(
                            "strictTool",
                            json!({ "type": "object", "properties": {} }),
                        )
                    }),
                    Tool::Function(FunctionTool {
                        description: Some("A non-strict tool".to_string()),
                        strict: Some(false),
                        ..FunctionTool::new(
                            "nonStrictTool",
                            json!({ "type": "object", "properties": {} }),
                        )
                    }),
                    function_tool(
                        "defaultTool",
                        "A tool without strict setting",
                        json!({ "type": "object", "properties": {} }),
                    ),
                ]),
                None,
            );

            assert_eq!(
                snapshot(&result),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {
                            "description": "A strict tool",
                            "name": "strictTool",
                            "parameters": {
                                "properties": {},
                                "type": "object",
                            },
                            "strict": true,
                            "type": "function",
                        },
                        {
                            "description": "A non-strict tool",
                            "name": "nonStrictTool",
                            "parameters": {
                                "properties": {},
                                "type": "object",
                            },
                            "strict": false,
                            "type": "function",
                        },
                        {
                            "description": "A tool without strict setting",
                            "name": "defaultTool",
                            "parameters": {
                                "properties": {},
                                "type": "object",
                            },
                            "type": "function",
                        },
                    ],
                })
            );
        }
    }

    mod tool_choice {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › tool choice › should handle tool choice auto
        #[test]
        fn should_handle_tool_choice_auto() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]),
                Some(LanguageModelV4ToolChoice::Auto),
            );

            assert_eq!(result.tool_choice, Some(json!("auto")));
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › tool choice › should handle tool choice required
        #[test]
        fn should_handle_tool_choice_required() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]),
                Some(LanguageModelV4ToolChoice::Required),
            );

            assert_eq!(result.tool_choice, Some(json!("required")));
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › tool choice › should handle tool choice none
        #[test]
        fn should_handle_tool_choice_none() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]),
                Some(LanguageModelV4ToolChoice::None),
            );

            assert_eq!(result.tool_choice, Some(json!("none")));
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › tool choice › should handle specific tool choice
        #[test]
        fn should_handle_specific_tool_choice() {
            let result = prepare(
                Some(vec![function_tool(
                    "weather",
                    "get weather",
                    json!({ "type": "object", "properties": {} }),
                )]),
                Some(LanguageModelV4ToolChoice::Tool {
                    tool_name: "weather".to_string(),
                }),
            );

            assert_eq!(
                result.tool_choice,
                Some(json!({
                    "type": "function",
                    "name": "weather",
                }))
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › tool choice › should warn when trying to force server-side tool via toolChoice
        #[test]
        fn should_warn_when_trying_to_force_server_side_tool_via_tool_choice() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]),
                Some(LanguageModelV4ToolChoice::Tool {
                    tool_name: "web_search".to_string(),
                }),
            );

            assert_eq!(result.tool_choice, None);
            assert!(result.tool_warnings.contains(&Warning::Unsupported {
                feature: "toolChoice for server-side tool \"web_search\"".to_string(),
                details: None,
            }));
        }
    }

    mod multiple_tools {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › multiple tools › should handle multiple tools including provider-defined and functions
        #[test]
        fn should_handle_multiple_tools_including_provider_defined_and_functions() {
            let result = prepare(
                Some(vec![
                    function_tool(
                        "calculator",
                        "calculate numbers",
                        json!({ "type": "object", "properties": {} }),
                    ),
                    provider_tool("xai.web_search", "web_search", json!({})),
                    provider_tool("xai.x_search", "x_search", json!({})),
                ]),
                None,
            );

            let tools = result.tools.expect("tools are prepared");
            assert_eq!(tools.len(), 3);
            assert_eq!(tools[0]["type"], "function");
            assert_eq!(tools[1]["type"], "web_search");
            assert_eq!(tools[2]["type"], "x_search");
        }
    }

    mod empty_tools {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › empty tools › should return undefined for empty tools array
        #[test]
        fn should_return_undefined_for_empty_tools_array() {
            let result = prepare(Some(Vec::new()), None);

            assert_eq!(result.tools, None);
            assert_eq!(result.tool_choice, None);
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › empty tools › should return undefined for undefined tools
        #[test]
        fn should_return_undefined_for_undefined_tools() {
            let result = prepare(None, None);

            assert_eq!(result.tools, None);
        }
    }

    mod unsupported_tools {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › unsupported tools › should warn about unsupported provider-defined tools
        #[test]
        fn should_warn_about_unsupported_provider_defined_tools() {
            let result = prepare(
                Some(vec![provider_tool(
                    "unsupported.tool",
                    "unsupported",
                    json!({}),
                )]),
                None,
            );

            assert_eq!(
                serde_json::to_value(&result.tool_warnings).expect("warnings serialize"),
                json!([
                    {
                        "feature": "provider-defined tool unsupported",
                        "type": "unsupported",
                    },
                ])
            );
        }
    }

    mod mcp {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › mcp › should prepare mcp tool with required args only
        #[test]
        fn should_prepare_mcp_tool_with_required_args_only() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.mcp",
                    "mcp",
                    json!({
                        "serverUrl": "https://example.com/mcp",
                        "serverLabel": "test-server",
                    }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "server_label": "test-server",
                    "server_url": "https://example.com/mcp",
                    "type": "mcp",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › mcp › should prepare mcp tool with all optional args
        #[test]
        fn should_prepare_mcp_tool_with_all_optional_args() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.mcp",
                    "mcp",
                    json!({
                        "serverUrl": "https://example.com/mcp",
                        "serverLabel": "test-server",
                        "serverDescription": "A test MCP server",
                        "allowedTools": ["tool1", "tool2"],
                        "headers": { "X-Custom": "value" },
                        "authorization": "Bearer token123",
                    }),
                )]),
                None,
            );

            assert_eq!(
                result.tools,
                Some(vec![json!({
                    "allowed_tools": ["tool1", "tool2"],
                    "authorization": "Bearer token123",
                    "headers": { "X-Custom": "value" },
                    "server_description": "A test MCP server",
                    "server_label": "test-server",
                    "server_url": "https://example.com/mcp",
                    "type": "mcp",
                })])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › mcp › should warn when trying to force mcp tool via toolChoice
        #[test]
        fn should_warn_when_trying_to_force_mcp_tool_via_tool_choice() {
            let result = prepare(
                Some(vec![provider_tool(
                    "xai.mcp",
                    "mcp",
                    json!({
                        "serverUrl": "https://example.com/mcp",
                        "serverLabel": "test-server",
                    }),
                )]),
                Some(LanguageModelV4ToolChoice::Tool {
                    tool_name: "mcp".to_string(),
                }),
            );

            assert_eq!(result.tool_choice, None);
            assert!(result.tool_warnings.contains(&Warning::Unsupported {
                feature: "toolChoice for server-side tool \"mcp\"".to_string(),
                details: None,
            }));
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-prepare-tools.test.ts › prepareResponsesTools › mcp › should handle multiple tools including mcp
        #[test]
        fn should_handle_multiple_tools_including_mcp() {
            let result = prepare(
                Some(vec![
                    provider_tool("xai.web_search", "web_search", json!({})),
                    provider_tool(
                        "xai.mcp",
                        "mcp",
                        json!({
                            "serverUrl": "https://example.com/mcp",
                            "serverLabel": "test-server",
                        }),
                    ),
                    function_tool(
                        "calculator",
                        "calculate numbers",
                        json!({ "type": "object", "properties": {} }),
                    ),
                ]),
                None,
            );

            let tools = result.tools.expect("tools are prepared");
            assert_eq!(tools.len(), 3);
            assert_eq!(tools[0]["type"], "web_search");
            assert_eq!(tools[1]["type"], "mcp");
            assert_eq!(tools[2]["type"], "function");
        }
    }
}
