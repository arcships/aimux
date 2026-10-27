//! Upstream `responses/convert-to-xai-responses-input.test.ts`.
//!
//! The upstream converter is `async` and returns `{ input, inputWarnings }`;
//! the Rust `convert_to_xai_responses_input` is synchronous and returns the
//! pair `(input, warnings)`, so the `await`s have no counterpart here. The
//! upstream `store: true` option has none either: upstream accepts it and
//! never reads it.
//!
//! Prompts are written as the JSON the Rust prompt types deserialize from.
//! The one exception is `new Uint8Array(...)`: JSON carries bytes as base64,
//! so those parts are built as `FileBytes::Binary` in Rust.

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::types::Warning;
use serde_json::{Value, json};

use crate::xai::options::NAMESPACE;
use crate::xai::responses::convert_to_xai_responses_input;

/// The prompt in its JSON form, as the Rust prompt types deserialize it.
fn parse_prompt(prompt: Value) -> LanguageModelPrompt {
    serde_json::from_value(prompt).expect("the prompt has the Rust prompt shape")
}

/// `convertToXaiResponsesInput({ prompt })`: the `input` array and the
/// `inputWarnings`.
fn convert(prompt: &LanguageModelPrompt) -> (Value, Vec<Warning>) {
    let (input, warnings) = convert_to_xai_responses_input(prompt)
        .expect("upstream converter does not reject this prompt");
    (Value::Array(input), warnings)
}

/// The error `convertToXaiResponsesInput` rejects the prompt with.
fn convert_error(prompt: &LanguageModelPrompt) -> AiMuxError {
    convert_to_xai_responses_input(prompt).expect_err("upstream converter rejects this prompt")
}

/// `[{ role: 'user', content: [{ type: 'text', text }, { type: 'file',
/// mediaType, data: { type: 'data', data: new Uint8Array(bytes) } }] }]`.
fn user_text_and_binary_file(text: &str, media_type: &str, bytes: &[u8]) -> LanguageModelPrompt {
    vec![LanguageModelMessage::User {
        content: vec![
            UserPart::Text(TextPart {
                text: text.to_string(),
                provider_options: None,
            }),
            UserPart::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Binary(bytes.to_vec()),
                },
                media_type: media_type.to_string(),
                filename: None,
                provider_options: None,
            }),
        ],
        provider_options: None,
    }]
}

mod convert_to_xai_responses_input {
    use super::*;

    mod system_messages {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › system messages › should convert system messages
        #[test]
        fn should_convert_system_messages() {
            let (input, warnings) = convert(&parse_prompt(json!([
                { "role": "system", "content": "you are a helpful assistant" },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": "you are a helpful assistant",
                        "role": "system",
                    },
                ])
            );
            assert_eq!(warnings, Vec::<Warning>::new());
        }
    }

