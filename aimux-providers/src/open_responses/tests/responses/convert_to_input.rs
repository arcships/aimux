//! Port of `src/responses/convert-to-open-responses-input.test.ts`.
//!
//! Cases whose older port (previously `tests/open_responses_test.rs`, now in
//! the modules below) already asserts every upstream expectation carry their
//! marker there instead.

use super::support::*;
use crate::open_responses::responses::convert_to_input::convert_to_open_responses_input_with_namespace;
use crate::open_responses::{
    OpenResponsesProviderSettings, convert_to_open_responses_input, create_open_responses,
};
use crate::test_support::mock_fetch::MockFetch;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart,
    ToolPart, ToolResultPart, UserPart,
};
use aimux_core::options::CallOptions;
use aimux_core::shared::{FileBytes, FileData};
use serde_json::{Value, json};

/// The parts of the upstream `convertToOpenResponsesInput` result these
/// cases assert.
struct Converted {
    input: Value,
    instructions: Option<String>,
}

/// The upstream prompt literal, parsed into the Rust prompt type.
fn prompt(value: Value) -> LanguageModelPrompt {
    serde_json::from_value(value).expect("valid prompt")
}

/// `convertToOpenResponsesInput({ prompt })`.
fn convert(value: Value) -> Converted {
    let (input, instructions, _) = convert_to_open_responses_input(&prompt(value));
    Converted {
        input,
        instructions,
    }
}

/// `convertToOpenResponsesInput({ providerOptionsName, prompt })`.
fn convert_with(provider_options_name: &str, value: Value) -> Converted {
    let (input, instructions, _) =
        convert_to_open_responses_input_with_namespace(&prompt(value), provider_options_name);
    Converted {
        input,
        instructions,
    }
}

mod convert_to_open_responses_input {
    use super::*;

