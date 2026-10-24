use aimux_core::error::AiMuxError;
use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::Tool;
use serde_json::{Value, json};

use crate::openai::responses::convert::prepare_responses_tools_for;

/// `prepareResponsesTools({ tools, toolChoice, allowedTools,
/// supportsAsyncToolCalling })` as the upstream result object; keys whose
/// value is `undefined` are omitted.
fn prepare_with(
    tools: Value,
    tool_choice: Option<Value>,
    allowed_tools: Option<Value>,
    supports_async: bool,
) -> Result<Value, AiMuxError> {
    let tools: Option<Vec<Tool>> = Some(serde_json::from_value(tools).unwrap());
    let tool_choice: Option<LanguageModelV4ToolChoice> =
        tool_choice.map(|choice| serde_json::from_value(choice).unwrap());
    let prepared = prepare_responses_tools_for(
        &tools,
        tool_choice.as_ref(),
        allowed_tools.as_ref(),
        supports_async,
    )?;
    let mut result = json!({ "toolWarnings": prepared.tool_warnings });
    if let Some(tools) = prepared.tools {
        result["tools"] = json!(tools);
    }
    if let Some(tool_choice) = prepared.tool_choice {
        result["toolChoice"] = tool_choice;
    }
    Ok(result)
}

fn prepare(tools: Value, tool_choice: Option<Value>) -> Value {
    prepare_with(tools, tool_choice, None, true).unwrap()
}

fn prepare_allowed(tools: Value, tool_choice: Option<Value>, allowed_tools: Value) -> Value {
    prepare_with(tools, tool_choice, Some(allowed_tools), true).unwrap()
}

/// `prepareResponsesTools` with a single tool and no tool choice, expecting
/// `{ toolWarnings: [], tools: [expected] }`.
fn assert_single_tool(tool: Value, expected: Value) {
    assert_eq!(
        prepare(json!([tool]), None),
        json!({ "toolWarnings": [], "tools": [expected] })
    );
}

fn test_function_tool() -> Value {
    json!({
        "type": "function",
        "name": "testFunction",
        "description": "A test function",
        "inputSchema": {"type": "object", "properties": {"input": {"type": "string"}}},
    })
}

fn test_function_tool_wire() -> Value {
    json!({
        "description": "A test function",
        "name": "testFunction",
        "parameters": {"properties": {"input": {"type": "string"}}, "type": "object"},
        "strict": false,
        "type": "function",
    })
}

fn assert_err_contains(result: Result<Value, AiMuxError>, message: &str) {
    match result {
        Err(error) => assert!(
            error.to_string().contains(message),
            "expected an error containing {message:?}, got {error:?}"
        ),
        Ok(value) => panic!("expected an error containing {message:?}, got {value}"),
    }
}

mod prepare_responses_tools {
    use super::*;

