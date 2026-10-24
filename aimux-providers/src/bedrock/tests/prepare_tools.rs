//! Upstream unit-tests `prepareTools` directly. aimux folds tool preparation
//! into `build_request_body_checked`, so the harness calls it with an empty
//! prompt and reads upstream's result fields back from the request body:
//! `toolConfig` is `body.toolConfig` (absent → `{}`), `additionalTools` is the
//! `tool_choice` that lands in `body.additionalModelRequestFields`, `betas` is
//! `additionalModelRequestFields.anthropic_beta`, and `toolWarnings` are the
//! returned warnings. `disableParallelToolUse` is passed the way the chat
//! model receives it (`providerOptions.anthropic.disableParallelToolUse`);
//! `rejectsForcedToolUse` is derived by aimux from the model id (every model
//! upstream passes it for rejects forced tool use).

use aimux_core::options::CallOptions;
use aimux_core::shared::provider_namespace;
use serde_json::{Value, json};

use crate::bedrock::chat_language_model::build_request_body_checked;

const NON_ANTHROPIC_MODEL: &str = "meta.llama3-70b-instruct-v1:0";
const ANTHROPIC_MODEL: &str = "anthropic.claude-sonnet-4-5-20250929-v1:0";

#[derive(Default)]
struct PrepareToolsArgs {
    tools: Option<Value>,
    tool_choice: Option<Value>,
    model_id: &'static str,
    model_family: Option<&'static str>,
    disable_parallel_tool_use: bool,
}

struct PreparedTools {
    tool_config: Value,
    additional_tools: Option<Value>,
    betas: Vec<Value>,
    tool_warnings: Value,
}

