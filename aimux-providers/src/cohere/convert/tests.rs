//! Ported `@ai-sdk/cohere` unit tests for `convert.rs`: prompt conversion
//! (`convert_prompt_to_cohere`) and tool preparation (`prepare_tools`).
//!
//! One module per upstream test file, one nested module per `describe`, one
//! `#[test]` per `it`. Each test carries its upstream chain in an `upstream:`
//! marker line.

use super::*;
use crate::cohere::options::NAMESPACE;
use aimux_core::language_model_message::{FilePart, ToolResultOutput};
use aimux_core::shared::provider_namespace;
use aimux_core::tool::{FunctionTool, ProviderTool};
use aimux_core::types::NamespacedKind;
use indexmap::IndexMap;

/// Upstream's `warnings: []`. `convert_prompt_to_cohere` returns no warnings;
/// the warnings a prompt can produce are collected by `build_request_body`, so
/// that path stands in for the TS `warnings` field.
fn assert_no_warnings(prompt: LanguageModelPrompt) {
    let options = CallOptions {
        prompt,
        ..Default::default()
    };
    let body = build_request_body("command-a", &options, false).expect("request body");
    assert!(body.warnings.is_empty(), "warnings: {:?}", body.warnings);
}

mod convert_to_cohere_chat_prompt {
    use super::*;

    // The top-level describe shares its name with the file module above.
    #[allow(clippy::module_inception)]
    mod convert_to_cohere_chat_prompt {
        use super::*;