    /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › should remove string propertyNames from function tools and warn
    #[test]
    fn should_remove_string_property_names_from_function_tools_and_warn() {
        let result = prepare(
            json!([{
                "type": "function",
                "name": "get_weather",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "values": {
                            "type": "object",
                            "propertyNames": {"type": "string", "format": "uuid"},
                        },
                    },
                },
            }]),
            None,
        );

        assert_eq!(
            result,
            json!({
                "tools": [{
                    "type": "function",
                    "name": "get_weather",
                    "parameters": {
                        "type": "object",
                        "properties": {"values": {"type": "object"}},
                    },
                    "strict": false,
                }],
                "toolWarnings": [{
                    "type": "compatibility",
                    "feature": "JSON Schema propertyNames",
                    "details": "OpenAI does not support JSON Schema propertyNames. It was removed before sending the schema, so OpenAI will not enforce property-name constraints.",
                }],
            })
        );
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › should remove regex lookaround patterns from function tools and warn
    #[test]
    fn should_remove_regex_lookaround_patterns_from_function_tools_and_warn() {
        let result = prepare(
            json!([{
                "type": "function",
                "name": "create_contact",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "email": {"type": "string", "format": "email", "pattern": "^(?!\\.).+@.+$"},
                        "username": {"type": "string", "pattern": "^@[a-zA-Z0-9_]+$"},
                    },
                },
            }]),
            None,
        );

        assert_eq!(
            result,
            json!({
                "tools": [{
                    "type": "function",
                    "name": "create_contact",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "email": {"type": "string", "format": "email"},
                            "username": {"type": "string", "pattern": "^@[a-zA-Z0-9_]+$"},
                        },
                    },
                    "strict": false,
                }],
                "toolWarnings": [{
                    "type": "compatibility",
                    "feature": "JSON Schema pattern with regex lookaround",
                    "details": "OpenAI does not support regex lookaround in JSON Schema patterns. The pattern was removed before sending the schema, so OpenAI will not enforce that constraint.",
                }],
            })
        );
    }

    mod async_tools {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › async tools › should pass through async mode for function tools
        #[test]
        fn should_pass_through_async_mode_for_function_tools() {
            let result = prepare(
                json!([{
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get the weather",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"city": {"type": "string"}},
                        "required": ["city"],
                        "additionalProperties": false,
                    },
                    "providerOptions": {"openai": {"async": true}},
                }]),
                None,
            );

            assert_eq!(
                result["tools"],
                json!([{
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get the weather",
                    "parameters": {
                        "type": "object",
                        "properties": {"city": {"type": "string"}},
                        "required": ["city"],
                        "additionalProperties": false,
                    },
                    "async": true,
                    "strict": false,
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › async tools › should pass through async mode for custom tools
        #[test]
        fn should_pass_through_async_mode_for_custom_tools() {
            let result = prepare(
                json!([{
                    "type": "provider",
                    "id": "openai.custom",
                    "name": "write_sql",
                    "args": {
                        "description": "Write a SQL query",
                        "async": true,
                        "format": {"type": "text"},
                    },
                }]),
                None,
            );

            assert_eq!(
                result["tools"],
                json!([{
                    "type": "custom",
                    "name": "write_sql",
                    "description": "Write a SQL query",
                    "async": true,
                    "format": {"type": "text"},
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › async tools › should omit async mode and warn for unsupported models
        #[test]
        fn should_omit_async_mode_and_warn_for_unsupported_models() {
            let result = prepare_with(
                json!([
                    {
                        "type": "function",
                        "name": "get_weather",
                        "inputSchema": {"type": "object", "properties": {}},
                        "providerOptions": {"openai": {"async": true}},
                    },
                    {
                        "type": "provider",
                        "id": "openai.custom",
                        "name": "write_sql",
                        "args": {"async": true},
                    },
                ]),
                None,
                None,
                false,
            )
            .unwrap();

            assert_eq!(
                result["tools"],
                json!([
                    {
                        "type": "function",
                        "name": "get_weather",
                        "parameters": {"type": "object", "properties": {}},
                        "strict": false,
                    },
                    {"type": "custom", "name": "write_sql"},
                ])
            );
            assert_eq!(
                result["toolWarnings"],
                json!([
                    {
                        "type": "unsupported",
                        "feature": "async tool calling for \"get_weather\"",
                        "details": "Async tool calling is only supported by GPT-6 and later models.",
                    },
                    {
                        "type": "unsupported",
                        "feature": "async tool calling for \"write_sql\"",
                        "details": "Async tool calling is only supported by GPT-6 and later models.",
                    },
                ])
            );
        }
    }

    mod function_tools_strict_mode {
        use super::*;

        fn tool(name: &str, description: &str, strict: Option<bool>) -> Value {
            let mut tool = json!({
                "type": "function",
                "name": name,
                "description": description,
                "inputSchema": {"type": "object", "properties": {}},
            });
            if let Some(strict) = strict {
                tool["strict"] = json!(strict);
            }
            tool
        }

        fn wire(name: &str, description: &str, strict: bool) -> Value {
            json!({
                "description": description,
                "name": name,
                "parameters": {"properties": {}, "type": "object"},
                "strict": strict,
                "type": "function",
            })
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools strict mode › should pass through strict mode when strict is true
        #[test]
        fn should_pass_through_strict_mode_when_strict_is_true() {
            assert_single_tool(
                tool("testFunction", "A test function", Some(true)),
                wire("testFunction", "A test function", true),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools strict mode › should pass through strict mode when strict is false
        #[test]
        fn should_pass_through_strict_mode_when_strict_is_false() {
            assert_single_tool(
                tool("testFunction", "A test function", Some(false)),
                wire("testFunction", "A test function", false),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools strict mode › should default strict mode to false when strict is undefined
        #[test]
        fn should_default_strict_mode_to_false_when_strict_is_undefined() {
            assert_single_tool(
                tool("testFunction", "A test function", None),
                wire("testFunction", "A test function", false),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › function tools strict mode › should pass through strict mode for multiple tools with different strict settings
        #[test]
        fn should_pass_through_strict_mode_for_multiple_tools_with_different_strict_settings() {
            let result = prepare(
                json!([
                    tool("strictTool", "A strict tool", Some(true)),
                    tool("nonStrictTool", "A non-strict tool", Some(false)),
                    tool("defaultTool", "A tool without strict setting", None),
                ]),
                None,
            );

            assert_eq!(
                result,
                json!({
                    "toolWarnings": [],
                    "tools": [
                        wire("strictTool", "A strict tool", true),
                        wire("nonStrictTool", "A non-strict tool", false),
                        wire("defaultTool", "A tool without strict setting", false),
                    ],
                })
            );
        }
    }

    mod code_interpreter {
        use super::*;

        fn tool(args: Value) -> Value {
            json!({
                "type": "provider",
                "id": "openai.code_interpreter",
                "name": "code_interpreter",
                "args": args,
            })
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › code interpreter › should prepare code interpreter tool with no container (auto mode)
        #[test]
        fn should_prepare_code_interpreter_tool_with_no_container_auto_mode() {
            assert_single_tool(
                tool(json!({})),
                json!({"container": {"type": "auto"}, "type": "code_interpreter"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › code interpreter › should prepare code interpreter tool with string container ID
        #[test]
        fn should_prepare_code_interpreter_tool_with_string_container_id() {
            assert_single_tool(
                tool(json!({"container": "container-123"})),
                json!({"container": "container-123", "type": "code_interpreter"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › code interpreter › should prepare code interpreter tool with file IDs container
        #[test]
        fn should_prepare_code_interpreter_tool_with_file_ids_container() {
            assert_single_tool(
                tool(json!({"container": {"fileIds": ["file-1", "file-2", "file-3"]}})),
                json!({
                    "container": {"file_ids": ["file-1", "file-2", "file-3"], "type": "auto"},
                    "type": "code_interpreter",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › code interpreter › should prepare code interpreter tool with empty file IDs array
        #[test]
        fn should_prepare_code_interpreter_tool_with_empty_file_ids_array() {
            assert_single_tool(
                tool(json!({"container": {"fileIds": []}})),
                json!({"container": {"file_ids": [], "type": "auto"}, "type": "code_interpreter"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › code interpreter › should prepare code interpreter tool with undefined file IDs
        #[test]
        fn should_prepare_code_interpreter_tool_with_undefined_file_ids() {
            // `fileIds: undefined` is an absent key.
            assert_single_tool(
                tool(json!({"container": {}})),
                json!({"container": {"type": "auto"}, "type": "code_interpreter"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › code interpreter › should handle tool choice selection with code interpreter
        #[test]
        fn should_handle_tool_choice_selection_with_code_interpreter() {
            assert_eq!(
                prepare(
                    json!([tool(json!({}))]),
                    Some(json!({"type": "tool", "toolName": "code_interpreter"})),
                ),
                json!({
                    "toolChoice": {"type": "code_interpreter"},
                    "toolWarnings": [],
                    "tools": [{"container": {"type": "auto"}, "type": "code_interpreter"}],
                })
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › code interpreter › should handle multiple tools including code interpreter
        #[test]
        fn should_handle_multiple_tools_including_code_interpreter() {
            assert_eq!(
                prepare(
                    json!([
                        test_function_tool(),
                        tool(json!({"container": "my-container"}))
                    ]),
                    None,
                ),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        test_function_tool_wire(),
                        {"container": "my-container", "type": "code_interpreter"},
                    ],
                })
            );
        }
    }

    mod image_generation {
        use super::*;

        fn tool(args: Value) -> Value {
            json!({
                "type": "provider",
                "id": "openai.image_generation",
                "name": "image_generation",
                "args": args,
            })
        }

        mod quality {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › image generation › %s quality › should pass %s quality
            #[test]
            fn should_pass_quality() {
                for model in ["gpt-image-2.5-flare", "gpt-image-2.5-sunburst"] {
                    for quality in ["xhigh", "max"] {
                        let args: crate::openai::tools::ImageGenerationArgs =
                            serde_json::from_value(json!({"model": model, "quality": quality}))
                                .unwrap();
                        let image_tool = crate::openai::tools::image_generation(args).unwrap();
                        let result = prepare(
                            json!([{
                                "type": "provider",
                                "id": "openai.image_generation",
                                "name": "image_generation",
                                "args": image_tool.args,
                            }]),
                            None,
                        );

                        assert_eq!(result["toolWarnings"], json!([]), "{model} {quality}");
                        let tools = result["tools"].as_array().unwrap();
                        assert_eq!(tools.len(), 1, "{model} {quality}");
                        assert_eq!(tools[0]["type"], "image_generation", "{model} {quality}");
                        assert_eq!(tools[0]["model"], model, "{model} {quality}");
                        assert_eq!(tools[0]["quality"], quality, "{model} {quality}");
                    }
                }
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › image generation › should prepare image_generation tool with all options
        #[test]
        fn should_prepare_image_generation_tool_with_all_options() {
            assert_single_tool(
                tool(json!({
                    "background": "opaque",
                    "size": "1536x1024",
                    "quality": "high",
                    "moderation": "auto",
                    "outputFormat": "png",
                    "outputCompression": 100,
                })),
                json!({
                    "background": "opaque",
                    "moderation": "auto",
                    "output_compression": 100,
                    "output_format": "png",
                    "quality": "high",
                    "size": "1536x1024",
                    "type": "image_generation",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › image generation › should pass action, low moderation and a gpt-image-2 size
        #[test]
        fn should_pass_action_low_moderation_and_a_gpt_image_2_size() {
            let result = prepare(
                json!([tool(json!({
                    "action": "edit",
                    "model": "gpt-image-2",
                    "moderation": "low",
                    "size": "1536x864",
                }))]),
                None,
            );

            assert_eq!(result["toolWarnings"], json!([]));
            assert_eq!(
                result["tools"],
                json!([{
                    "action": "edit",
                    "model": "gpt-image-2",
                    "moderation": "low",
                    "size": "1536x864",
                    "type": "image_generation",
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › image generation › should reject a size that is not WIDTHxHEIGHT
        #[test]
        fn should_reject_a_size_that_is_not_widthxheight() {
            let result = prepare_with(json!([tool(json!({"size": "large"}))]), None, None, true);
            assert!(result.is_err(), "{result:?}");
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › image generation › should support tool choice selection for image_generation
        #[test]
        fn should_support_tool_choice_selection_for_image_generation() {
            let result = prepare(
                json!([tool(json!({}))]),
                Some(json!({"type": "tool", "toolName": "image_generation"})),
            );

            assert_eq!(result["tools"], json!([{"type": "image_generation"}]));
        }
    }

    mod local_shell {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › local shell › should prepare local_shell tool
        #[test]
        fn should_prepare_local_shell_tool() {
            assert_single_tool(
                json!({"type": "provider", "id": "openai.local_shell", "name": "local_shell", "args": {}}),
                json!({"type": "local_shell"}),
            );
        }
    }

    mod web_search {
        use super::*;

        fn tool(args: Value) -> Value {
            json!({"type": "provider", "id": "openai.web_search", "name": "web_search", "args": args})
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › web search › should prepare web_search tool with no options
        #[test]
        fn should_prepare_web_search_tool_with_no_options() {
            assert_single_tool(tool(json!({})), json!({"type": "web_search"}));
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › web search › should prepare web_search tool with externalWebAccess set to true
        #[test]
        fn should_prepare_web_search_tool_with_external_web_access_set_to_true() {
            assert_single_tool(
                tool(json!({"externalWebAccess": true})),
                json!({"external_web_access": true, "type": "web_search"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › web search › should prepare web_search tool with externalWebAccess set to false
        #[test]
        fn should_prepare_web_search_tool_with_external_web_access_set_to_false() {
            assert_single_tool(
                tool(json!({"externalWebAccess": false})),
                json!({"external_web_access": false, "type": "web_search"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › web search › should prepare web_search tool with all options including externalWebAccess
        #[test]
        fn should_prepare_web_search_tool_with_all_options_including_external_web_access() {
            assert_single_tool(
                tool(json!({
                    "externalWebAccess": true,
                    "filters": {
                        "allowedDomains": ["example.com", "test.org"],
                        "blockedDomains": ["blocked.example", "blocked.test"],
                    },
                    "searchContextSize": "high",
                    "userLocation": {
                        "type": "approximate",
                        "country": "US",
                        "city": "San Francisco",
                        "region": "California",
                        "timezone": "America/Los_Angeles",
                    },
                })),
                json!({
                    "external_web_access": true,
                    "filters": {
                        "allowed_domains": ["example.com", "test.org"],
                        "blocked_domains": ["blocked.example", "blocked.test"],
                    },
                    "search_context_size": "high",
                    "type": "web_search",
                    "user_location": {
                        "city": "San Francisco",
                        "country": "US",
                        "region": "California",
                        "timezone": "America/Los_Angeles",
                        "type": "approximate",
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › web search › should prepare web_search tool with blocked domains
        #[test]
        fn should_prepare_web_search_tool_with_blocked_domains() {
            assert_single_tool(
                tool(json!({"filters": {"blockedDomains": ["example.com"]}})),
                json!({"filters": {"blocked_domains": ["example.com"]}, "type": "web_search"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › web search › should handle tool choice selection with web_search
        #[test]
        fn should_handle_tool_choice_selection_with_web_search() {
            assert_eq!(
                prepare(
                    json!([tool(json!({"externalWebAccess": true}))]),
                    Some(json!({"type": "tool", "toolName": "web_search"})),
                ),
                json!({
                    "toolChoice": {"type": "web_search"},
                    "toolWarnings": [],
                    "tools": [{"external_web_access": true, "type": "web_search"}],
                })
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › web search › should handle multiple tools including web_search
        #[test]
        fn should_handle_multiple_tools_including_web_search() {
            assert_eq!(
                prepare(
                    json!([
                        test_function_tool(),
                        tool(json!({"externalWebAccess": false, "searchContextSize": "medium"})),
                    ]),
                    None,
                ),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        test_function_tool_wire(),
                        {
                            "external_web_access": false,
                            "search_context_size": "medium",
                            "type": "web_search",
                        },
                    ],
                })
            );
        }
    }

    mod shell {
        use super::*;

        fn tool(args: Value) -> Value {
            json!({"type": "provider", "id": "openai.shell", "name": "shell", "args": args})
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool without environment args
        #[test]
        fn should_prepare_shell_tool_without_environment_args() {
            assert_single_tool(tool(json!({})), json!({"type": "shell"}));
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with containerAuto without skills
        #[test]
        fn should_prepare_shell_tool_with_container_auto_without_skills() {
            assert_single_tool(
                tool(json!({"environment": {"type": "containerAuto"}})),
                json!({"environment": {"type": "container_auto"}, "type": "shell"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with containerAuto and providerReference skills
        #[test]
        fn should_prepare_shell_tool_with_container_auto_and_provider_reference_skills() {
            assert_single_tool(
                tool(json!({"environment": {
                    "type": "containerAuto",
                    "skills": [{
                        "type": "skillReference",
                        "providerReference": {"openai": "skill_abc"},
                        "version": "1.0.0",
                    }],
                }})),
                json!({
                    "environment": {
                        "skills": [{"skill_id": "skill_abc", "type": "skill_reference", "version": "1.0.0"}],
                        "type": "container_auto",
                    },
                    "type": "shell",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should default shell skillReference version to latest when omitted
        #[test]
        fn should_default_shell_skill_reference_version_to_latest_when_omitted() {
            assert_single_tool(
                tool(json!({"environment": {
                    "type": "containerAuto",
                    "skills": [{"type": "skillReference", "providerReference": {"openai": "skill_abc"}}],
                }})),
                json!({
                    "environment": {
                        "skills": [{"skill_id": "skill_abc", "type": "skill_reference", "version": "latest"}],
                        "type": "container_auto",
                    },
                    "type": "shell",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should throw when a providerReference cannot be resolved for openai
        #[test]
        fn should_throw_when_a_provider_reference_cannot_be_resolved_for_openai() {
            // aimux has no `NoSuchProviderReferenceError` type; its message
            // names the provider (`openai`) and the providers of the reference.
            let result = prepare_with(
                json!([tool(json!({"environment": {
                    "type": "containerAuto",
                    "skills": [{"type": "skillReference", "providerReference": {"anthropic": "skill_abc"}}],
                }}))]),
                None,
                None,
                true,
            );
            let error = result.expect_err("should have thrown");
            let message = error.to_string();
            assert!(
                message.contains(
                    "No provider reference found for provider 'openai'. Available providers: anthropic"
                ),
                "{message}"
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with containerAuto and inline skill
        #[test]
        fn should_prepare_shell_tool_with_container_auto_and_inline_skill() {
            assert_single_tool(
                tool(json!({"environment": {
                    "type": "containerAuto",
                    "skills": [{
                        "type": "inline",
                        "name": "my-skill",
                        "description": "A test skill",
                        "source": {"type": "base64", "mediaType": "application/zip", "data": "dGVzdA=="},
                    }],
                }})),
                json!({
                    "environment": {
                        "skills": [{
                            "description": "A test skill",
                            "name": "my-skill",
                            "source": {"data": "dGVzdA==", "media_type": "application/zip", "type": "base64"},
                            "type": "inline",
                        }],
                        "type": "container_auto",
                    },
                    "type": "shell",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with containerAuto and networkPolicy disabled
        #[test]
        fn should_prepare_shell_tool_with_container_auto_and_network_policy_disabled() {
            assert_single_tool(
                tool(
                    json!({"environment": {"type": "containerAuto", "networkPolicy": {"type": "disabled"}}}),
                ),
                json!({
                    "environment": {"network_policy": {"type": "disabled"}, "type": "container_auto"},
                    "type": "shell",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with containerAuto and networkPolicy allowlist with domain secrets
        #[test]
        fn should_prepare_shell_tool_with_container_auto_and_network_policy_allowlist_with_domain_secrets()
         {
            assert_single_tool(
                tool(json!({"environment": {
                    "type": "containerAuto",
                    "networkPolicy": {
                        "type": "allowlist",
                        "allowedDomains": ["example.com", "api.test.org"],
                        "domainSecrets": [{"domain": "api.test.org", "name": "API_KEY", "value": "secret123"}],
                    },
                }})),
                json!({
                    "environment": {
                        "network_policy": {
                            "allowed_domains": ["example.com", "api.test.org"],
                            "domain_secrets": [{"domain": "api.test.org", "name": "API_KEY", "value": "secret123"}],
                            "type": "allowlist",
                        },
                        "type": "container_auto",
                    },
                    "type": "shell",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with containerAuto, fileIds, and memoryLimit
        #[test]
        fn should_prepare_shell_tool_with_container_auto_file_ids_and_memory_limit() {
            assert_single_tool(
                tool(json!({"environment": {
                    "type": "containerAuto",
                    "fileIds": ["file-1", "file-2"],
                    "memoryLimit": "16g",
                }})),
                json!({
                    "environment": {
                        "file_ids": ["file-1", "file-2"],
                        "memory_limit": "16g",
                        "type": "container_auto",
                    },
                    "type": "shell",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with containerReference
        #[test]
        fn should_prepare_shell_tool_with_container_reference() {
            assert_single_tool(
                tool(
                    json!({"environment": {"type": "containerReference", "containerId": "ctr_abc123"}}),
                ),
                json!({
                    "environment": {"container_id": "ctr_abc123", "type": "container_reference"},
                    "type": "shell",
                }),
            );
        }

        fn calculator_skill() -> Value {
            json!({
                "description": "Perform math calculations",
                "name": "calculator",
                "path": "/path/to/calculator",
            })
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with local environment and skills
        #[test]
        fn should_prepare_shell_tool_with_local_environment_and_skills() {
            assert_single_tool(
                tool(json!({"environment": {"type": "local", "skills": [calculator_skill()]}})),
                json!({
                    "environment": {"skills": [calculator_skill()], "type": "local"},
                    "type": "shell",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with local environment without explicit type
        #[test]
        fn should_prepare_shell_tool_with_local_environment_without_explicit_type() {
            assert_single_tool(
                tool(json!({"environment": {"skills": [calculator_skill()]}})),
                json!({
                    "environment": {"skills": [calculator_skill()], "type": "local"},
                    "type": "shell",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › shell › should prepare shell tool with local environment without skills
        #[test]
        fn should_prepare_shell_tool_with_local_environment_without_skills() {
            assert_single_tool(
                tool(json!({"environment": {"type": "local"}})),
                json!({"environment": {"type": "local"}, "type": "shell"}),
            );
        }
    }

    mod custom_tool {
        use super::*;

        fn tool(name: &str, args: Value) -> Value {
            json!({"type": "provider", "id": "openai.custom", "name": name, "args": args})
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › custom tool › should prepare custom tool with regex format
        #[test]
        fn should_prepare_custom_tool_with_regex_format() {
            assert_single_tool(
                tool(
                    "write_sql",
                    json!({
                        "description": "Write a SQL SELECT query.",
                        "format": {"type": "grammar", "syntax": "regex", "definition": "SELECT .+"},
                    }),
                ),
                json!({
                    "description": "Write a SQL SELECT query.",
                    "format": {"definition": "SELECT .+", "syntax": "regex", "type": "grammar"},
                    "name": "write_sql",
                    "type": "custom",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › custom tool › should prepare custom tool with lark format
        #[test]
        fn should_prepare_custom_tool_with_lark_format() {
            assert_single_tool(
                tool(
                    "generate_json",
                    json!({
                        "format": {"type": "grammar", "syntax": "lark", "definition": "start: \"{\"  \"}\""},
                    }),
                ),
                json!({
                    "format": {"definition": "start: \"{\"  \"}\"", "syntax": "lark", "type": "grammar"},
                    "name": "generate_json",
                    "type": "custom",
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › custom tool › should handle multiple tools including custom tool
        #[test]
        fn should_handle_multiple_tools_including_custom_tool() {
            assert_eq!(
                prepare(
                    json!([
                        test_function_tool(),
                        tool(
                            "write_sql",
                            json!({
                                "description": "Write SQL.",
                                "format": {"type": "grammar", "syntax": "regex", "definition": "SELECT .+"},
                            }),
                        ),
                    ]),
                    None,
                ),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        test_function_tool_wire(),
                        {
                            "description": "Write SQL.",
                            "format": {"definition": "SELECT .+", "syntax": "regex", "type": "grammar"},
                            "name": "write_sql",
                            "type": "custom",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › custom tool › should resolve custom tool choice using tool name
        #[test]
        fn should_resolve_custom_tool_choice_using_tool_name() {
            let result = prepare(
                json!([tool("write_sql", json!({}))]),
                Some(json!({"type": "tool", "toolName": "write_sql"})),
            );
            assert_eq!(
                result["toolChoice"],
                json!({"type": "custom", "name": "write_sql"})
            );
        }
    }

    mod computer {
        use super::*;

        fn tool() -> Value {
            json!({"type": "provider", "id": "openai.computer", "name": "computer", "args": {}})
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › computer › should prepare computer tool
        #[test]
        fn should_prepare_computer_tool() {
            assert_eq!(
                prepare(json!([tool()]), None),
                json!({"tools": [{"type": "computer"}], "toolWarnings": []})
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › computer › should handle computer tool choice
        #[test]
        fn should_handle_computer_tool_choice() {
            let result = prepare(
                json!([tool()]),
                Some(json!({"type": "tool", "toolName": "computer"})),
            );
            assert_eq!(result["toolChoice"], json!({"type": "computer"}));
        }
    }

    mod apply_patch {
        use super::*;

        fn tool() -> Value {
            json!({"type": "provider", "id": "openai.apply_patch", "name": "apply_patch", "args": {}})
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › apply_patch › should prepare apply_patch tool
        #[test]
        fn should_prepare_apply_patch_tool() {
            assert_single_tool(tool(), json!({"type": "apply_patch"}));
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › apply_patch › should handle tool choice selection with apply_patch
        #[test]
        fn should_handle_tool_choice_selection_with_apply_patch() {
            assert_eq!(
                prepare(
                    json!([tool()]),
                    Some(json!({"type": "tool", "toolName": "apply_patch"})),
                ),
                json!({
                    "toolChoice": {"type": "apply_patch"},
                    "toolWarnings": [],
                    "tools": [{"type": "apply_patch"}],
                })
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › apply_patch › should handle multiple tools including apply_patch
        #[test]
        fn should_handle_multiple_tools_including_apply_patch() {
            assert_eq!(
                prepare(json!([test_function_tool(), tool()]), None),
                json!({
                    "toolWarnings": [],
                    "tools": [test_function_tool_wire(), {"type": "apply_patch"}],
                })
            );
        }
    }

    mod tool_search {
        use super::*;

        fn tool_search() -> Value {
            json!({"type": "provider", "id": "openai.tool_search", "name": "toolSearch", "args": {}})
        }

        fn location_schema() -> Value {
            json!({
                "type": "object",
                "properties": {"location": {"type": "string"}},
                "required": ["location"],
                "additionalProperties": false,
            })
        }

        fn customer_schema() -> Value {
            json!({
                "type": "object",
                "properties": {"customer_id": {"type": "string"}},
                "required": ["customer_id"],
                "additionalProperties": false,
            })
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › tool search › should prepare tool_search tool
        #[test]
        fn should_prepare_tool_search_tool() {
            assert_single_tool(tool_search(), json!({"type": "tool_search"}));
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › tool search › should prepare tool_search alongside function tools with defer_loading
        #[test]
        fn should_prepare_tool_search_alongside_function_tools_with_defer_loading() {
            assert_eq!(
                prepare(
                    json!([
                        tool_search(),
                        {
                            "type": "function",
                            "name": "get_weather",
                            "description": "Get the current weather",
                            "inputSchema": location_schema(),
                            "providerOptions": {"openai": {"deferLoading": true}},
                        },
                    ]),
                    None,
                ),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {"type": "tool_search"},
                        {
                            "defer_loading": true,
                            "description": "Get the current weather",
                            "name": "get_weather",
                            "parameters": location_schema(),
                            "strict": false,
                            "type": "function",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › tool search › should group function tools by OpenAI namespace provider option
        #[test]
        fn should_group_function_tools_by_open_ai_namespace_provider_option() {
            let namespace = json!({
                "name": "crm",
                "description": "CRM tools for customer lookup and order management.",
            });
            assert_eq!(
                prepare(
                    json!([
                        tool_search(),
                        {
                            "type": "function",
                            "name": "get_customer_profile",
                            "description": "Fetch a customer profile by customer ID.",
                            "inputSchema": customer_schema(),
                            "providerOptions": {"openai": {"namespace": namespace}},
                        },
                        {
                            "type": "function",
                            "name": "get_weather",
                            "description": "Get the current weather",
                            "inputSchema": location_schema(),
                        },
                        {
                            "type": "function",
                            "name": "list_open_orders",
                            "description": "List open orders for a customer ID.",
                            "inputSchema": customer_schema(),
                            "strict": true,
                            "providerOptions": {"openai": {"deferLoading": true, "namespace": namespace}},
                        },
                    ]),
                    None,
                ),
                json!({
                    "toolWarnings": [],
                    "tools": [
                        {"type": "tool_search"},
                        {
                            "description": "CRM tools for customer lookup and order management.",
                            "name": "crm",
                            "tools": [
                                {
                                    "description": "Fetch a customer profile by customer ID.",
                                    "name": "get_customer_profile",
                                    "parameters": customer_schema(),
                                    "strict": false,
                                    "type": "function",
                                },
                                {
                                    "defer_loading": true,
                                    "description": "List open orders for a customer ID.",
                                    "name": "list_open_orders",
                                    "parameters": customer_schema(),
                                    "strict": true,
                                    "type": "function",
                                },
                            ],
                            "type": "namespace",
                        },
                        {
                            "description": "Get the current weather",
                            "name": "get_weather",
                            "parameters": location_schema(),
                            "strict": false,
                            "type": "function",
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › tool search › should reject conflicting descriptions for the same OpenAI namespace
        #[test]
        fn should_reject_conflicting_descriptions_for_the_same_open_ai_namespace() {
            let result = prepare_with(
                json!([
                    {
                        "type": "function",
                        "name": "get_customer_profile",
                        "description": "Fetch a customer profile by customer ID.",
                        "inputSchema": {"type": "object", "properties": {}},
                        "providerOptions": {"openai": {"namespace": {"name": "crm", "description": "CRM tools."}}},
                    },
                    {
                        "type": "function",
                        "name": "list_open_orders",
                        "description": "List open orders for a customer ID.",
                        "inputSchema": {"type": "object", "properties": {}},
                        "providerOptions": {"openai": {"namespace": {"name": "crm", "description": "Different CRM tools."}}},
                    },
                ]),
                None,
                None,
                true,
            );
            assert_err_contains(
                result,
                "conflicting descriptions for OpenAI tool namespace \"crm\"",
            );
        }
    }

    mod allowed_tools_provider_option {
        use super::*;

        fn function_tool() -> Value {
            json!({
                "type": "function",
                "name": "get_weather",
                "description": "Get weather",
                "inputSchema": {"type": "object", "properties": {}},
            })
        }

        fn time_tool() -> Value {
            json!({
                "type": "function",
                "name": "get_time",
                "description": "Get time",
                "inputSchema": {"type": "object", "properties": {}},
            })
        }

        fn web_search_tool() -> Value {
            json!({"type": "provider", "id": "openai.web_search", "name": "search", "args": {}})
        }

        fn mcp_tool(name: &str, server_label: &str) -> Value {
            json!({
                "type": "provider",
                "id": "openai.mcp",
                "name": name,
                "args": {
                    "serverLabel": server_label,
                    "serverUrl": format!("https://{server_label}.example.com/mcp"),
                },
            })
        }

        // Upstream passes a `toolNameMapping` mirroring the provider-tool
        // registry; aimux derives the same mapping from each provider tool's
        // id, so those cases need no extra argument.

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › should emit allowed_tools with default auto mode
        #[test]
        fn should_emit_allowed_tools_with_default_auto_mode() {
            let result = prepare_allowed(
                json!([function_tool(), time_tool()]),
                None,
                json!({"toolNames": ["get_weather"]}),
            );
            assert_eq!(
                result["toolChoice"],
                json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "get_weather"}]})
            );
            assert_eq!(result["tools"].as_array().unwrap().len(), 2);
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › should emit allowed_tools with required mode
        #[test]
        fn should_emit_allowed_tools_with_required_mode() {
            let result = prepare_allowed(
                json!([function_tool(), time_tool()]),
                None,
                json!({"toolNames": ["get_weather", "get_time"], "mode": "required"}),
            );
            assert_eq!(
                result["toolChoice"],
                json!({
                    "type": "allowed_tools",
                    "mode": "required",
                    "tools": [
                        {"type": "function", "name": "get_weather"},
                        {"type": "function", "name": "get_time"},
                    ],
                })
            );
            assert_eq!(result["tools"].as_array().unwrap().len(), 2);
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › should override request-level toolChoice when allowedTools is set
        #[test]
        fn should_override_request_level_tool_choice_when_allowed_tools_is_set() {
            let result = prepare_allowed(
                json!([function_tool()]),
                Some(json!({"type": "required"})),
                json!({"toolNames": ["get_weather"]}),
            );
            assert_eq!(
                result["toolChoice"],
                json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "get_weather"}]})
            );
        }

        mod entry_shapes {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › entry shapes › should emit the correct allowed_tools entry for %s
            #[test]
            fn should_emit_the_correct_allowed_tools_entry_for() {
                for (label, tool, allowed_name, expected_entry) in [
                    (
                        "web search",
                        json!({"type": "provider", "id": "openai.web_search", "name": "search", "args": {}}),
                        "search",
                        json!({"type": "web_search"}),
                    ),
                    (
                        "file search",
                        json!({"type": "provider", "id": "openai.file_search", "name": "file_search", "args": {"vectorStoreIds": ["vs-1"]}}),
                        "file_search",
                        json!({"type": "file_search"}),
                    ),
                    (
                        "web search preview",
                        json!({"type": "provider", "id": "openai.web_search_preview", "name": "web_search_preview", "args": {}}),
                        "web_search_preview",
                        json!({"type": "web_search_preview"}),
                    ),
                    (
                        "image generation",
                        json!({"type": "provider", "id": "openai.image_generation", "name": "image_generation", "args": {}}),
                        "image_generation",
                        json!({"type": "image_generation"}),
                    ),
                    (
                        "code interpreter",
                        json!({"type": "provider", "id": "openai.code_interpreter", "name": "code_interpreter", "args": {}}),
                        "code_interpreter",
                        json!({"type": "code_interpreter"}),
                    ),
                    (
                        "apply patch",
                        json!({"type": "provider", "id": "openai.apply_patch", "name": "apply_patch", "args": {}}),
                        "apply_patch",
                        json!({"type": "apply_patch"}),
                    ),
                    (
                        "shell",
                        json!({"type": "provider", "id": "openai.shell", "name": "shell", "args": {}}),
                        "shell",
                        json!({"type": "shell"}),
                    ),
                    (
                        "computer",
                        json!({"type": "provider", "id": "openai.computer", "name": "computer", "args": {}}),
                        "computer",
                        json!({"type": "computer"}),
                    ),
                    (
                        "mcp (carries its server label)",
                        json!({"type": "provider", "id": "openai.mcp", "name": "deepwiki", "args": {"serverLabel": "deepwiki", "serverUrl": "https://mcp.deepwiki.com/mcp"}}),
                        "deepwiki",
                        json!({"type": "mcp", "server_label": "deepwiki"}),
                    ),
                    (
                        "custom (carries its name)",
                        json!({"type": "provider", "id": "openai.custom", "name": "write_sql", "args": {"description": "Write a SQL SELECT query."}}),
                        "write_sql",
                        json!({"type": "custom", "name": "write_sql"}),
                    ),
                ] {
                    let result =
                        prepare_allowed(json!([tool]), None, json!({"toolNames": [allowed_name]}));
                    assert_eq!(
                        result["toolChoice"],
                        json!({"type": "allowed_tools", "mode": "auto", "tools": [expected_entry]}),
                        "{label}"
                    );
                    assert_eq!(result["toolWarnings"], json!([]), "{label}");
                }
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › should preserve the order of mixed function and built-in entries without deduplicating
        #[test]
        fn should_preserve_the_order_of_mixed_function_and_built_in_entries_without_deduplicating()
        {
            let result = prepare_allowed(
                json!([function_tool(), web_search_tool()]),
                None,
                json!({"toolNames": ["search", "get_weather", "search"]}),
            );
            assert_eq!(
                result["toolChoice"],
                json!({
                    "type": "allowed_tools",
                    "mode": "auto",
                    "tools": [
                        {"type": "web_search"},
                        {"type": "function", "name": "get_weather"},
                        {"type": "web_search"},
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › should resolve a built-in tool by its canonical provider name
        #[test]
        fn should_resolve_a_built_in_tool_by_its_canonical_provider_name() {
            let result = prepare_allowed(
                json!([web_search_tool()]),
                None,
                json!({"toolNames": ["web_search"]}),
            );
            assert_eq!(
                result["toolChoice"],
                json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "web_search"}]})
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › should prefer the user tool name over a canonical name alias on collision
        #[test]
        fn should_prefer_the_user_tool_name_over_a_canonical_name_alias_on_collision() {
            let result = prepare_allowed(
                json!([
                    {
                        "type": "function",
                        "name": "web_search",
                        "description": "A function that happens to be named web_search",
                        "inputSchema": {"type": "object", "properties": {}},
                    },
                    web_search_tool(),
                ]),
                None,
                json!({"toolNames": ["web_search"]}),
            );
            assert_eq!(
                result["toolChoice"],
                json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "web_search"}]})
            );
            assert_eq!(
                result["toolWarnings"],
                json!([{
                    "type": "unsupported",
                    "feature": "allowedTools entry \"web_search\"",
                    "details": "this name is both a tool name and the provider tool name of another tool in this request; the tool with this name is allowed",
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › should not resolve a function tool by the literal name "function"
        #[test]
        fn should_not_resolve_a_function_tool_by_the_literal_name_function() {
            let result = prepare_allowed(
                json!([function_tool()]),
                None,
                json!({"toolNames": ["function"]}),
            );
            assert_eq!(
                result["toolChoice"],
                json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "function"}]})
            );
            assert_eq!(
                result["toolWarnings"],
                json!([{
                    "type": "unsupported",
                    "feature": "allowedTools entry \"function\"",
                    "details": "the tool is not part of the tools for this request and is sent as a function tool",
                }])
            );
        }

        mod tools_that_cannot_be_allow_listed {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › tools that cannot be allow-listed › should drop %s from the allow-list and warn
            #[test]
            fn should_drop_from_the_allow_list_and_warn() {
                for (label, problem_tool, allowed_name, expected_details) in [
                    (
                        "a deferred tool",
                        json!({
                            "type": "function",
                            "name": "problem_tool",
                            "description": "Deferred",
                            "inputSchema": {"type": "object", "properties": {}},
                            "providerOptions": {"openai": {"deferLoading": true}},
                        }),
                        "problem_tool",
                        "deferred tools are not visible to tool_choice.allowed_tools; the tool is removed from the allowed tools",
                    ),
                    (
                        "a namespaced tool",
                        json!({
                            "type": "function",
                            "name": "problem_tool",
                            "description": "Namespaced",
                            "inputSchema": {"type": "object", "properties": {}},
                            "providerOptions": {"openai": {"namespace": {"name": "crm", "description": "CRM tools"}}},
                        }),
                        "problem_tool",
                        "tools inside an OpenAI tool namespace are not visible to tool_choice.allowed_tools; the tool is removed from the allowed tools",
                    ),
                    (
                        "the tool search tool",
                        json!({"type": "provider", "id": "openai.tool_search", "name": "problem_tool", "args": {}}),
                        "problem_tool",
                        "OpenAI does not support tool_search tools in tool_choice.allowed_tools; the tool is removed from the allowed tools",
                    ),
                ] {
                    let result = prepare_allowed(
                        json!([function_tool(), problem_tool]),
                        None,
                        json!({"toolNames": ["get_weather", allowed_name]}),
                    );
                    assert_eq!(
                        result["toolChoice"],
                        json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "get_weather"}]}),
                        "{label}"
                    );
                    assert_eq!(
                        result["toolWarnings"],
                        json!([{
                            "type": "unsupported",
                            "feature": format!("allowedTools entry \"{allowed_name}\""),
                            "details": expected_details,
                        }]),
                        "{label}"
                    );
                }
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › tools that cannot be allow-listed › should drop an unlistable provider tool referenced by its canonical name
            #[test]
            fn should_drop_an_unlistable_provider_tool_referenced_by_its_canonical_name() {
                let result = prepare_allowed(
                    json!([
                        function_tool(),
                        {"type": "provider", "id": "openai.tool_search", "name": "my_search", "args": {}},
                    ]),
                    None,
                    json!({"toolNames": ["get_weather", "tool_search"]}),
                );
                assert_eq!(
                    result["toolChoice"],
                    json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "get_weather"}]})
                );
                assert_eq!(
                    result["toolWarnings"],
                    json!([{
                        "type": "unsupported",
                        "feature": "allowedTools entry \"tool_search\"",
                        "details": "OpenAI does not support tool_search tools in tool_choice.allowed_tools; the tool is removed from the allowed tools",
                    }])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › tools that cannot be allow-listed › should drop an ambiguous canonical name when several tools share it
            #[test]
            fn should_drop_an_ambiguous_canonical_name_when_several_tools_share_it() {
                let result = prepare_allowed(
                    json!([
                        function_tool(),
                        mcp_tool("alpha", "alpha"),
                        mcp_tool("beta", "beta")
                    ]),
                    None,
                    json!({"toolNames": ["get_weather", "mcp"]}),
                );
                assert_eq!(
                    result["toolChoice"],
                    json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "get_weather"}]})
                );
                assert_eq!(
                    result["toolWarnings"],
                    json!([{
                        "type": "unsupported",
                        "feature": "allowedTools entry \"mcp\"",
                        "details": "several tools in this request share this provider tool name; use the tool name from the tools for this request instead",
                    }])
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › tools that cannot be allow-listed › should still resolve each mcp server by its own tool name
            #[test]
            fn should_still_resolve_each_mcp_server_by_its_own_tool_name() {
                let result = prepare_allowed(
                    json!([mcp_tool("alpha", "alpha"), mcp_tool("beta", "beta")]),
                    None,
                    json!({"toolNames": ["beta"]}),
                );
                assert_eq!(
                    result["toolChoice"],
                    json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "mcp", "server_label": "beta"}]})
                );
                assert_eq!(result["toolWarnings"], json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › tools that cannot be allow-listed › should throw when no allowed tool can be expressed
            #[test]
            fn should_throw_when_no_allowed_tool_can_be_expressed() {
                let result = prepare_with(
                    json!([
                        {"type": "provider", "id": "openai.tool_search", "name": "tool_search", "args": {}},
                        {
                            "type": "function",
                            "name": "deferred_tool",
                            "description": "Deferred",
                            "inputSchema": {"type": "object", "properties": {}},
                            "providerOptions": {"openai": {"deferLoading": true}},
                        },
                    ]),
                    None,
                    Some(json!({"toolNames": ["tool_search", "deferred_tool"]})),
                    true,
                );
                assert_err_contains(
                    result,
                    "allowedTools with only tools that cannot be allow-listed (tool_search, deferred_tool)",
                );
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › allowedTools provider option › should warn and still send an entry for a tool that is not part of the request
        #[test]
        fn should_warn_and_still_send_an_entry_for_a_tool_that_is_not_part_of_the_request() {
            let result = prepare_allowed(
                json!([function_tool()]),
                None,
                json!({"toolNames": ["get_weather", "typo_tool"]}),
            );
            assert_eq!(
                result["toolChoice"],
                json!({
                    "type": "allowed_tools",
                    "mode": "auto",
                    "tools": [
                        {"type": "function", "name": "get_weather"},
                        {"type": "function", "name": "typo_tool"},
                    ],
                })
            );
            assert_eq!(
                result["toolWarnings"],
                json!([{
                    "type": "unsupported",
                    "feature": "allowedTools entry \"typo_tool\"",
                    "details": "the tool is not part of the tools for this request and is sent as a function tool",
                }])
            );
        }
    }

    mod programmatic_tool_calling {
        use super::*;

        fn program_tool() -> Value {
            json!({"type": "provider", "id": "openai.programmatic_tool_calling", "name": "program", "args": {}})
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › programmatic tool calling › should serialize the hosted tool and function tool options
        #[test]
        fn should_serialize_the_hosted_tool_and_function_tool_options() {
            let output_schema = json!({
                "type": "object",
                "properties": {"sku": {"type": "string"}, "availableUnits": {"type": "number"}},
                "required": ["sku", "availableUnits"],
                "additionalProperties": false,
            });
            let input_schema = json!({
                "type": "object",
                "properties": {"sku": {"type": "string"}},
                "required": ["sku"],
                "additionalProperties": false,
            });
            let result = prepare(
                json!([
                    program_tool(),
                    {
                        "type": "function",
                        "name": "get_inventory",
                        "description": "Get inventory",
                        "inputSchema": input_schema,
                        "providerOptions": {
                            "openai": {"allowedCallers": ["programmatic"], "outputSchema": output_schema},
                        },
                    },
                ]),
                None,
            );

            assert_eq!(
                result["tools"],
                json!([
                    {"type": "programmatic_tool_calling"},
                    {
                        "type": "function",
                        "name": "get_inventory",
                        "description": "Get inventory",
                        "parameters": input_schema,
                        "strict": false,
                        "allowed_callers": ["programmatic"],
                        "output_schema": output_schema,
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-prepare-tools.test.ts › prepareResponsesTools › programmatic tool calling › should support forcing the hosted tool
        #[test]
        fn should_support_forcing_the_hosted_tool() {
            let result = prepare(
                json!([program_tool()]),
                Some(json!({"type": "tool", "toolName": "program"})),
            );
            assert_eq!(
                result["toolChoice"],
                json!({"type": "programmatic_tool_calling"})
            );
        }
    }
}