fn prepare_tools(args: PrepareToolsArgs) -> PreparedTools {
    let mut call = CallOptions::new(vec![]);
    call.tools = args
        .tools
        .map(|tools| serde_json::from_value(tools).expect("valid tools"));
    call.tool_choice = args
        .tool_choice
        .map(|choice| serde_json::from_value(choice).expect("valid tool choice"));
    if args.disable_parallel_tool_use {
        call.provider_options = Some(
            provider_namespace("anthropic", json!({ "disableParallelToolUse": true })).unwrap(),
        );
    }
    let (body, warnings, _, _) =
        build_request_body_checked(args.model_id, &call, args.model_family).unwrap();
    let fields = &body["additionalModelRequestFields"];
    PreparedTools {
        tool_config: body.get("toolConfig").cloned().unwrap_or_else(|| json!({})),
        additional_tools: fields
            .get("tool_choice")
            .map(|choice| json!({ "tool_choice": choice })),
        betas: fields["anthropic_beta"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        tool_warnings: serde_json::to_value(warnings).unwrap(),
    }
}

fn tool_spec(result: &PreparedTools, index: usize) -> &serde_json::Map<String, Value> {
    result.tool_config["tools"][index]["toolSpec"]
        .as_object()
        .expect("toolSpec")
}

#[allow(clippy::module_inception)] // the describe has the file's name
mod prepare_tools {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › should return empty toolConfig when tools are undefined
    #[test]
    fn should_return_empty_tool_config_when_tools_are_undefined() {
        let result = prepare_tools(PrepareToolsArgs {
            tools: None,
            model_id: ANTHROPIC_MODEL,
            ..Default::default()
        });

        assert_eq!(result.tool_config, json!({}));
        assert_eq!(result.additional_tools, None);
        assert!(result.betas.is_empty());
        assert_eq!(result.tool_warnings, json!([]));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › should return empty toolConfig when tools are empty
    #[test]
    fn should_return_empty_tool_config_when_tools_are_empty() {
        let result = prepare_tools(PrepareToolsArgs {
            tools: Some(json!([])),
            model_id: ANTHROPIC_MODEL,
            ..Default::default()
        });

        assert_eq!(result.tool_config, json!({}));
        assert_eq!(result.additional_tools, None);
        assert!(result.betas.is_empty());
        assert_eq!(result.tool_warnings, json!([]));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › should correctly prepare function tools
    #[test]
    fn should_correctly_prepare_function_tools() {
        let result = prepare_tools(PrepareToolsArgs {
            tools: Some(json!([{
                "type": "function",
                "name": "testFunction",
                "description": "A test function",
                "inputSchema": { "type": "object", "properties": {} },
            }])),
            model_id: ANTHROPIC_MODEL,
            ..Default::default()
        });

        assert_eq!(
            result.tool_config["tools"],
            json!([{
                "toolSpec": {
                    "name": "testFunction",
                    "description": "A test function",
                    "inputSchema": { "json": { "type": "object", "properties": {} } },
                },
            }])
        );
        assert_eq!(result.tool_warnings, json!([]));
    }

    mod tool_description_handling {
        use super::*;

        fn with_description(description: &str) -> PreparedTools {
            prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "function",
                    "name": "testFunction",
                    "description": description,
                    "inputSchema": { "type": "object", "properties": {} },
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            })
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool description handling › should exclude description when it is empty string
        #[test]
        fn should_exclude_description_when_it_is_empty_string() {
            let result = with_description("");

            assert!(!tool_spec(&result, 0).contains_key("description"));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool description handling › should exclude description when it is whitespace-only
        #[test]
        fn should_exclude_description_when_it_is_whitespace_only() {
            let result = with_description("   ");

            assert!(!tool_spec(&result, 0).contains_key("description"));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool description handling › should include description when it has content
        #[test]
        fn should_include_description_when_it_has_content() {
            let result = with_description("Valid description");

            assert_eq!(tool_spec(&result, 0)["description"], "Valid description");
        }
    }

    mod unsupported_provider_defined_tools {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › unsupported provider-defined tools › should warn for provider-defined tools on non-anthropic models
        #[test]
        fn should_warn_for_provider_defined_tools_on_non_anthropic_models() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "provider",
                    "id": "some.custom_tool",
                    "name": "custom_tool",
                    "args": {},
                }])),
                model_id: NON_ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(result.tool_config, json!({}));
            assert_eq!(
                result.tool_warnings,
                json!([{ "type": "unsupported", "feature": "tool some.custom_tool" }])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › unsupported provider-defined tools › should warn and filter out web_search_20250305 tool
        #[test]
        fn should_warn_and_filter_out_web_search_20250305_tool() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "provider",
                    "id": "anthropic.web_search_20250305",
                    "name": "web_search",
                    "args": {},
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(result.tool_config, json!({}));
            assert_eq!(
                result.tool_warnings,
                json!([{
                    "type": "unsupported",
                    "feature": "web_search_20250305 tool",
                    "details": "The web_search_20250305 tool is not supported on Amazon Bedrock.",
                }])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › unsupported provider-defined tools › should warn and filter out unsupported %s tool
        #[test]
        fn should_warn_and_filter_out_unsupported_tool() {
            for tool_id in [
                "anthropic.web_search_20260318",
                "anthropic.web_fetch_20260318",
            ] {
                let result = prepare_tools(PrepareToolsArgs {
                    tools: Some(json!([{
                        "type": "provider",
                        "id": tool_id,
                        "name": if tool_id.contains("search") { "web_search" } else { "web_fetch" },
                        "args": {},
                    }])),
                    model_id: ANTHROPIC_MODEL,
                    ..Default::default()
                });

                let tool_type = &tool_id["anthropic.".len()..];
                assert_eq!(result.tool_config, json!({}), "{tool_id}");
                assert_eq!(
                    result.tool_warnings,
                    json!([{
                        "type": "unsupported",
                        "feature": format!("{tool_type} tool"),
                        "details": format!("The {tool_type} tool is not supported on Amazon Bedrock."),
                    }]),
                    "{tool_id}"
                );
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › unsupported provider-defined tools › should return empty toolConfig when all tools are filtered out
        #[test]
        fn should_return_empty_tool_config_when_all_tools_are_filtered_out() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "provider",
                    "id": "anthropic.web_search_20250305",
                    "name": "web_search",
                    "args": {},
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(result.tool_config, json!({}));
        }
    }

    mod tool_choice {
        use super::*;

        fn test_function() -> Value {
            json!([{
                "type": "function",
                "name": "testFunction",
                "description": "Test",
                "inputSchema": {},
            }])
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › should use Anthropic tool choice fields for a declared Anthropic application inference profile
        #[test]
        fn should_use_anthropic_tool_choice_fields_for_a_declared_anthropic_inference_profile() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(test_function()),
                model_id: "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/custom-profile",
                model_family: Some("anthropic"),
                disable_parallel_tool_use: true,
                ..Default::default()
            });

            assert_eq!(
                result.additional_tools,
                Some(json!({
                    "tool_choice": { "type": "auto", "disable_parallel_tool_use": true },
                }))
            );
            assert_eq!(result.tool_config.get("toolChoice"), None);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › should use Anthropic tool choice fields when parallel tool use is disabled
        #[test]
        fn should_use_anthropic_tool_choice_fields_when_parallel_tool_use_is_disabled() {
            for (tool_choice, expected) in [
                (
                    None,
                    json!({ "type": "auto", "disable_parallel_tool_use": true }),
                ),
                (
                    Some(json!({ "type": "auto" })),
                    json!({ "type": "auto", "disable_parallel_tool_use": true }),
                ),
                (
                    Some(json!({ "type": "required" })),
                    json!({ "type": "any", "disable_parallel_tool_use": true }),
                ),
                (
                    Some(json!({ "type": "tool", "toolName": "testFunction" })),
                    json!({
                        "type": "tool",
                        "name": "testFunction",
                        "disable_parallel_tool_use": true,
                    }),
                ),
            ] {
                let label = format!("{tool_choice:?}");
                let result = prepare_tools(PrepareToolsArgs {
                    tools: Some(test_function()),
                    tool_choice,
                    model_id: ANTHROPIC_MODEL,
                    disable_parallel_tool_use: true,
                    ..Default::default()
                });

                assert_eq!(
                    result.additional_tools,
                    Some(json!({ "tool_choice": expected })),
                    "{label}"
                );
                assert_eq!(result.tool_config.get("toolChoice"), None, "{label}");
            }
        }

        mod models_that_reject_forced_tool_use {
            use super::*;

            fn weather_tool() -> Value {
                json!({
                    "type": "function",
                    "name": "getWeather",
                    "description": "Get weather",
                    "inputSchema": { "type": "object" },
                })
            }

            fn time_tool() -> Value {
                json!({
                    "type": "function",
                    "name": "getTime",
                    "description": "Get time",
                    "inputSchema": { "type": "object" },
                })
            }

            fn tool_names(result: &PreparedTools) -> Value {
                result.tool_config["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|tool| tool["toolSpec"]["name"].clone())
                    .collect()
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › models that reject forced tool use › should fall back to auto for tool choice "required" on %s
            #[test]
            fn should_fall_back_to_auto_for_tool_choice_required_on() {
                for model_id in [
                    "anthropic.claude-opus-5-5",
                    "us.anthropic.claude-opus-5-5",
                    "global.anthropic.claude-opus-5-5",
                    "anthropic.claude-sonnet-5-5",
                    "us.anthropic.claude-sonnet-5-5",
                ] {
                    let result = prepare_tools(PrepareToolsArgs {
                        tools: Some(json!([weather_tool()])),
                        tool_choice: Some(json!({ "type": "required" })),
                        model_id,
                        ..Default::default()
                    });

                    assert_eq!(
                        result.tool_config["toolChoice"],
                        json!({ "auto": {} }),
                        "{model_id}"
                    );
                    assert_eq!(
                        result.tool_warnings,
                        json!([{
                            "type": "unsupported",
                            "feature": "toolChoice",
                            "details": "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.",
                        }]),
                        "{model_id}"
                    );
                }
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › models that reject forced tool use › should fall back to auto and send only the named function tool for tool choice "tool"
            #[test]
            fn should_fall_back_to_auto_and_send_only_the_named_function_tool() {
                let result = prepare_tools(PrepareToolsArgs {
                    tools: Some(json!([weather_tool(), time_tool()])),
                    tool_choice: Some(json!({ "type": "tool", "toolName": "getWeather" })),
                    model_id: "us.anthropic.claude-opus-5-5",
                    ..Default::default()
                });

                assert_eq!(result.tool_config["toolChoice"], json!({ "auto": {} }));
                assert_eq!(tool_names(&result), json!(["getWeather"]));
                assert_eq!(
                    result.tool_warnings,
                    json!([{
                        "type": "unsupported",
                        "feature": "toolChoice",
                        "details": "toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the 'getWeather' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made.",
                    }])
                );
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › models that reject forced tool use › should drop provider tools that are not the named tool for tool choice "tool"
            #[test]
            fn should_drop_provider_tools_that_are_not_the_named_tool_for_tool_choice_tool() {
                let result = prepare_tools(PrepareToolsArgs {
                    tools: Some(json!([
                        weather_tool(),
                        {
                            "type": "provider",
                            "id": "anthropic.bash_20250124",
                            "name": "bash",
                            "args": {},
                        },
                    ])),
                    tool_choice: Some(json!({ "type": "tool", "toolName": "getWeather" })),
                    model_id: "us.anthropic.claude-opus-5-5",
                    ..Default::default()
                });

                assert_eq!(result.tool_config["toolChoice"], json!({ "auto": {} }));
                assert_eq!(tool_names(&result), json!(["getWeather"]));
                assert_eq!(result.additional_tools, None);
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › models that reject forced tool use › should preserve disabled parallel tool use in the auto fallback
            #[test]
            fn should_preserve_disabled_parallel_tool_use_in_the_auto_fallback() {
                let result = prepare_tools(PrepareToolsArgs {
                    tools: Some(json!([weather_tool()])),
                    tool_choice: Some(json!({ "type": "required" })),
                    model_id: "us.anthropic.claude-opus-5-5",
                    disable_parallel_tool_use: true,
                    ..Default::default()
                });

                assert_eq!(
                    result.additional_tools,
                    Some(json!({
                        "tool_choice": { "type": "auto", "disable_parallel_tool_use": true },
                    }))
                );
                assert_eq!(result.tool_config.get("toolChoice"), None);
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › models that reject forced tool use › should keep forced tool choice for models that support it
            #[test]
            fn should_keep_forced_tool_choice_for_models_that_support_it() {
                let result = prepare_tools(PrepareToolsArgs {
                    tools: Some(json!([weather_tool()])),
                    tool_choice: Some(json!({ "type": "required" })),
                    model_id: "us.anthropic.claude-opus-5",
                    ..Default::default()
                });

                assert_eq!(result.tool_config["toolChoice"], json!({ "any": {} }));
                assert_eq!(result.tool_warnings, json!([]));
            }
        }
    }

    mod strict_mode_for_function_tools {
        use super::*;

        const STRICT_WARNING: &str = "but Amazon Bedrock requires every object in a strict tool schema to set additionalProperties: false. The strict property will be ignored.";

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode when strict is true
        #[test]
        fn should_pass_through_strict_mode_when_strict_is_true() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "function",
                    "name": "testFunction",
                    "description": "A test function",
                    "inputSchema": {
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false,
                    },
                    "strict": true,
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(
                result.tool_config["tools"],
                json!([{
                    "toolSpec": {
                        "name": "testFunction",
                        "description": "A test function",
                        "strict": true,
                        "inputSchema": {
                            "json": {
                                "type": "object",
                                "properties": {},
                                "additionalProperties": false,
                            },
                        },
                    },
                }])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should keep strict mode enabled for %s
        #[test]
        fn should_keep_strict_mode_enabled_for() {
            for model_id in [
                "anthropic.claude-sonnet-4-6-v1",
                "us.anthropic.claude-haiku-4-5-20251001-v1:0",
            ] {
                let result = prepare_tools(PrepareToolsArgs {
                    tools: Some(json!([{
                        "type": "function",
                        "name": "testFunction",
                        "description": "A test function",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false,
                        },
                        "strict": true,
                    }])),
                    model_id,
                    ..Default::default()
                });

                assert_eq!(tool_spec(&result, 0)["strict"], json!(true), "{model_id}");
                assert_eq!(result.tool_warnings, json!([]), "{model_id}");
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should omit strict mode when the top-level object schema is open
        #[test]
        fn should_omit_strict_mode_when_the_top_level_object_schema_is_open() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "function",
                    "name": "testFunction",
                    "description": "A test function",
                    "inputSchema": { "type": "object", "properties": {} },
                    "strict": true,
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert!(!tool_spec(&result, 0).contains_key("strict"));
            assert_eq!(
                result.tool_warnings,
                json!([{
                    "type": "unsupported",
                    "feature": "strict",
                    "details": format!("Tool 'testFunction' has strict: true, {STRICT_WARNING}"),
                }])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should omit strict mode when a nested object schema is open
        #[test]
        fn should_omit_strict_mode_when_a_nested_object_schema_is_open() {
            let input_schema = json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "object",
                        "properties": { "city": { "type": "string" } },
                        "required": ["city"],
                    },
                },
                "required": ["location"],
                "additionalProperties": false,
            });

            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "function",
                    "name": "getWeather",
                    "inputSchema": input_schema,
                    "strict": true,
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(
                Value::Object(tool_spec(&result, 0).clone()),
                json!({ "name": "getWeather", "inputSchema": { "json": input_schema } })
            );
            assert_eq!(
                result.tool_warnings,
                json!([{
                    "type": "unsupported",
                    "feature": "strict",
                    "details": format!("Tool 'getWeather' has strict: true, {STRICT_WARNING}"),
                }])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should omit strict mode when a referenced object schema in $defs is open
        #[test]
        fn should_omit_strict_mode_when_a_referenced_object_schema_in_defs_is_open() {
            let input_schema = json!({
                "$ref": "#/$defs/location",
                "$defs": {
                    "location": {
                        "type": "object",
                        "properties": { "city": { "type": "string" } },
                        "required": ["city"],
                    },
                },
            });

            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "function",
                    "name": "getWeather",
                    "inputSchema": input_schema,
                    "strict": true,
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(
                Value::Object(tool_spec(&result, 0).clone()),
                json!({ "name": "getWeather", "inputSchema": { "json": input_schema } })
            );
            assert_eq!(
                result.tool_warnings,
                json!([{
                    "type": "unsupported",
                    "feature": "strict",
                    "details": format!("Tool 'getWeather' has strict: true, {STRICT_WARNING}"),
                }])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode when all nested object schemas are closed
        #[test]
        fn should_pass_through_strict_mode_when_all_nested_object_schemas_are_closed() {
            let input_schema = json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "object",
                        "properties": { "city": { "type": "string" } },
                        "required": ["city"],
                        "additionalProperties": false,
                    },
                },
                "required": ["location"],
                "additionalProperties": false,
            });

            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "function",
                    "name": "getWeather",
                    "inputSchema": input_schema,
                    "strict": true,
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(
                Value::Object(tool_spec(&result, 0).clone()),
                json!({
                    "name": "getWeather",
                    "strict": true,
                    "inputSchema": { "json": input_schema },
                })
            );
            assert_eq!(result.tool_warnings, json!([]));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode when strict is false
        #[test]
        fn should_pass_through_strict_mode_when_strict_is_false() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "function",
                    "name": "testFunction",
                    "description": "A test function",
                    "inputSchema": { "type": "object", "properties": {} },
                    "strict": false,
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(
                result.tool_config["tools"],
                json!([{
                    "toolSpec": {
                        "name": "testFunction",
                        "description": "A test function",
                        "strict": false,
                        "inputSchema": { "json": { "type": "object", "properties": {} } },
                    },
                }])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should not include strict when strict is undefined
        #[test]
        fn should_not_include_strict_when_strict_is_undefined() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([{
                    "type": "function",
                    "name": "testFunction",
                    "description": "A test function",
                    "inputSchema": { "type": "object", "properties": {} },
                }])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert!(!tool_spec(&result, 0).contains_key("strict"));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode for multiple tools with different strict settings
        #[test]
        fn should_pass_through_strict_mode_for_multiple_tools_with_different_strict_settings() {
            let result = prepare_tools(PrepareToolsArgs {
                tools: Some(json!([
                    {
                        "type": "function",
                        "name": "strictTool",
                        "description": "A strict tool",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false,
                        },
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
                ])),
                model_id: ANTHROPIC_MODEL,
                ..Default::default()
            });

            assert_eq!(tool_spec(&result, 0)["strict"], json!(true));
            assert_eq!(tool_spec(&result, 1)["strict"], json!(false));
            assert!(!tool_spec(&result, 2).contains_key("strict"));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › strict mode for function tools › should warn when strict is omitted for %s
        #[test]
        fn should_warn_when_strict_is_omitted_for() {
            for model_id in [
                "us.anthropic.claude-opus-4-7",
                "anthropic.claude-opus-4-8",
                "us.anthropic.claude-opus-5",
                "anthropic.claude-sonnet-5",
                "eu.anthropic.claude-fable-5",
                "anthropic.claude-fable-5-1",
                "us.anthropic.claude-fable-5-1",
                "global.anthropic.claude-fable-5-1",
            ] {
                let result = prepare_tools(PrepareToolsArgs {
                    tools: Some(json!([{
                        "type": "function",
                        "name": "testFunction",
                        "description": "A test function",
                        "inputSchema": { "type": "object", "properties": {} },
                        "strict": true,
                    }])),
                    model_id,
                    ..Default::default()
                });

                assert!(!tool_spec(&result, 0).contains_key("strict"), "{model_id}");
                assert_eq!(
                    result.tool_warnings,
                    json!([{
                        "type": "unsupported",
                        "feature": "strict",
                        "details": "Tool 'testFunction' has strict: true, but strict mode is not supported by this model on Amazon Bedrock. The strict property will be ignored.",
                    }]),
                    "{model_id}"
                );
            }
        }
    }
}

// ── Older ports (previously `tests/bedrock_convert_test.rs`) ──
//
// Pure-function tests for the Amazon Bedrock provider's conversion layer.
//
// Translated from the TS test files:
// - `convert-to-amazon-bedrock-chat-messages.test.ts` (64 cases)
// - `amazon-bedrock-prepare-tools.test.ts` (23 cases incl. `it.each`)
// - `convert-amazon-bedrock-usage.test.ts` (9 cases)

mod bedrock_convert {
    use serde_json::{Value, json};

    use aimux_core::options::LanguageModelV4ToolChoice;
    use aimux_core::tool::FunctionTool;

    use crate::bedrock::prepare_tools::prepare_tools;

    // ════════════════════════════════════════════════════════════════════════════
    // amazon-bedrock-prepare-tools
    // ════════════════════════════════════════════════════════════════════════════

    const NON_ANTHROPIC_MODEL: &str = "meta.llama3-70b-instruct-v1:0";

    fn func_tool(name: &str, description: Option<&str>, input_schema: Value) -> FunctionTool {
        FunctionTool {
            name: name.to_string(),
            description: description.map(std::string::ToString::to_string),
            input_schema,
            strict: None,
            provider_options: None,
            input_examples: None,
        }
    }

    mod prepare_tools {
        use super::*;

        mod tool_choice {
            use super::*;

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › should handle tool choice "auto"
            #[test]
            fn prepare_tools_tool_choice_auto() {
                let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
                let config = prepare_tools(
                    &tools,
                    Some(&LanguageModelV4ToolChoice::Auto),
                    NON_ANTHROPIC_MODEL,
                );
                assert_eq!(config["toolChoice"], json!({ "auto": {} }));
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › should handle tool choice "required"
            #[test]
            fn prepare_tools_tool_choice_required() {
                let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
                let config = prepare_tools(
                    &tools,
                    Some(&LanguageModelV4ToolChoice::Required),
                    NON_ANTHROPIC_MODEL,
                );
                assert_eq!(config["toolChoice"], json!({ "any": {} }));
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › should handle tool choice "none" by clearing tools
            #[test]
            fn prepare_tools_tool_choice_none_clears() {
                let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
                let config = prepare_tools(
                    &tools,
                    Some(&LanguageModelV4ToolChoice::None),
                    NON_ANTHROPIC_MODEL,
                );
                assert_eq!(config, json!({}));
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › should handle tool choice "tool"
            #[test]
            fn prepare_tools_tool_choice_tool() {
                let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
                let config = prepare_tools(
                    &tools,
                    Some(&LanguageModelV4ToolChoice::Tool {
                        tool_name: "testFunction".to_string(),
                    }),
                    NON_ANTHROPIC_MODEL,
                );
                assert_eq!(
                    config["toolChoice"],
                    json!({ "tool": { "name": "testFunction" } })
                );
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-prepare-tools.test.ts › prepareTools › tool choice › should filter function tools to only the named tool when tool choice is "tool"
            #[test]
            fn prepare_tools_tool_choice_filters_to_named() {
                let tools = Some(vec![
                    func_tool(
                        "getWeather",
                        Some("Get weather"),
                        json!({ "type": "object" }),
                    ),
                    func_tool("getTime", Some("Get time"), json!({ "type": "object" })),
                ]);
                let config = prepare_tools(
                    &tools,
                    Some(&LanguageModelV4ToolChoice::Tool {
                        tool_name: "getWeather".to_string(),
                    }),
                    NON_ANTHROPIC_MODEL,
                );
                assert_eq!(config["tools"].as_array().unwrap().len(), 1);
                assert_eq!(config["tools"][0]["toolSpec"]["name"], json!("getWeather"));
            }
        }
    }
}

// ── Older ports (previously `tests/bedrock_upstream_e2e_test.rs`) ──

mod bedrock_upstream_e2e {
    use crate::bedrock::tests::upstream_e2e_support::*;
    use aimux_core::AiMuxError;

    use aimux_core::language_model::LanguageModel;

    use crate::test_support::mock_fetch::{Canned, MockFetch};
    use serde_json::{Value, json};

    /// Provider tool args go through the Anthropic `prepareTools` check, and
    /// the tools Bedrock leaves out are dropped before it runs.
    #[tokio::test]
    async fn provider_tool_args_are_checked_before_the_request() {
        let tool = |id: &str, args: Value| {
            aimux_core::tool::Tool::Provider(aimux_core::tool::ProviderTool {
                id: id.parse().unwrap(),
                name: "web".into(),
                args: args.as_object().cloned().unwrap(),
                supports_deferred_results: None,
            })
        };
        let fetch = MockFetch::new(vec![]);
        let mut call = options();
        call.tools = Some(vec![tool(
            "anthropic.web_fetch_20250910",
            json!({"maxUses": "3"}),
        )]);
        let error = provider(&fetch)
            .chat("anthropic.model")
            .do_generate(&call)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AiMuxError::InvalidArgument { message, .. } if message == "Invalid configuration for provider-defined tool anthropic.web_fetch_20250910"),
            "{error:?}"
        );
        assert!(fetch.seen().is_empty());

        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "output":{"message":{"role":"assistant","content":[{"text":"Hi"}]}},
            "usage":{"inputTokens":1,"outputTokens":1,"totalTokens":2},"stopReason":"end_turn"
        }))]);
        call.tools = Some(vec![tool(
            "anthropic.web_search_20250305",
            json!({"maxUses": "3"}),
        )]);
        let result = provider(&fetch)
            .chat("anthropic.model")
            .do_generate(&call)
            .await
            .unwrap();
        assert_eq!(fetch.seen().len(), 1);
        assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
    }
}