    mod system_messages {
        use super::*;

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › system messages › should join multiple system messages with newlines
        #[test]
        fn should_join_multiple_system_messages_with_newlines() {
            let result = convert(json!([
                { "role": "system", "content": "You are a helpful assistant." },
                { "role": "system", "content": "Always be concise." },
            ]));

            assert_eq!(
                result.instructions.as_deref(),
                Some("You are a helpful assistant.\nAlways be concise.")
            );
            assert_eq!(result.input, json!([]));
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › system messages › should convert a single system message to instructions
        #[test]
        fn convert_single_system_message_to_instructions() {
            let prompt = vec![LanguageModelMessage::System {
                content: "You are a helpful assistant.".into(),
                provider_options: None,
            }];
            let (input, instructions, warnings) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                instructions,
                Some("You are a helpful assistant.".to_string())
            );
            assert_eq!(input, json!([]));
            assert!(warnings.is_empty());
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › system messages › should return undefined instructions when no system messages
        #[test]
        fn convert_no_system_messages_returns_none_instructions() {
            let prompt = wiremock_test_prompt();
            let (_, instructions, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(instructions, None);
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › system messages › should handle system message with user and assistant messages
        #[test]
        fn convert_system_message_with_user_and_assistant() {
            let prompt = vec![
                LanguageModelMessage::System {
                    content: "You are a helpful assistant.".into(),
                    provider_options: None,
                },
                LanguageModelMessage::user_text("Hello"),
                LanguageModelMessage::Assistant {
                    content: vec![AssistantPart::Text(TextPart {
                        text: "Hi there!".into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
            ];
            let (input, instructions, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                instructions,
                Some("You are a helpful assistant.".to_string())
            );
            assert_eq!(
                input,
                json!([
                    {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "Hello"}]
                    },
                    {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "Hi there!"}]
                    }
                ])
            );
        }
    }

    mod user_messages {
        use super::*;

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › user messages › should convert image file parts with base64 data to input_image
        #[test]
        fn should_convert_image_file_parts_with_base64_data_to_input_image() {
            let result = convert(json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "data", "data": "ZmFrZS1kYXRh" },
                    "mediaType": "image/png",
                }],
            }]));

            assert_eq!(
                result.input,
                json!([{
                    "content": [{
                        "detail": "auto",
                        "image_url": "data:image/png;base64,ZmFrZS1kYXRh",
                        "type": "input_image",
                    }],
                    "role": "user",
                    "type": "message",
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › user messages › should convert image file parts with URL data to input_image
        #[test]
        fn should_convert_image_file_parts_with_url_data_to_input_image() {
            let result = convert(json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "url", "url": "https://example.com/image.png" },
                    "mediaType": "image/png",
                }],
            }]));

            assert_eq!(
                result.input,
                json!([{
                    "content": [{
                        "detail": "auto",
                        "image_url": "https://example.com/image.png",
                        "type": "input_image",
                    }],
                    "role": "user",
                    "type": "message",
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › user messages › should preserve image detail provider options
        #[test]
        fn should_preserve_image_detail_provider_options() {
            let result = convert_with(
                "test-provider",
                json!([{
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "data": { "type": "data", "data": "ZmFrZS1kYXRh" },
                            "mediaType": "image/png",
                            "providerOptions": {
                                "test-provider": { "imageDetail": "low" },
                            },
                        },
                        {
                            "type": "file",
                            "data": { "type": "url", "url": "https://example.com/image.png" },
                            "mediaType": "image/png",
                            "providerOptions": {
                                "test-provider": { "imageDetail": "high" },
                            },
                        },
                    ],
                }]),
            );

            assert_eq!(
                result.input,
                json!([{
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_image",
                            "image_url": "data:image/png;base64,ZmFrZS1kYXRh",
                            "detail": "low",
                        },
                        {
                            "type": "input_image",
                            "image_url": "https://example.com/image.png",
                            "detail": "high",
                        },
                    ],
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › user messages › should convert messages with only a text part to a string content
        #[test]
        fn convert_user_text_part() {
            let prompt = wiremock_test_prompt();
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Hello"}]
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › user messages › should convert PDF file parts with base64 data to input_file
        #[test]
        fn convert_pdf_file_base64_to_input_file() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![
                    UserPart::Text(TextPart {
                        text: "What does this PDF say?".into(),
                        provider_options: None,
                    }),
                    UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Base64("UERGREFUQQ==".into()),
                        },
                        media_type: "application/pdf".into(),
                        filename: None,
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];
            let (input, _, warnings) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "message",
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "What does this PDF say?"},
                        {"type": "input_file", "filename": "data", "file_data": "data:application/pdf;base64,UERGREFUQQ=="}
                    ]
                }])
            );
            assert!(warnings.is_empty());
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › user messages › should convert PDF file parts with URL data to input_file
        #[test]
        fn convert_pdf_file_url_to_input_file() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![UserPart::File(FilePart {
                    data: FileData::Url {
                        original_url: None,
                        url: "https://example.com/document.pdf".into(),
                    },
                    media_type: "application/pdf".into(),
                    filename: None,
                    provider_options: None,
                })],
                provider_options: None,
            }];
            let (input, _, warnings) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_file", "file_url": "https://example.com/document.pdf"}]
                }])
            );
            assert!(warnings.is_empty());
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › user messages › should use provided filename for non-image file parts
        #[test]
        fn convert_file_with_custom_filename() {
            let prompt = vec![LanguageModelMessage::User {
                content: vec![UserPart::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64("UERGREFUQQ==".to_string()),
                    },
                    media_type: "application/pdf".to_string(),
                    filename: Some("report.pdf".to_string()),
                    provider_options: None,
                })],
                provider_options: None,
            }];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_file",
                        "filename": "report.pdf",
                        "file_data": "data:application/pdf;base64,UERGREFUQQ=="
                    }]
                }])
            );
        }
    }

    mod assistant_messages {
        use super::*;

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages › should convert reasoning parts to reasoning items
        #[test]
        fn should_convert_reasoning_parts_to_reasoning_items() {
            let result = convert(json!([{
                "role": "assistant",
                "content": [{
                    "type": "reasoning",
                    "text": "Analyzing the problem step by step",
                }],
            }]));

            assert_eq!(
                result.input,
                json!([{
                    "type": "reasoning",
                    "summary": [],
                    "content": [{
                        "type": "reasoning_text",
                        "text": "Analyzing the problem step by step",
                    }],
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages › should preserve interleaved assistant content order
        #[test]
        fn should_preserve_interleaved_assistant_content_order() {
            let result = convert(json!([{
                "role": "assistant",
                "content": [
                    { "type": "reasoning", "text": "Analyzing the problem" },
                    {
                        "type": "tool-call",
                        "toolCallId": "call_123",
                        "toolName": "search",
                        "input": "{}",
                    },
                    { "type": "text", "text": "Answer after the call" },
                ],
            }]));

            let types: Option<Vec<Value>> = result
                .input
                .as_array()
                .map(|items| items.iter().map(|item| item["type"].clone()).collect());
            assert_eq!(
                types,
                Some(vec![
                    json!("reasoning"),
                    json!("function_call"),
                    json!("message"),
                ])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages › should preserve reasoning item provider data
        #[test]
        fn should_preserve_reasoning_item_provider_data() {
            let result = convert_with(
                "test-provider",
                json!([{
                    "role": "assistant",
                    "content": [{
                        "type": "reasoning",
                        "text": "safe summary",
                        "providerOptions": {
                            "test-provider": {
                                "itemId": "rs_123",
                                "reasoningSummary": [
                                    { "type": "summary_text", "text": "safe summary" },
                                ],
                                "reasoningEncryptedContent": "encrypted-state",
                            },
                        },
                    }],
                }]),
            );

            assert_eq!(
                result.input,
                json!([{
                    "id": "rs_123",
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": "safe summary" }],
                    "content": [{ "type": "reasoning_text", "text": "safe summary" }],
                    "encrypted_content": "encrypted-state",
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages › should preserve output text annotations from provider data
        #[test]
        fn should_preserve_output_text_annotations_from_provider_data() {
            let result = convert_with(
                "test-provider",
                json!([{
                    "role": "assistant",
                    "content": [{
                        "type": "text",
                        "text": "Sourced answer",
                        "providerOptions": {
                            "test-provider": {
                                "itemId": "msg_123",
                                "annotations": [{
                                    "type": "url_citation",
                                    "start_index": 0,
                                    "end_index": 7,
                                    "url": "https://example.com/source",
                                    "title": "Example source",
                                }],
                            },
                        },
                    }],
                }]),
            );

            assert_eq!(
                result.input,
                json!([{
                    "id": "msg_123",
                    "type": "message",
                    "role": "assistant",
                    "content": [{
                        "type": "output_text",
                        "text": "Sourced answer",
                        "annotations": [{
                            "type": "url_citation",
                            "start_index": 0,
                            "end_index": 7,
                            "url": "https://example.com/source",
                            "title": "Example source",
                        }],
                    }],
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages › should convert messages with only a text part to output_text content
        #[test]
        fn convert_assistant_text_part() {
            let prompt = vec![LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: "Hello from assistant".into(),
                    provider_options: None,
                })],
                provider_options: None,
            }];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Hello from assistant"}]
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages › should convert messages with multiple text parts
        #[test]
        fn convert_assistant_multiple_text_parts() {
            let prompt = vec![LanguageModelMessage::Assistant {
                content: vec![
                    AssistantPart::Text(TextPart {
                        text: "First part".into(),
                        provider_options: None,
                    }),
                    AssistantPart::Text(TextPart {
                        text: "Second part".into(),
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "First part"},
                        {"type": "output_text", "text": "Second part"}
                    ]
                }])
            );
        }
    }

    mod tool_messages {
        use super::*;

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › tool messages › should convert tool message with text output
        #[test]
        fn should_convert_tool_message_with_text_output() {
            let result = convert(json!([{
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "call_456",
                    "toolName": "search",
                    "output": {
                        "type": "text",
                        "value": "Search results: Found 5 items",
                    },
                }],
            }]));

            assert_eq!(
                result.input,
                json!([{
                    "call_id": "call_456",
                    "output": "Search results: Found 5 items",
                    "type": "function_call_output",
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › tool messages › should convert tool message with error-text output
        #[test]
        fn should_convert_tool_message_with_error_text_output() {
            let result = convert(json!([{
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "call_789",
                    "toolName": "api_call",
                    "output": {
                        "type": "error-text",
                        "value": "API request failed: timeout",
                    },
                }],
            }]));

            assert_eq!(
                result.input,
                json!([{
                    "call_id": "call_789",
                    "output": "API request failed: timeout",
                    "type": "function_call_output",
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › tool messages › should convert tool message with execution-denied output
        #[test]
        fn should_convert_tool_message_with_execution_denied_output() {
            let result = convert(json!([{
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "call_denied",
                    "toolName": "dangerous_action",
                    "output": {
                        "type": "execution-denied",
                        "reason": "User declined the action",
                    },
                }],
            }]));

            assert_eq!(
                result.input,
                json!([{
                    "call_id": "call_denied",
                    "output": "User declined the action",
                    "type": "function_call_output",
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › tool messages › should convert tool message with content output containing text
        #[test]
        fn should_convert_tool_message_with_content_output_containing_text() {
            let result = convert(json!([{
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "call_content",
                    "toolName": "multi_output",
                    "output": {
                        "type": "content",
                        "value": [
                            { "type": "text", "text": "First result" },
                            { "type": "text", "text": "Second result" },
                        ],
                    },
                }],
            }]));

            assert_eq!(
                result.input,
                json!([{
                    "call_id": "call_content",
                    "output": [
                        { "text": "First result", "type": "input_text" },
                        { "text": "Second result", "type": "input_text" },
                    ],
                    "type": "function_call_output",
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › tool messages › should convert tool message with content output containing image-url
        #[test]
        fn should_convert_tool_message_with_content_output_containing_image_url() {
            let result = convert(json!([{
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "call_image",
                    "toolName": "screenshot",
                    "output": {
                        "type": "content",
                        "value": [{
                            "type": "file",
                            "data": { "type": "url", "url": "https://example.com/image.png" },
                            "mediaType": "image/png",
                        }],
                    },
                }],
            }]));

            assert_eq!(
                result.input,
                json!([{
                    "call_id": "call_image",
                    "output": [{
                        "detail": "auto",
                        "image_url": "https://example.com/image.png",
                        "type": "input_image",
                    }],
                    "type": "function_call_output",
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › tool messages › should preserve image detail provider options in tool output
        #[test]
        fn should_preserve_image_detail_provider_options_in_tool_output() {
            let result = convert_with(
                "test-provider",
                json!([{
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolCallId": "call_image",
                        "toolName": "screenshot",
                        "output": {
                            "type": "content",
                            "value": [
                                {
                                    "type": "file",
                                    "data": { "type": "data", "data": "ZmFrZS1kYXRh" },
                                    "mediaType": "image/png",
                                    "providerOptions": {
                                        "test-provider": { "imageDetail": "low" },
                                    },
                                },
                                {
                                    "type": "file",
                                    "data": {
                                        "type": "url",
                                        "url": "https://example.com/image.png",
                                    },
                                    "mediaType": "image/png",
                                    "providerOptions": {
                                        "test-provider": { "imageDetail": "high" },
                                    },
                                },
                            ],
                        },
                    }],
                }]),
            );

            assert_eq!(
                result.input,
                json!([{
                    "type": "function_call_output",
                    "call_id": "call_image",
                    "output": [
                        {
                            "type": "input_image",
                            "image_url": "data:image/png;base64,ZmFrZS1kYXRh",
                            "detail": "low",
                        },
                        {
                            "type": "input_image",
                            "image_url": "https://example.com/image.png",
                            "detail": "high",
                        },
                    ],
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › tool messages › should convert tool message with multiple tool results
        #[test]
        fn should_convert_tool_message_with_multiple_tool_results() {
            let result = convert(json!([{
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "call_001",
                        "toolName": "get_weather",
                        "output": { "type": "json", "value": { "temp": 72 } },
                    },
                    {
                        "type": "tool-result",
                        "toolCallId": "call_002",
                        "toolName": "get_time",
                        "output": { "type": "text", "value": "3:00 PM" },
                    },
                ],
            }]));

            assert_eq!(
                result.input,
                json!([
                    {
                        "call_id": "call_001",
                        "output": "{\"temp\":72}",
                        "type": "function_call_output",
                    },
                    {
                        "call_id": "call_002",
                        "output": "3:00 PM",
                        "type": "function_call_output",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › tool messages › should convert tool message with json output
        #[test]
        fn convert_tool_message_json_output() {
            let prompt = vec![LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call_123".into(),
                    output: serde_json::from_value(
                        json!({"type": "json", "value": {"temperature": 72, "condition": "sunny"}}),
                    )
                    .unwrap(),
                    tool_name: "get_weather".into(),
                    provider_options: None,
                })],
                provider_options: None,
            }];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "function_call_output",
                    "call_id": "call_123",
                    "output": "{\"temperature\":72,\"condition\":\"sunny\"}"
                }])
            );
        }
    }

    mod provider_reference {
        use super::*;

        // The Rust conversion returns no `Result`; `do_generate` checks the
        // prompt's files before sending. The transport has no canned
        // response, so a call that gets past the check fails with a
        // transport error instead.
        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › provider reference › should throw for file parts with provider references
        #[tokio::test]
        async fn should_throw_for_file_parts_with_provider_references() {
            let fetch = MockFetch::new(Vec::new());
            let provider = create_open_responses(OpenResponsesProviderSettings {
                fetch: Some(fetch.transport()),
                ..OpenResponsesProviderSettings::new("open-responses", "https://example.com/v1")
            })
            .expect("valid settings");
            let model = provider.responses("test-model");

            let result = model
                .do_generate(&CallOptions::new(prompt(json!([{
                    "role": "user",
                    "content": [{
                        "type": "file",
                        "data": {
                            "type": "reference",
                            "reference": { "openResponses": "file-ref-123" },
                        },
                        "mediaType": "image/png",
                    }],
                }]))))
                .await;

            // `UnsupportedFunctionalityError({ functionality })` is
            // `AiMuxError::UnsupportedFunctionality(functionality)`.
            match result {
                Err(aimux_core::error::AiMuxError::UnsupportedFunctionality(functionality)) => {
                    assert_eq!(functionality, "file parts with provider references");
                }
                Err(error) => panic!("unexpected error: {error:?}"),
                Ok(_) => panic!("expected the conversion to throw"),
            }
        }
    }

    mod top_level_only_media_type_resolution {
        use super::*;

        const PNG_BASE64: &str = "iVBORw0KGgo=";

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › top-level-only media type resolution › passes full image/png through unchanged for inline data
        #[test]
        fn passes_full_image_png_through_unchanged_for_inline_data() {
            let result = convert(json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "mediaType": "image/png",
                    "data": { "type": "data", "data": PNG_BASE64 },
                }],
            }]));

            assert_eq!(
                result.input[0]["content"][0],
                json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}"),
                    "detail": "auto",
                })
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
        #[test]
        fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
            let result = convert(json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "mediaType": "image",
                    "data": { "type": "data", "data": PNG_BASE64 },
                }],
            }]));

            assert_eq!(
                result.input[0]["content"][0],
                json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}"),
                    "detail": "auto",
                })
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › top-level-only media type resolution › passes through URL source for top-level-only image
        #[test]
        fn passes_through_url_source_for_top_level_only_image() {
            let result = convert(json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "mediaType": "image",
                    "data": { "type": "url", "url": "https://example.com/x.png" },
                }],
            }]));

            assert_eq!(
                result.input[0]["content"][0],
                json!({
                    "type": "input_image",
                    "image_url": "https://example.com/x.png",
                    "detail": "auto",
                })
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › top-level-only media type resolution › normalizes image/* wildcard via detection
        #[test]
        fn normalizes_image_wildcard_via_detection() {
            let result = convert(json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "mediaType": "image/*",
                    "data": { "type": "data", "data": PNG_BASE64 },
                }],
            }]));

            assert_eq!(
                result.input[0]["content"][0],
                json!({
                    "type": "input_image",
                    "image_url": format!("data:image/png;base64,{PNG_BASE64}"),
                    "detail": "auto",
                })
            );
        }
    }

    mod assistant_messages_with_tool_calls {
        use super::*;

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages with tool calls › should convert assistant message with a single tool-call
        #[test]
        fn convert_assistant_single_tool_call() {
            let prompt = vec![LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call_123".into(),
                    tool_name: "get_weather".into(),
                    input: json!({"location": "San Francisco"}),
                    provider_executed: None,
                    provider_options: None,
                })],
                provider_options: None,
            }];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "function_call",
                    "call_id": "call_123",
                    "name": "get_weather",
                    "arguments": "{\"location\":\"San Francisco\"}"
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages with tool calls › should pass through tool-call string input
        #[test]
        fn convert_assistant_tool_call_string_input() {
            let prompt = vec![LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call_124".into(),
                    tool_name: "get_weather".into(),
                    input: Value::String("{\"location\":\"Berlin\"}".to_string()),
                    provider_executed: None,
                    provider_options: None,
                })],
                provider_options: None,
            }];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([{
                    "type": "function_call",
                    "call_id": "call_124",
                    "name": "get_weather",
                    "arguments": "{\"location\":\"Berlin\"}"
                }])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages with tool calls › should convert assistant message with text and tool-call
        #[test]
        fn convert_assistant_text_and_tool_call() {
            let prompt = vec![LanguageModelMessage::Assistant {
                content: vec![
                    AssistantPart::Text(TextPart {
                        text: "Let me check the weather for you.".into(),
                        provider_options: None,
                    }),
                    AssistantPart::ToolCall(ToolCallPart {
                        tool_call_id: "call_456".into(),
                        tool_name: "get_weather".into(),
                        input: json!({"location": "New York"}),
                        provider_executed: None,
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([
                    {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "Let me check the weather for you."}]
                    },
                    {
                        "type": "function_call",
                        "call_id": "call_456",
                        "name": "get_weather",
                        "arguments": "{\"location\":\"New York\"}"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › assistant messages with tool calls › should convert assistant message with multiple tool-calls
        #[test]
        fn convert_assistant_multiple_tool_calls() {
            let prompt = vec![LanguageModelMessage::Assistant {
                content: vec![
                    AssistantPart::ToolCall(ToolCallPart {
                        tool_call_id: "call_001".into(),
                        tool_name: "get_weather".into(),
                        input: json!({"location": "Paris"}),
                        provider_executed: None,
                        provider_options: None,
                    }),
                    AssistantPart::ToolCall(ToolCallPart {
                        tool_call_id: "call_002".into(),
                        tool_name: "get_time".into(),
                        input: json!({"timezone": "Europe/Paris"}),
                        provider_executed: None,
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([
                    {
                        "type": "function_call",
                        "call_id": "call_001",
                        "name": "get_weather",
                        "arguments": "{\"location\":\"Paris\"}"
                    },
                    {
                        "type": "function_call",
                        "call_id": "call_002",
                        "name": "get_time",
                        "arguments": "{\"timezone\":\"Europe/Paris\"}"
                    }
                ])
            );
        }
    }

    mod message_chains {
        use super::*;

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › message chains › should convert user - assistant - user message chain
        #[test]
        fn convert_user_assistant_user_chain() {
            let prompt = vec![
                LanguageModelMessage::user_text("What is the capital of France?"),
                LanguageModelMessage::Assistant {
                    content: vec![AssistantPart::Text(TextPart {
                        text: "The capital of France is Paris.".into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
                LanguageModelMessage::user_text("And what about Germany?"),
            ];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What is the capital of France?"}]},
                    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "The capital of France is Paris."}]},
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "And what about Germany?"}]}
                ])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › message chains › should convert user - assistant (tool call) - tool message chain
        #[test]
        fn convert_user_assistant_tool_tool_chain() {
            let prompt = vec![
                LanguageModelMessage::user_text("What is the weather in Tokyo?"),
                LanguageModelMessage::Assistant {
                    content: vec![AssistantPart::ToolCall(ToolCallPart {
                        tool_call_id: "call_weather".into(),
                        tool_name: "get_weather".into(),
                        input: json!({"location": "Tokyo"}),
                        provider_executed: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                },
                LanguageModelMessage::Tool {
                    content: vec![ToolPart::ToolResult(ToolResultPart {
                        tool_call_id: "call_weather".into(),
                        output: serde_json::from_value(json!({"type": "json", "value": {"temperature": 25, "condition": "cloudy"}})).unwrap(),
                        tool_name: "get_weather".into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
            ];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What is the weather in Tokyo?"}]},
                    {"type": "function_call", "call_id": "call_weather", "name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"},
                    {"type": "function_call_output", "call_id": "call_weather", "output": "{\"temperature\":25,\"condition\":\"cloudy\"}"}
                ])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/convert-to-open-responses-input.test.ts › convertToOpenResponsesInput › message chains › should convert a tool roundtrip with follow-up assistant message
        #[test]
        fn convert_tool_roundtrip_with_followup_assistant() {
            let prompt = vec![
                LanguageModelMessage::user_text("What is the weather in Tokyo?"),
                LanguageModelMessage::Assistant {
                    content: vec![AssistantPart::ToolCall(ToolCallPart {
                        tool_call_id: "call_weather".into(),
                        tool_name: "get_weather".into(),
                        input: Value::String("{\"location\":\"Tokyo\"}".to_string()),
                        provider_executed: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                },
                LanguageModelMessage::Tool {
                    content: vec![ToolPart::ToolResult(ToolResultPart {
                        tool_call_id: "call_weather".into(),
                        output: serde_json::from_value(json!({"type": "json", "value": {"temperature": 25, "condition": "cloudy"}})).unwrap(),
                        tool_name: "get_weather".into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
                LanguageModelMessage::Assistant {
                    content: vec![AssistantPart::Text(TextPart {
                        text: "It is 25 C and cloudy in Tokyo.".into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
            ];
            let (input, _, _) = convert_to_open_responses_input(&prompt);
            assert_eq!(
                input,
                json!([
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What is the weather in Tokyo?"}]},
                    {"type": "function_call", "call_id": "call_weather", "name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"},
                    {"type": "function_call_output", "call_id": "call_weather", "output": "{\"temperature\":25,\"condition\":\"cloudy\"}"},
                    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "It is 25 C and cloudy in Tokyo."}]}
                ])
            );
        }
    }
}

mod convert_tests {
    use super::*;

    // -- System messages --

    /// partial: `should join multiple system messages with newlines`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_multiple_system_messages_joined_with_newlines() {
        let prompt = vec![
            LanguageModelMessage::System {
                content: "You are a helpful assistant.".into(),
                provider_options: None,
            },
            LanguageModelMessage::System {
                content: "Always be concise.".into(),
                provider_options: None,
            },
        ];
        let (_, instructions, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            instructions,
            Some("You are a helpful assistant.\nAlways be concise.".to_string())
        );
    }

    // -- User messages --

    /// partial: `should convert image file parts with base64 data to input_image`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_image_file_base64_to_input_image() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Base64("ZmFrZS1kYXRh".into()),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, warnings) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_image", "image_url": "data:image/png;base64,ZmFrZS1kYXRh", "detail": "auto"}]
            }])
        );
        assert!(warnings.is_empty());
    }

    /// partial: `should convert image file parts with URL data to input_image`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_image_file_url_to_input_image() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Url {
                    original_url: None,
                    url: "https://example.com/image.png".into(),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_image", "image_url": "https://example.com/image.png", "detail": "auto"}]
            }])
        );
    }

    // -- Tool messages --

    /// partial: `should convert tool message with text output`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_tool_message_text_output() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_456".into(),
                output: serde_json::from_value(
                    json!({"type": "text", "value": "Search results: Found 5 items"}),
                )
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_456",
                "output": "Search results: Found 5 items"
            }])
        );
    }

    /// partial: `should convert tool message with error-text output`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_tool_message_error_text_output() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_789".into(),
                output: serde_json::from_value(
                    json!({"type": "error-text", "value": "API request failed: timeout"}),
                )
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_789",
                "output": "API request failed: timeout"
            }])
        );
    }

    /// partial: `should convert tool message with execution-denied output`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_tool_message_execution_denied_output() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_denied".into(),
                output: serde_json::from_value(
                    json!({"type": "execution-denied", "reason": "User declined the action"}),
                )
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_denied",
                "output": "User declined the action"
            }])
        );
    }

    /// partial: `should convert tool message with content output containing text`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_tool_message_content_output_text() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_content".into(),
                output: serde_json::from_value(json!({
                    "type": "content",
                    "value": [
                        {"type": "text", "text": "First result"},
                        {"type": "text", "text": "Second result"}
                    ]
                }))
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_content",
                "output": [
                    {"type": "input_text", "text": "First result"},
                    {"type": "input_text", "text": "Second result"}
                ]
            }])
        );
    }

    /// partial: `should convert tool message with multiple tool results`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_tool_message_multiple_results() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![
                ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call_001".into(),
                    output: serde_json::from_value(json!({"type": "json", "value": {"temp": 72}}))
                        .unwrap(),
                    tool_name: "get_weather".into(),
                    provider_options: None,
                }),
                ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call_002".into(),
                    output: serde_json::from_value(json!({"type": "text", "value": "3:00 PM"}))
                        .unwrap(),
                    tool_name: "get_weather".into(),
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([
                {
                    "type": "function_call_output",
                    "call_id": "call_001",
                    "output": "{\"temp\":72}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_002",
                    "output": "3:00 PM"
                }
            ])
        );
    }

    // -- Provider reference (warning, not error in Rust) --

    /// partial: `should throw for file parts with provider references`; full port in src/open_responses/tests/responses/convert_to_input.rs.
    #[test]
    fn convert_file_reference_produces_warning() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Reference {
                    reference: [("openResponses".into(), "file-ref-123".into())]
                        .into_iter()
                        .collect(),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, warnings) = convert_to_open_responses_input(&prompt);
        // The reference part is skipped, so user content is empty array.
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": []
            }])
        );
        assert!(!warnings.is_empty());
    }
}

/// TS: `convertToOpenResponsesInput` (convert-to-open-responses-input.ts:279-305):
/// an assistant text part's `providerOptions.<name>.itemId` becomes the
/// message `id`, its `url_citation` annotations are replayed, and text parts
/// with different item ids go out as separate messages.
#[test]
fn assistant_text_replays_the_item_id_and_annotations() {
    let citation = json!({
        "type": "url_citation", "start_index": 0, "end_index": 7,
        "url": "https://example.com/source", "title": "Example source"
    });
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "assistant",
        "content": [
            { "type": "text", "text": "Sourced answer",
              "providerOptions": { "open-responses": { "itemId": "msg_1", "annotations": [citation] } } },
            { "type": "text", "text": "Next",
              "providerOptions": { "open-responses": { "itemId": "msg_2" } } }
        ]
    }]))
    .unwrap();
    let (input, _, _) = convert_to_open_responses_input(&prompt);
    assert_eq!(
        input,
        json!([
            { "type": "message", "role": "assistant", "id": "msg_1",
              "content": [{ "type": "output_text", "text": "Sourced answer", "annotations": [citation] }] },
            { "type": "message", "role": "assistant", "id": "msg_2",
              "content": [{ "type": "output_text", "text": "Next" }] }
        ])
    );
}

/// TS: `convertToOpenResponsesInput` (convert-to-open-responses-input.ts:88-98):
/// a user image carries `detail` from `providerOptions.<name>.imageDetail`,
/// `auto` when it is not set.
#[test]
fn user_images_carry_the_image_detail() {
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "user",
        "content": [
            { "type": "file", "mediaType": "image/png", "data": { "type": "data", "data": "AAEC" },
              "providerOptions": { "open-responses": { "imageDetail": "low" } } },
            { "type": "file", "mediaType": "image/jpeg", "data": { "type": "url", "url": "https://example.com/a.jpg" } }
        ]
    }]))
    .unwrap();
    let (input, _, _) = convert_to_open_responses_input(&prompt);
    assert_eq!(
        input,
        json!([{ "type": "message", "role": "user", "content": [
            { "type": "input_image", "image_url": "data:image/png;base64,AAEC", "detail": "low" },
            { "type": "input_image", "image_url": "https://example.com/a.jpg", "detail": "auto" }
        ] }])
    );
}

/// TS: `convertToOpenResponsesInput` (convert-to-open-responses-input.ts:91-108):
/// inline user files go out with `resolveFullMediaType`, so a partial media
/// type such as `image` is detected from the bytes.
#[test]
fn inline_user_files_resolve_the_full_media_type() {
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "user",
        "content": [{ "type": "file", "mediaType": "image",
                      "data": { "type": "data", "data": "iVBORw0KGgoAAAANSUhEUg==" } }]
    }]))
    .unwrap();
    let (input, _, _) = convert_to_open_responses_input(&prompt);
    assert_eq!(
        input[0]["content"][0]["image_url"],
        json!("data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==")
    );
}

/// TS: `convertToOpenResponsesInput` (convert-to-open-responses-input.ts:306-336):
/// an assistant tool call's `providerOptions.<name>.itemId` becomes the
/// `function_call` item `id`.
#[test]
fn tool_calls_replay_the_item_id() {
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "assistant",
        "content": [{ "type": "tool-call", "toolCallId": "call_1", "toolName": "weather",
                      "input": { "city": "Paris" },
                      "providerOptions": { "open-responses": { "itemId": "fc_1" } } }]
    }]))
    .unwrap();
    let (input, _, _) = convert_to_open_responses_input(&prompt);
    assert_eq!(
        input,
        json!([{ "type": "function_call", "id": "fc_1", "call_id": "call_1",
                 "name": "weather", "arguments": "{\"city\":\"Paris\"}" }])
    );
}
