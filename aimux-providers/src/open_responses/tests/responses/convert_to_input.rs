//! Port of `src/responses/convert-to-open-responses-input.test.ts`.
//!
//! Cases whose old integration test in `tests/open_responses_test.rs` already
//! asserts every upstream expectation carry their marker there instead.

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::options::CallOptions;
use serde_json::{Value, json};

use crate::open_responses::{
    OpenResponsesProviderSettings, convert_to_open_responses_input,
    convert_to_open_responses_input_with_namespace, create_open_responses,
};
use crate::test_support::mock_fetch::MockFetch;

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
}
