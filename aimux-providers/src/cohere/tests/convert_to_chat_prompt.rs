//! Ports of `src/convert-to-cohere-chat-prompt.test.ts` from `@ai-sdk/cohere@4.0.52`.
//!
//! One nested module per `describe`, one `#[test]` per `it`. Each test carries
//! its upstream chain in an `upstream:` marker line.

use crate::cohere::convert::{build_request_body, convert_prompt_to_cohere};
use crate::cohere::options::NAMESPACE;
use aimux_core::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart, ToolPart,
    ToolResultPart, UserPart,
};
use aimux_core::language_model_message::{FilePart, ToolResultOutput};
use aimux_core::options::CallOptions;
use aimux_core::shared::provider_namespace;
use aimux_core::shared::{FileBytes, FileData};
use indexmap::IndexMap;
use serde_json::json;

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