    mod user_messages {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › user messages › should convert single text part
        #[test]
        fn should_convert_single_text_part() {
            let (input, _) = convert(&parse_prompt(json!([
                { "role": "user", "content": [{ "type": "text", "text": "hello" }] },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "hello",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › user messages › should concatenate multiple text parts
        #[test]
        fn should_concatenate_multiple_text_parts() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "hello " },
                        { "type": "text", "text": "world" },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "hello ",
                                "type": "input_text",
                            },
                            {
                                "text": "world",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › user messages › should convert image file parts with URL
        #[test]
        fn should_convert_image_file_parts_with_url() {
            let (input, warnings) = convert(&parse_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "what is in this image" },
                        {
                            "type": "file",
                            "mediaType": "image/jpeg",
                            "data": { "type": "url", "url": "https://example.com/image.jpg" },
                        },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "what is in this image",
                                "type": "input_text",
                            },
                            {
                                "image_url": "https://example.com/image.jpg",
                                "type": "input_image",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
            assert_eq!(warnings, Vec::<Warning>::new());
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › user messages › should convert image file parts with base64 data
        #[test]
        fn should_convert_image_file_parts_with_base64_data() {
            let (input, _) = convert(&user_text_and_binary_file(
                "describe this",
                "image/png",
                &[1, 2, 3],
            ));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "describe this",
                                "type": "input_text",
                            },
                            {
                                "image_url": "data:image/png;base64,AQID",
                                "type": "input_image",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › user messages › should convert non-image file parts with URL to input_file with file_url
        #[test]
        fn should_convert_non_image_file_parts_with_url_to_input_file_with_file_url() {
            let (input, warnings) = convert(&parse_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "summarize this PDF" },
                        {
                            "type": "file",
                            "mediaType": "application/pdf",
                            "data": { "type": "url", "url": "https://example.com/document.pdf" },
                        },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "summarize this PDF",
                                "type": "input_text",
                            },
                            {
                                "file_url": "https://example.com/document.pdf",
                                "type": "input_file",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
            assert_eq!(warnings, Vec::<Warning>::new());
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › user messages › should convert text/* file parts with URL to input_file with file_url
        #[test]
        fn should_convert_text_file_parts_with_url_to_input_file_with_file_url() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "analyze this CSV" },
                        {
                            "type": "file",
                            "mediaType": "text/csv",
                            "data": { "type": "url", "url": "https://example.com/data.csv" },
                        },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "analyze this CSV",
                                "type": "input_text",
                            },
                            {
                                "file_url": "https://example.com/data.csv",
                                "type": "input_file",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › user messages › should throw for non-image file parts provided as inline bytes
        #[test]
        fn should_throw_for_non_image_file_parts_provided_as_inline_bytes() {
            let error = convert_error(&user_text_and_binary_file(
                "check this file",
                "application/pdf",
                &[1, 2, 3],
            ));

            assert!(
                matches!(error, AiMuxError::UnsupportedFunctionality(_)),
                "{error:?}"
            );
            assert!(
                error
                    .to_string()
                    .contains("file part media type application/pdf"),
                "{error}"
            );
        }
    }

    mod assistant_messages {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › assistant messages › should convert text content
        #[test]
        fn should_convert_text_content() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "hi there" }],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": "hi there",
                        "role": "assistant",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › assistant messages › should handle client-side tool-call parts
        #[test]
        fn should_handle_client_side_tool_call_parts() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "call_123",
                            "toolName": "weather",
                            "input": { "location": "sf" },
                        },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "arguments": "{\"location\":\"sf\"}",
                        "call_id": "call_123",
                        "id": "call_123",
                        "name": "weather",
                        "status": "completed",
                        "type": "function_call",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › assistant messages › should handle client-side tool-call parts named like server-side tools
        #[test]
        fn should_handle_client_side_tool_call_parts_named_like_server_side_tools() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "call_ws",
                            "toolName": "web_search",
                            "input": { "query": "latest news" },
                        },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "arguments": "{\"query\":\"latest news\"}",
                        "call_id": "call_ws",
                        "id": "call_ws",
                        "name": "web_search",
                        "status": "completed",
                        "type": "function_call",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › assistant messages › should skip server-side tool-call parts
        #[test]
        fn should_skip_server_side_tool_call_parts() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "ws_123",
                            "toolName": "web_search",
                            "input": {},
                            "providerExecuted": true,
                        },
                    ],
                },
            ])));

            assert_eq!(input, json!([]));
        }
    }

    mod tool_messages {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › tool messages › should convert tool-result to function_call_output with json
        #[test]
        fn should_convert_tool_result_to_function_call_output_with_json() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "call_123",
                            "toolName": "weather",
                            "output": { "type": "json", "value": { "temp": 72 } },
                        },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": "{\"temp\":72}",
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › tool messages › should handle text output
        #[test]
        fn should_handle_text_output() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "call_123",
                            "toolName": "weather",
                            "output": { "type": "text", "value": "sunny, 72 degrees" },
                        },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": "sunny, 72 degrees",
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › tool messages › should preserve image URLs in content output
        #[test]
        fn should_preserve_image_urls_in_content_output() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "call_123",
                            "toolName": "inspectImage",
                            "output": {
                                "type": "content",
                                "value": [
                                    {
                                        "type": "text",
                                        "text": "The requested image is attached.",
                                    },
                                    {
                                        "type": "file",
                                        "mediaType": "image/png",
                                        "data": {
                                            "type": "url",
                                            "url": "https://example.com/image.png",
                                        },
                                    },
                                ],
                            },
                        },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "call_id": "call_123",
                        "output": [
                            {
                                "text": "The requested image is attached.",
                                "type": "input_text",
                            },
                            {
                                "image_url": "https://example.com/image.png",
                                "type": "input_image",
                            },
                        ],
                        "type": "function_call_output",
                    },
                ])
            );
        }
    }

    mod multi_turn_conversations {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › multi-turn conversations › should handle full conversation with client-side tool calls
        #[test]
        fn should_handle_full_conversation_with_client_side_tool_calls() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "whats the weather" }],
                },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "call_123",
                            "toolName": "weather",
                            "input": { "location": "sf" },
                        },
                    ],
                },
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "call_123",
                            "toolName": "weather",
                            "output": { "type": "json", "value": { "temp": 72 } },
                        },
                    ],
                },
                {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "its 72 degrees" }],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "whats the weather",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                    {
                        "arguments": "{\"location\":\"sf\"}",
                        "call_id": "call_123",
                        "id": "call_123",
                        "name": "weather",
                        "status": "completed",
                        "type": "function_call",
                    },
                    {
                        "call_id": "call_123",
                        "output": "{\"temp\":72}",
                        "type": "function_call_output",
                    },
                    {
                        "content": "its 72 degrees",
                        "role": "assistant",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › multi-turn conversations › should handle conversation with server-side tool calls and item references
        #[test]
        fn should_handle_conversation_with_server_side_tool_calls_and_item_references() {
            let (input, _) = convert(&parse_prompt(json!([
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "search for ai news" }],
                },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "ws_123",
                            "toolName": "web_search",
                            "input": {},
                            "providerExecuted": true,
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "ws_123",
                            "toolName": "web_search",
                            "output": { "type": "json", "value": {} },
                        },
                        { "type": "text", "text": "here are the results" },
                    ],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "search for ai news",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                    {
                        "content": "here are the results",
                        "role": "assistant",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › multi-turn conversations › should round-trip reasoning with encrypted content in multi-turn
        #[test]
        fn should_round_trip_reasoning_with_encrypted_content_in_multi_turn() {
            let (input, warnings) = convert(&parse_prompt(json!([
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "What is the capital of France?" }],
                },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "reasoning",
                            "text": "The user is asking about geography.",
                            "providerOptions": {
                                (NAMESPACE): {
                                    "itemId": "rs_789",
                                    "reasoningEncryptedContent": "encrypted_xyz",
                                },
                            },
                        },
                        { "type": "text", "text": "The capital of France is Paris." },
                    ],
                },
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "And what about Germany?" }],
                },
            ])));

            assert_eq!(
                input,
                json!([
                    {
                        "content": [
                            {
                                "text": "What is the capital of France?",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                    {
                        "encrypted_content": "encrypted_xyz",
                        "id": "rs_789",
                        "status": "completed",
                        "summary": [
                            {
                                "text": "The user is asking about geography.",
                                "type": "summary_text",
                            },
                        ],
                        "type": "reasoning",
                    },
                    {
                        "content": "The capital of France is Paris.",
                        "role": "assistant",
                    },
                    {
                        "content": [
                            {
                                "text": "And what about Germany?",
                                "type": "input_text",
                            },
                        ],
                        "role": "user",
                    },
                ])
            );
            assert_eq!(warnings, Vec::<Warning>::new());
        }
    }

    mod top_level_only_media_type_resolution {
        use super::*;

        const PNG_BASE64: &str = "iVBORw0KGgo=";

        /// The `content[0]` of the single user message the prompt converts to.
        fn first_content_part(prompt: Value) -> Value {
            let (input, _) = convert(&parse_prompt(prompt));
            input[0]["content"][0].clone()
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › top-level-only media type resolution › passes full image/png through unchanged for inline data
        #[test]
        fn passes_full_image_png_through_unchanged_for_inline_data() {
            let part = first_content_part(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "mediaType": "image/png",
                            "data": { "type": "data", "data": PNG_BASE64 },
                        },
                    ],
                },
            ]));

            assert_eq!(
                part,
                json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}"),
                })
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
        #[test]
        fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
            let part = first_content_part(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "mediaType": "image",
                            "data": { "type": "data", "data": PNG_BASE64 },
                        },
                    ],
                },
            ]));

            assert_eq!(
                part,
                json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}"),
                })
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › top-level-only media type resolution › passes through URL source for top-level-only image
        #[test]
        fn passes_through_url_source_for_top_level_only_image() {
            let part = first_content_part(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "mediaType": "image",
                            "data": { "type": "url", "url": "https://example.com/x.png" },
                        },
                    ],
                },
            ]));

            assert_eq!(
                part,
                json!({
                    "type": "input_image",
                    "image_url": "https://example.com/x.png",
                })
            );
        }

        /// upstream: @ai-sdk/xai src/responses/convert-to-xai-responses-input.test.ts › convertToXaiResponsesInput › top-level-only media type resolution › normalizes image/* wildcard via detection
        #[test]
        fn normalizes_image_wildcard_via_detection() {
            let part = first_content_part(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "mediaType": "image/*",
                            "data": { "type": "data", "data": PNG_BASE64 },
                        },
                    ],
                },
            ]));

            assert_eq!(
                part,
                json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}"),
                })
            );
        }
    }
}
