//! Ports of the upstream `@ai-sdk/mistral` `convert-to-mistral-chat-messages.test.ts`.
//!
//! `toEqual` and inline snapshots treat an `undefined` property as absent, so
//! the expected values below leave such keys out. Snapshot files hold the same
//! values as the upstream `.snap` file, minus those keys.

use super::*;
use aimux_core::language_model_message::FilePart;

/// Build a prompt from its upstream shape, written as JSON.
fn json_prompt(value: Value) -> LanguageModelPrompt {
    serde_json::from_value(value).expect("prompt in the upstream shape")
}

/// The Mistral messages of a prompt, as the upstream array.
fn messages(prompt: &LanguageModelPrompt) -> Value {
    Value::Array(convert_prompt_to_mistral_messages(prompt).expect("prompt converts"))
}

mod convert_to_mistral_chat_messages {
    use super::*;

    mod user_messages {
        use super::*;

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › user messages › should convert messages with image parts
        #[test]
        fn should_convert_messages_with_image_parts() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Hello" },
                        {
                            "type": "file",
                            "data": { "type": "data", "data": "AAECAw==" },
                            "mediaType": "image/png"
                        }
                    ]
                }
            ])));

            let expected: Value =
                serde_json::from_str(include_str!("../snapshots/convert-image-parts.json"))
                    .expect("snapshot is JSON");
            assert_eq!(result, expected);
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › user messages › should convert messages with image parts from Uint8Array
        #[test]
        fn should_convert_messages_with_image_parts_from_uint8_array() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![
                    UserPart::Text(TextPart {
                        text: "Hi".into(),
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

            assert_eq!(
                messages(&prompt),
                json!([
                    {
                        "content": [
                            { "text": "Hi", "type": "text" },
                            {
                                "image_url": "data:image/png;base64,AAECAw==",
                                "type": "image_url"
                            }
                        ],
                        "role": "user"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › user messages › should convert messages with PDF file parts using URL
        #[test]
        fn should_convert_messages_with_pdf_file_parts_using_url() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Please analyze this document" },
                        {
                            "type": "file",
                            "data": {
                                "type": "url",
                                "url": "https://example.com/document.pdf"
                            },
                            "mediaType": "application/pdf"
                        }
                    ]
                }
            ])));

            let expected: Value =
                serde_json::from_str(include_str!("../snapshots/convert-pdf-url.json"))
                    .expect("snapshot is JSON");
            assert_eq!(result, expected);
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › user messages › should convert messages with PDF file parts from Uint8Array
        #[test]
        fn should_convert_messages_with_pdf_file_parts_from_uint8_array() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![
                    UserPart::Text(TextPart {
                        text: "Analyze this PDF".into(),
                        provider_options: None,
                    }),
                    UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Binary(vec![0, 1, 2, 3]),
                        },
                        media_type: "application/pdf".into(),
                        filename: None,
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];

            assert_eq!(
                messages(&prompt),
                json!([
                    {
                        "content": [
                            { "text": "Analyze this PDF", "type": "text" },
                            {
                                "document_url": "data:application/pdf;base64,AAECAw==",
                                "type": "document_url"
                            }
                        ],
                        "role": "user"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › user messages › should convert messages with reasoning content
        #[test]
        fn should_convert_messages_with_reasoning_content() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        { "type": "reasoning", "text": "Let me think about this..." },
                        { "type": "text", "text": "The answer is 42." }
                    ]
                }
            ])));

            assert_eq!(
                result,
                json!([
                    {
                        "content": [
                            {
                                "closed": true,
                                "thinking": [
                                    { "text": "Let me think about this...", "type": "text" }
                                ],
                                "type": "thinking"
                            },
                            { "text": "The answer is 42.", "type": "text" }
                        ],
                        "prefix": true,
                        "role": "assistant"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › user messages › should preserve the ordering of interleaved reasoning and text content
        #[test]
        fn should_preserve_the_ordering_of_interleaved_reasoning_and_text_content() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        { "type": "reasoning", "text": "First thought." },
                        { "type": "text", "text": "Partial answer." },
                        { "type": "reasoning", "text": "Second thought." },
                        { "type": "text", "text": "Final answer." }
                    ]
                },
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Continue." }]
                }
            ])));

            assert_eq!(
                result[0],
                json!({
                    "role": "assistant",
                    "content": [
                        {
                            "type": "thinking",
                            "thinking": [{ "type": "text", "text": "First thought." }],
                            "closed": true
                        },
                        { "type": "text", "text": "Partial answer." },
                        {
                            "type": "thinking",
                            "thinking": [{ "type": "text", "text": "Second thought." }],
                            "closed": true
                        },
                        { "type": "text", "text": "Final answer." }
                    ]
                })
            );
        }
    }

    mod tool_calls {
        use super::*;

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › tool calls › should stringify arguments to tool calls
        #[test]
        fn should_stringify_arguments_to_tool_calls() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "input": { "key": "arg-value" },
                            "toolCallId": "tool-call-id-1",
                            "toolName": "tool-1"
                        }
                    ]
                },
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "tool-call-id-1",
                            "toolName": "tool-1",
                            "output": { "type": "json", "value": { "key": "result-value" } }
                        }
                    ]
                }
            ])));

            assert_eq!(
                result,
                json!([
                    {
                        "content": "",
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "function": {
                                    "arguments": "{\"key\":\"arg-value\"}",
                                    "name": "tool-1"
                                },
                                "id": "tool-call-id-1",
                                "type": "function"
                            }
                        ]
                    },
                    {
                        "content": "{\"key\":\"result-value\"}",
                        "name": "tool-1",
                        "role": "tool",
                        "tool_call_id": "tool-call-id-1"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › tool calls › should handle text output format
        #[test]
        fn should_handle_text_output_format() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "input": { "query": "test" },
                            "toolCallId": "tool-call-id-2",
                            "toolName": "text-tool"
                        }
                    ]
                },
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "tool-call-id-2",
                            "toolName": "text-tool",
                            "output": { "type": "text", "value": "This is a text response" }
                        }
                    ]
                }
            ])));

            assert_eq!(
                result,
                json!([
                    {
                        "content": "",
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "function": {
                                    "arguments": "{\"query\":\"test\"}",
                                    "name": "text-tool"
                                },
                                "id": "tool-call-id-2",
                                "type": "function"
                            }
                        ]
                    },
                    {
                        "content": "This is a text response",
                        "name": "text-tool",
                        "role": "tool",
                        "tool_call_id": "tool-call-id-2"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › tool calls › should handle content output format
        #[test]
        fn should_handle_content_output_format() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "input": { "query": "generate image" },
                            "toolCallId": "tool-call-id-3",
                            "toolName": "image-tool"
                        }
                    ]
                },
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "tool-call-id-3",
                            "toolName": "image-tool",
                            "output": {
                                "type": "content",
                                "value": [
                                    { "type": "text", "text": "Here is the result:" },
                                    {
                                        "type": "file",
                                        "data": { "type": "data", "data": "base64data" },
                                        "mediaType": "image/png"
                                    }
                                ]
                            }
                        }
                    ]
                }
            ])));

            assert_eq!(
                result,
                json!([
                    {
                        "content": "",
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "function": {
                                    "arguments": "{\"query\":\"generate image\"}",
                                    "name": "image-tool"
                                },
                                "id": "tool-call-id-3",
                                "type": "function"
                            }
                        ]
                    },
                    {
                        "content": "[{\"type\":\"text\",\"text\":\"Here is the result:\"},{\"type\":\"file\",\"data\":{\"type\":\"data\",\"data\":\"base64data\"},\"mediaType\":\"image/png\"}]",
                        "name": "image-tool",
                        "role": "tool",
                        "tool_call_id": "tool-call-id-3"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › tool calls › should handle error output format
        #[test]
        fn should_handle_error_output_format() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "input": { "query": "test" },
                            "toolCallId": "tool-call-id-4",
                            "toolName": "error-tool"
                        }
                    ]
                },
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "tool-call-id-4",
                            "toolName": "error-tool",
                            "output": { "type": "error-text", "value": "Invalid input provided" }
                        }
                    ]
                }
            ])));

            assert_eq!(
                result,
                json!([
                    {
                        "content": "",
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "function": {
                                    "arguments": "{\"query\":\"test\"}",
                                    "name": "error-tool"
                                },
                                "id": "tool-call-id-4",
                                "type": "function"
                            }
                        ]
                    },
                    {
                        "content": "Invalid input provided",
                        "name": "error-tool",
                        "role": "tool",
                        "tool_call_id": "tool-call-id-4"
                    }
                ])
            );
        }
    }

    mod assistant_messages {
        use super::*;

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › assistant messages › should add prefix true to trailing assistant messages
        #[test]
        fn should_add_prefix_true_to_trailing_assistant_messages() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Hello" }]
                },
                {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Hello!" }]
                }
            ])));

            let expected: Value = serde_json::from_str(include_str!(
                "../snapshots/convert-prefix-trailing-assistant.json"
            ))
            .expect("snapshot is JSON");
            assert_eq!(result, expected);
        }
    }

    mod top_level_only_media_type_resolution {
        use super::*;

        const PNG_BASE64: &str = "iVBORw0KGgo=";

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › top-level-only media type resolution › passes full image/png through unchanged for inline data
        #[test]
        fn passes_full_image_png_through_unchanged_for_inline_data() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "mediaType": "image/png",
                            "data": { "type": "data", "data": PNG_BASE64 }
                        }
                    ]
                }
            ])));

            assert_eq!(
                result[0]["content"][0],
                json!({
                    "type": "image_url",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}")
                })
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
        #[test]
        fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "mediaType": "image",
                            "data": { "type": "data", "data": PNG_BASE64 }
                        }
                    ]
                }
            ])));

            assert_eq!(
                result[0]["content"][0],
                json!({
                    "type": "image_url",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}")
                })
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › top-level-only media type resolution › throws for top-level-only application with URL source
        #[test]
        fn throws_for_top_level_only_application_with_url_source() {
            let prompt = json_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "mediaType": "application",
                            "data": { "type": "url", "url": "https://example.com/x.pdf" }
                        }
                    ]
                }
            ]));

            let err = convert_prompt_to_mistral_messages(&prompt)
                .expect_err("application file parts with a URL are rejected");
            assert!(
                err.to_string()
                    .contains("Only images and PDF file parts are supported"),
                "unexpected error: {err}"
            );
        }

        /// upstream: @ai-sdk/mistral src/convert-to-mistral-chat-messages.test.ts › top-level-only media type resolution › normalizes image/* wildcard via detection
        #[test]
        fn normalizes_image_wildcard_via_detection() {
            let result = messages(&json_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "mediaType": "image/*",
                            "data": { "type": "data", "data": PNG_BASE64 }
                        }
                    ]
                }
            ])));

            assert_eq!(
                result[0]["content"][0],
                json!({
                    "type": "image_url",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}")
                })
            );
        }
    }
}
