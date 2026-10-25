//! Upstream `groq-prepare-tools.test.ts`: direct calls, plus the older cases
//! (formerly `tests/groq_test.rs`) checked through the `do_generate` request
//! body on a wiremock `MockServer` ([`super::support`]).

use aimux_core::options::CallOptions;
use aimux_core::tool::{FunctionTool, LanguageModelV4ToolChoice, ProviderTool, Tool};
use serde_json::{Value, json};

use crate::groq::prepare_tools::prepare_tools;

use super::support::*;

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

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode when strict is true
        #[tokio::test]
        async fn strict_mode_true() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = FunctionTool {
                name: "testFunction".to_string(),
                description: Some("A test function".to_string()),
                input_schema: json!({"type": "object", "properties": {}}),
                strict: Some(true),
                provider_options: None,
                input_examples: None,
            };
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["tools"],
                json!([{
                    "type": "function",
                    "function": {
                        "name": "testFunction",
                        "description": "A test function",
                        "parameters": { "type": "object", "properties": {} },
                        "strict": true
                    }
                }])
            );
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode when strict is false
        #[tokio::test]
        async fn strict_mode_false() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = FunctionTool {
                name: "testFunction".to_string(),
                description: Some("A test function".to_string()),
                input_schema: json!({"type": "object", "properties": {}}),
                strict: Some(false),
                provider_options: None,
                input_examples: None,
            };
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["tools"],
                json!([{
                    "type": "function",
                    "function": {
                        "name": "testFunction",
                        "description": "A test function",
                        "parameters": { "type": "object", "properties": {} },
                        "strict": false
                    }
                }])
            );
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › strict mode for function tools › should not include strict when strict is undefined
        #[tokio::test]
        async fn strict_mode_undefined() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool =
                FunctionTool::new("testFunction", json!({"type": "object", "properties": {}}))
                    .with_description("A test function");
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["tools"],
                json!([{
                    "type": "function",
                    "function": {
                        "name": "testFunction",
                        "description": "A test function",
                        "parameters": { "type": "object", "properties": {} }
                    }
                }])
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

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should handle browser search tool with supported model
        #[tokio::test]
        async fn browser_search_supported_model() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = model_at(&server, browser_search_model_ids()[1]);

            let tool = Tool::Provider(aimux_core::tool::ProviderTool {
                id: "groq.browser_search".parse().unwrap(),
                name: "browser_search".to_string(),
                args: serde_json::Map::new(),
                supports_deferred_results: None,
            });
            let options = CallOptions {
                tools: Some(vec![tool]),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["tools"], json!([{ "type": "browser_search" }]));
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should warn when browser search is used with unsupported model
        #[tokio::test]
        async fn browser_search_unsupported_model() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = Tool::Provider(aimux_core::tool::ProviderTool {
                id: "groq.browser_search".parse().unwrap(),
                name: "browser_search".to_string(),
                args: serde_json::Map::new(),
                supports_deferred_results: None,
            });
            let options = CallOptions {
                tools: Some(vec![tool]),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert!(body.get("tools").is_none_or(|tools| *tools == json!([])));
            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([{
                    "details": "Browser search is only supported on the following models: openai/gpt-oss-20b, openai/gpt-oss-120b. Current model: gemma2-9b-it",
                    "feature": "provider-defined tool groq.browser_search",
                    "type": "unsupported"
                }])
            );
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should handle mixed tools with model validation
        #[tokio::test]
        async fn mixed_tools_with_browser_search() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = model_at(&server, browser_search_model_ids()[0]);

            let func_tool =
                FunctionTool::new("test-tool", json!({"type":"object","properties":{}}))
                    .with_description("A test tool");
            let browser_tool = Tool::Provider(aimux_core::tool::ProviderTool {
                id: "groq.browser_search".parse().unwrap(),
                name: "browser_search".to_string(),
                args: serde_json::Map::new(),
                supports_deferred_results: None,
            });
            let options = CallOptions {
                tools: Some(vec![Tool::from(func_tool), browser_tool]),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["tools"],
                json!([
                    {
                        "type": "function",
                        "function": {
                            "name": "test-tool",
                            "description": "A test tool",
                            "parameters": { "type": "object", "properties": {} }
                        }
                    },
                    { "type": "browser_search" }
                ])
            );
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should validate all browser search supported models
        #[tokio::test]
        async fn browser_search_all_supported_models() {
            let model_ids = browser_search_model_ids();
            assert_eq!(model_ids, ["openai/gpt-oss-20b", "openai/gpt-oss-120b"]);
            for model_id in model_ids {
                let server = MockServer::start().await;
                mock_json(&server, text_completion_body()).await;

                let model = model_at(&server, model_id);

                let tool = Tool::Provider(aimux_core::tool::ProviderTool {
                    id: "groq.browser_search".parse().unwrap(),
                    name: "browser_search".to_string(),
                    args: serde_json::Map::new(),
                    supports_deferred_results: None,
                });
                let options = CallOptions {
                    tools: Some(vec![tool]),
                    ..default_options(test_prompt())
                };
                let result = model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(body["tools"], json!([{ "type": "browser_search" }]));
                assert!(result.warnings.is_empty());
            }
        }
    }

    /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should correctly prepare function tools
    #[tokio::test]
    async fn function_tools() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let tool = FunctionTool {
            name: "testFunction".to_string(),
            description: Some("A test function".to_string()),
            input_schema: json!({"type": "object", "properties": {}}),
            strict: None,
            provider_options: None,
            input_examples: None,
        };
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body["tools"],
            json!([{
                "type": "function",
                "function": {
                    "name": "testFunction",
                    "description": "A test function",
                    "parameters": { "type": "object", "properties": {} }
                }
            }])
        );
        assert!(body.get("tool_choice").is_none());
        assert!(result.warnings.is_empty());
    }

    /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should add warnings for unsupported provider-defined tools
    #[tokio::test]
    async fn unsupported_provider_tool_warning() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let tool = Tool::Provider(aimux_core::tool::ProviderTool {
            id: "some.unsupported_tool".parse().unwrap(),
            name: "unsupported_tool".to_string(),
            args: serde_json::Map::new(),
            supports_deferred_results: None,
        });
        let options = CallOptions {
            tools: Some(vec![tool]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        // Upstream's prepared `tools: []`: no tool reaches the request.
        assert!(body.get("tools").is_none_or(|tools| *tools == json!([])));
        assert!(body.get("tool_choice").is_none());
        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([{
                "feature": "provider-defined tool some.unsupported_tool",
                "type": "unsupported"
            }])
        );
    }

    /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should handle tool choice "auto"
    #[tokio::test]
    async fn tool_choice_auto() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: Some(LanguageModelV4ToolChoice::Auto),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        // upstream groq-prepare-tools.ts: `case 'auto': tool_choice: 'auto'`
        assert_eq!(body["tool_choice"], "auto");
    }

    /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should handle tool choice "required"
    #[tokio::test]
    async fn tool_choice_required() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: Some(LanguageModelV4ToolChoice::Required),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tool_choice"], "required");
    }

    /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should handle tool choice "none"
    #[tokio::test]
    async fn tool_choice_none() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: Some(LanguageModelV4ToolChoice::None),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tool_choice"], "none");
    }

    /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should handle tool choice "tool"
    #[tokio::test]
    async fn tool_choice_tool() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: Some(LanguageModelV4ToolChoice::Tool {
                tool_name: "testFunction".to_string(),
            }),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body["tool_choice"],
            json!({ "type": "function", "function": { "name": "testFunction" } })
        );
    }
}