        mod file_processing {
            use super::*;

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › file processing › should extract documents from file parts
            #[test]
            fn should_extract_documents_from_file_parts() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![
                        UserPart::Text(TextPart {
                            text: "Analyze this file: ".into(),
                            provider_options: None,
                        }),
                        UserPart::File(FilePart {
                            data: FileData::Data {
                                data: FileBytes::Binary(b"This is file content".to_vec()),
                            },
                            media_type: "text/plain".into(),
                            filename: Some("test.txt".into()),
                            provider_options: None,
                        }),
                    ],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.messages,
                    vec![json!({
                        "role": "user",
                        "content": "Analyze this file: ",
                    })]
                );
                assert_eq!(
                    result.documents,
                    vec![json!({
                        "data": {
                            "text": "This is file content",
                            "title": "test.txt",
                        },
                    })]
                );
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › file processing › should accept top-level-only mediaType without error (category D: mediaType not consumed)
            #[test]
            fn should_accept_top_level_only_media_type_without_error_category_d_media_type_not_consumed()
             {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Binary(b"This is file content".to_vec()),
                        },
                        media_type: "text".into(),
                        filename: Some("test.txt".into()),
                        provider_options: None,
                    })],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.messages,
                    vec![json!({
                        "role": "user",
                        "content": "",
                    })]
                );
                assert_eq!(
                    result.documents,
                    vec![json!({
                        "data": {
                            "text": "This is file content",
                            "title": "test.txt",
                        },
                    })]
                );
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › file processing › should not read mediaType (document payload carries only text + title)
            #[test]
            fn should_not_read_media_type_document_payload_carries_only_text_title() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Binary(b"PDF-like content".to_vec()),
                        },
                        media_type: "application/pdf".into(),
                        filename: Some("test.pdf".into()),
                        provider_options: None,
                    })],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.documents,
                    vec![json!({
                        "data": {
                            "text": "PDF-like content",
                            "title": "test.pdf",
                        },
                    })]
                );
                let payload = json!({
                    "messages": result.messages,
                    "documents": result.documents,
                })
                .to_string();
                assert!(!payload.contains("application/pdf"), "{payload}");
                assert!(!payload.contains("mediaType"), "{payload}");
                assert_no_warnings(prompt);
            }
        }

        mod image_processing {
            use super::*;

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › image processing › should convert image file with data bytes into image_url data URI
            #[test]
            fn should_convert_image_file_with_data_bytes_into_image_url_data_uri() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![
                        UserPart::Text(TextPart {
                            text: "What is in this image?".into(),
                            provider_options: None,
                        }),
                        UserPart::File(FilePart {
                            data: FileData::Data {
                                data: FileBytes::Binary(vec![0, 1, 2, 3]),
                            },
                            media_type: "image/png".into(),
                            filename: None,
                            provider_options: None,
                        }),
                    ],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.messages,
                    vec![json!({
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "What is in this image?" },
                            {
                                "type": "image_url",
                                "image_url": { "url": "data:image/png;base64,AAECAw==" },
                            },
                        ],
                    })]
                );
                assert!(result.documents.is_empty());
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › image processing › should convert image file with URL data into image_url URL
            #[test]
            fn should_convert_image_file_with_url_data_into_image_url_url() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Url {
                            url: "https://example.com/cat.png".into(),
                            original_url: None,
                        },
                        media_type: "image/png".into(),
                        filename: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.messages,
                    vec![json!({
                        "role": "user",
                        "content": [
                            {
                                "type": "image_url",
                                "image_url": { "url": "https://example.com/cat.png" },
                            },
                        ],
                    })]
                );
                assert!(result.documents.is_empty());
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › image processing › should pass through detail provider option as image_url.detail
            #[test]
            fn should_pass_through_detail_provider_option_as_image_url_detail() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Binary(vec![0, 1, 2, 3]),
                        },
                        media_type: "image/png".into(),
                        filename: None,
                        provider_options: Some(
                            provider_namespace(NAMESPACE, json!({ "detail": "high" })).unwrap(),
                        ),
                    })],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.messages[0],
                    json!({
                        "role": "user",
                        "content": [
                            {
                                "type": "image_url",
                                "image_url": {
                                    "url": "data:image/png;base64,AAECAw==",
                                    "detail": "high",
                                },
                            },
                        ],
                    })
                );
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › image processing › should omit detail when no provider option is set
            #[test]
            fn should_omit_detail_when_no_provider_option_is_set() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Binary(vec![0, 1, 2, 3]),
                        },
                        media_type: "image/png".into(),
                        filename: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                let user_message = &result.messages[0];
                assert_eq!(user_message["role"], "user");
                assert!(user_message["content"].is_array());
                let part = &user_message["content"][0];
                assert_eq!(part["type"], "image_url");
                assert!(part["image_url"].get("detail").is_none());
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › image processing › should send image inline and route non-image file to documents
            #[test]
            fn should_send_image_inline_and_route_non_image_file_to_documents() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![
                        UserPart::Text(TextPart {
                            text: "See attached:".into(),
                            provider_options: None,
                        }),
                        UserPart::File(FilePart {
                            data: FileData::Data {
                                data: FileBytes::Binary(vec![0, 1, 2, 3]),
                            },
                            media_type: "image/png".into(),
                            filename: None,
                            provider_options: None,
                        }),
                        UserPart::File(FilePart {
                            data: FileData::Data {
                                data: FileBytes::Binary(b"Doc text".to_vec()),
                            },
                            media_type: "text/plain".into(),
                            filename: Some("note.txt".into()),
                            provider_options: None,
                        }),
                    ],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.messages,
                    vec![json!({
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "See attached:" },
                            {
                                "type": "image_url",
                                "image_url": { "url": "data:image/png;base64,AAECAw==" },
                            },
                        ],
                    })]
                );
                assert_eq!(
                    result.documents,
                    vec![json!({
                        "data": {
                            "text": "Doc text",
                            "title": "note.txt",
                        },
                    })]
                );
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › image processing › should accept top-level "image" media type and detect full type from bytes
            #[test]
            fn should_accept_top_level_image_media_type_and_detect_full_type_from_bytes() {
                let png_signature = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Binary(png_signature),
                        },
                        media_type: "image".into(),
                        filename: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                let part = &result.messages[0]["content"][0];
                assert_eq!(part["type"], "image_url");
                let url = part["image_url"]["url"].as_str().unwrap();
                assert!(
                    url.starts_with("data:image/png;base64,"),
                    "unexpected url: {url}"
                );
                assert_no_warnings(prompt);
            }
        }

        mod tool_messages {
            use super::*;

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › tool messages › should convert a tool call into a cohere chatbot message
            #[test]
            fn should_convert_a_tool_call_into_a_cohere_chatbot_message() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Assistant {
                    content: vec![
                        AssistantPart::Text(TextPart {
                            text: "Calling a tool".into(),
                            provider_options: None,
                        }),
                        AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id: "tool-call-1".into(),
                            tool_name: "tool-1".into(),
                            input: json!({ "test": "This is a tool message" }),
                            provider_executed: None,
                            provider_options: None,
                        }),
                    ],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                // Upstream `content: undefined` is absent from the message here.
                assert_eq!(
                    result.messages,
                    vec![json!({
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "id": "tool-call-1",
                                "type": "function",
                                "function": {
                                    "name": "tool-1",
                                    "arguments": json!({ "test": "This is a tool message" }).to_string(),
                                },
                            },
                        ],
                    })]
                );
                assert!(result.documents.is_empty());
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › tool messages › should convert a single tool result into a cohere tool message
            #[test]
            fn should_convert_a_single_tool_result_into_a_cohere_tool_message() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Tool {
                    content: vec![ToolPart::ToolResult(ToolResultPart {
                        tool_call_id: "tool-call-1".into(),
                        tool_name: "tool-1".into(),
                        output: ToolResultOutput::Json {
                            value: json!({ "test": "This is a tool message" }),
                            provider_options: None,
                        },
                        provider_options: None,
                    })],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.messages,
                    vec![json!({
                        "role": "tool",
                        "content": json!({ "test": "This is a tool message" }).to_string(),
                        "tool_call_id": "tool-call-1",
                    })]
                );
                assert!(result.documents.is_empty());
                assert_no_warnings(prompt);
            }

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › tool messages › should convert multiple tool results into a cohere tool message
            #[test]
            fn should_convert_multiple_tool_results_into_a_cohere_tool_message() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Tool {
                    content: vec![
                        ToolPart::ToolResult(ToolResultPart {
                            tool_call_id: "tool-call-1".into(),
                            tool_name: "tool-1".into(),
                            output: ToolResultOutput::Json {
                                value: json!({ "test": "This is a tool message" }),
                                provider_options: None,
                            },
                            provider_options: None,
                        }),
                        ToolPart::ToolResult(ToolResultPart {
                            tool_call_id: "tool-call-2".into(),
                            tool_name: "tool-2".into(),
                            output: ToolResultOutput::Json {
                                value: json!({ "something": "else" }),
                                provider_options: None,
                            },
                            provider_options: None,
                        }),
                    ],
                    provider_options: None,
                }];

                let result = convert_prompt_to_cohere(&prompt).unwrap();

                assert_eq!(
                    result.messages,
                    vec![
                        json!({
                            "role": "tool",
                            "content": json!({ "test": "This is a tool message" }).to_string(),
                            "tool_call_id": "tool-call-1",
                        }),
                        json!({
                            "role": "tool",
                            "content": json!({ "something": "else" }).to_string(),
                            "tool_call_id": "tool-call-2",
                        }),
                    ]
                );
                assert!(result.documents.is_empty());
                assert_no_warnings(prompt);
            }
        }

        mod provider_reference {
            use super::*;

            /// upstream: @ai-sdk/cohere src/convert-to-cohere-chat-prompt.test.ts › convert to cohere chat prompt › provider reference › should throw for file parts with provider references
            #[test]
            fn should_throw_for_file_parts_with_provider_references() {
                let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Reference {
                            reference: IndexMap::from([(
                                NAMESPACE.to_string(),
                                "doc-ref-123".to_string(),
                            )]),
                        },
                        media_type: "text/plain".into(),
                        filename: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                }];

                // Upstream's message is "'file parts with provider references'
                // functionality not supported"; the Rust error carries the
                // functionality name, which is what is asserted here.
                let Err(err) = convert_prompt_to_cohere(&prompt) else {
                    panic!("expected an error for a file part with a provider reference");
                };
                assert!(
                    matches!(
                        &err,
                        AiMuxError::UnsupportedFunctionality(name)
                            if name == "file parts with provider references"
                    ),
                    "unexpected error: {err}"
                );
            }
        }
    }
}

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
