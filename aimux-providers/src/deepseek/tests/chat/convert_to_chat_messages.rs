//! Port of `src/chat/convert-to-deepseek-chat-messages.test.ts`
//! (`@ai-sdk/deepseek` 3.0.56). The six cases of "message names", "tool calls"
//! and "assistant prefix completion" call the function directly; the others
//! observe it through the request body of a `wiremock` server (`support.rs`).
//!
//! Upstream writes prompts as object literals. Here they are JSON, read through
//! the request path's own serde shapes. The one `Uint8Array` payload is built as
//! `FileBytes::Binary` directly: JSON can only carry base64, which parses to the
//! other variant.

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    FilePart, LanguageModelMessage, LanguageModelPrompt, ToolPart, ToolResultContent,
    ToolResultOutput, ToolResultPart,
};
use aimux_core::shared::{FileBytes, FileData};
use serde_json::{Value, json};

use crate::deepseek::chat::convert_to_chat_messages::convert_to_deepseek_chat_messages;

use super::support::*;

/// A prompt, read as the request path reads it.
fn prompt_from(value: Value) -> LanguageModelPrompt {
    serde_json::from_value(value).expect("the prompt literal is valid")
}

/// A tool-result content part, read as the request path reads it.
fn tool_result_content(value: Value) -> ToolResultContent {
    serde_json::from_value(value).expect("the tool result content literal is valid")
}

mod convert_to_deep_seek_chat_messages {
    use super::*;

    mod message_names {
        use super::*;

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › message names › should serialize names for system, user, and assistant messages
        #[test]
        fn should_serialize_names_for_system_user_and_assistant_messages() {
            let prompt = prompt_from(json!([
                {
                    "role": "system",
                    "content": "You are a helpful assistant.",
                    "providerOptions": { "deepseek": { "name": "guide" } },
                },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Describe this image." },
                        {
                            "type": "file",
                            "data": { "type": "url", "url": "https://example.com/image.png" },
                            "mediaType": "image/png",
                        },
                    ],
                    "providerOptions": { "deepseek": { "name": "alice" } },
                },
                {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "I will inspect it." },
                        {
                            "type": "tool-call",
                            "toolCallId": "call-1",
                            "toolName": "inspectImage",
                            "input": { "detail": "high" },
                        },
                    ],
                    "providerOptions": { "deepseek": { "name": "vision_assistant" } },
                },
            ]));

            let result = convert_to_deepseek_chat_messages(
                &prompt,
                None,
                "deepseek-v4-flash-vision-exp",
                "deepseek",
                false,
            )
            .expect("the prompt converts");

            assert_eq!(
                Value::Array(result.messages),
                json!([
                    {
                        "role": "system",
                        "content": "You are a helpful assistant.",
                        "name": "guide",
                    },
                    {
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "Describe this image." },
                            {
                                "type": "image_url",
                                "image_url": { "url": "https://example.com/image.png" },
                            },
                        ],
                        "name": "alice",
                    },
                    {
                        "role": "assistant",
                        "content": "I will inspect it.",
                        "name": "vision_assistant",
                        // V4 requires the field on every assistant turn.
                        "reasoning_content": "",
                        "tool_calls": [
                            {
                                "id": "call-1",
                                "type": "function",
                                "function": {
                                    "name": "inspectImage",
                                    "arguments": "{\"detail\":\"high\"}",
                                },
                            },
                        ],
                    },
                ])
            );
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › message names › should serialize a name from a custom provider options namespace
        #[test]
        fn should_serialize_a_name_from_a_custom_provider_options_namespace() {
            let prompt = prompt_from(json!([{
                "role": "user",
                "content": [{ "type": "text", "text": "Hello" }],
                "providerOptions": { "azure": { "name": "alice" } },
            }]));

            let result =
                convert_to_deepseek_chat_messages(&prompt, None, "deepseek-chat", "azure", false)
                    .expect("the prompt converts");

            assert_eq!(
                Value::Array(result.messages),
                json!([{ "role": "user", "content": "Hello", "name": "alice" }])
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › message names › should ignore a name on a tool message with an unsupported warning
        #[tokio::test]
        async fn convert_should_ignore_a_name_on_a_tool_message_with_an_unsupported_warning() {
            let options = options_for(vec![with_options(
                LanguageModelMessage::Tool {
                    content: vec![ToolPart::ToolResult(ToolResultPart {
                        tool_call_id: ("call-1").into(),
                        output: ToolResultOutput::Text {
                            value: "sunny".into(),
                            provider_options: None,
                        },
                        tool_name: "weather_tool".into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
                json!({ "deepseek": { "name": "weather_tool" } }),
            )]);
            let result = generate_result("deepseek-chat", &options).await;
            assert_eq!(
                result.request.and_then(|r| r.body).unwrap()["messages"],
                json!([{ "role": "tool", "tool_call_id": "call-1", "content": "sunny" }])
            );
            assert_eq!(
                warning_values(&result.warnings),
                warning_values(&[Warning::Unsupported {
                    feature: "message name on tool messages".to_string(),
                    details: None,
                }])
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › message names › should reject a non-string name
        #[tokio::test]
        async fn convert_should_reject_a_non_string_name() {
            let options = options_for(vec![with_options(
                LanguageModelMessage::user_text("Hello"),
                json!({ "deepseek": { "name": 123 } }),
            )]);
            let message = generate_error("deepseek-chat", &options).await;
            assert!(message.contains("invalid provider options"), "{message}");
        }
    }

    mod tool_calls {
        use super::*;

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › tool calls › should convert inline, URL, and referenced images in tool results to content parts
        #[test]
        fn should_convert_inline_url_and_referenced_images_in_tool_results_to_content_parts() {
            let inline_png = ToolResultContent::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Binary(vec![0, 1, 2, 3]),
                },
                media_type: "image/png".to_string(),
                filename: None,
                provider_options: None,
            });
            let prompt = vec![LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "inspectImage".to_string(),
                    output: ToolResultOutput::Content {
                        value: vec![
                            tool_result_content(
                                json!({ "type": "text", "text": "Screenshot captured." }),
                            ),
                            inline_png,
                            tool_result_content(json!({
                                "type": "file",
                                "data": { "type": "url", "url": "https://example.com/image.webp" },
                                "mediaType": "image/webp",
                                "providerOptions": { "deepseek": { "imageDetail": "low" } },
                            })),
                            tool_result_content(json!({
                                "type": "file",
                                "data": {
                                    "type": "reference",
                                    "reference": {
                                        "deepseek": "file-api-deepseek",
                                        "openai": "file-openai",
                                    },
                                },
                                "mediaType": "image/png",
                            })),
                        ],
                    },
                    provider_options: None,
                })],
                provider_options: None,
            }];

            let result = convert_to_deepseek_chat_messages(
                &prompt,
                None,
                "deepseek-flash",
                "deepseek",
                false,
            )
            .expect("the prompt converts");

            assert_eq!(
                Value::Array(result.messages),
                json!([{
                    "role": "tool",
                    "tool_call_id": "call-1",
                    "content": [
                        { "type": "text", "text": "Screenshot captured." },
                        {
                            "type": "image_url",
                            "image_url": { "url": "data:image/png;base64,AAECAw==" },
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": "https://example.com/image.webp",
                                "detail": "low",
                            },
                        },
                        { "type": "file", "file_id": "file-api-deepseek" },
                    ],
                }])
            );
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › tool calls › should keep tool result content without images JSON-stringified
        #[test]
        fn should_keep_tool_result_content_without_images_json_stringified() {
            let prompt = prompt_from(json!([{
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "call-1",
                    "toolName": "getWeather",
                    "output": {
                        "type": "content",
                        "value": [{ "type": "text", "text": "It is sunny today" }],
                    },
                }],
            }]));

            let result = convert_to_deepseek_chat_messages(
                &prompt,
                None,
                "deepseek-chat",
                "deepseek",
                false,
            )
            .expect("the prompt converts");

            assert_eq!(
                Value::Array(result.messages),
                json!([{
                    "role": "tool",
                    "tool_call_id": "call-1",
                    "content": "[{\"type\":\"text\",\"text\":\"It is sunny today\"}]",
                }])
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › tool calls › should stringify arguments to tool calls
        #[tokio::test]
        async fn convert_should_stringify_arguments_to_tool_calls() {
            let result = generate_result("deepseek-chat", &options_for(tool_turn(false))).await;
            assert_eq!(
                result.request.and_then(|r| r.body).unwrap()["messages"],
                json!([
                    // upstream sends the empty text as `content: ""`, not null.
                    { "role": "assistant", "content": "", "tool_calls": wire_tool_call() },
                    { "role": "tool", "tool_call_id": "quux", "content": "{\"oof\":\"321rab\"}" }
                ])
            );
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › tool calls › should handle text output type in tool results
        #[tokio::test]
        async fn convert_should_handle_text_output_type_in_tool_results() {
            let messages = wire_messages(
                "deepseek-chat",
                vec![
                    LanguageModelMessage::Assistant {
                        content: vec![AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id: ("call-1").into(),
                            tool_name: ("getWeather").into(),
                            input: json!({ "query": "weather" }),
                            provider_executed: None,
                            provider_options: None,
                        })],
                        provider_options: None,
                    },
                    LanguageModelMessage::Tool {
                        content: vec![ToolPart::ToolResult(ToolResultPart {
                            tool_call_id: ("call-1").into(),
                            output: ToolResultOutput::Text {
                                value: "It is sunny today".into(),
                                provider_options: None,
                            },
                            tool_name: "getWeather".into(),
                            provider_options: None,
                        })],
                        provider_options: None,
                    },
                ],
            )
            .await;
            assert_eq!(
                messages[1],
                json!({ "role": "tool", "tool_call_id": "call-1", "content": "It is sunny today" })
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › tool calls › should support reasoning content in tool calls
        #[tokio::test]
        async fn convert_should_support_reasoning_content_in_tool_calls() {
            let mut prompt = vec![LanguageModelMessage::user_text("Hello")];
            prompt.extend(tool_turn(true));
            let messages = wire_messages("deepseek-chat", prompt).await;
            assert_eq!(
                messages[1],
                json!({
                    "role": "assistant", "content": "",
                    "reasoning_content": "I think the tool will return the correct value.",
                    "tool_calls": wire_tool_call()
                })
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › tool calls › should filter out reasoning content from turns before the last user message
        #[tokio::test]
        async fn convert_should_filter_out_reasoning_content_from_turns_before_the_last_user_message()
         {
            let mut prompt = vec![LanguageModelMessage::user_text("Hello")];
            prompt.extend(tool_turn(true));
            prompt.push(LanguageModelMessage::user_text("Goodbye"));
            let messages = wire_messages("deepseek-chat", prompt).await;
            assert_eq!(
                messages[1],
                json!({ "role": "assistant", "content": "", "tool_calls": wire_tool_call() })
            );
        }
    }

    mod assistant_prefix_completion {
        use super::*;

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › assistant prefix completion › should serialize prefix true on the final assistant message
        #[test]
        fn should_serialize_prefix_true_on_the_final_assistant_message() {
            let prompt = prompt_from(json!([
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Complete this sentence." }],
                },
                {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "The answer is" }],
                    "providerOptions": { "deepseek": { "prefix": true } },
                },
            ]));

            let result =
                convert_to_deepseek_chat_messages(&prompt, None, "deepseek-chat", "deepseek", true)
                    .expect("the prompt converts");

            // Upstream's `reasoning_content` and `tool_calls` are undefined, so
            // they are absent from the JSON.
            assert_eq!(
                Value::Array(result.messages),
                json!([
                    { "role": "user", "content": "Complete this sentence." },
                    { "role": "assistant", "content": "The answer is", "prefix": true },
                ])
            );
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › assistant prefix completion › should reject prefix completion without a beta base URL capability
        #[test]
        fn should_reject_prefix_completion_without_a_beta_base_url_capability() {
            let prompt = prompt_from(json!([{
                "role": "assistant",
                "content": [{ "type": "text", "text": "The answer is" }],
                "providerOptions": { "deepseek": { "prefix": true } },
            }]));

            let error = convert_to_deepseek_chat_messages(
                &prompt,
                None,
                "deepseek-chat",
                "deepseek",
                false,
            )
            .err()
            .expect("prefix completion is rejected without the beta endpoint");

            assert!(
                matches!(error, AiMuxError::UnsupportedFunctionality(_)),
                "{error}"
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › assistant prefix completion › should reject prefix completion on a non-assistant message
        #[tokio::test]
        async fn convert_should_reject_prefix_completion_on_a_non_assistant_message() {
            let message = generate_error(
                "deepseek-chat",
                &options_for(vec![with_options(
                    LanguageModelMessage::user_text("Hello"),
                    json!({ "deepseek": { "prefix": true } }),
                )]),
            )
            .await;
            assert!(
        message.contains(
            "DeepSeek assistant prefix completion requires `prefix: true` on an assistant message."
        ),
        "{message}"
    );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › assistant prefix completion › should reject prefix completion on a non-final assistant message
        #[tokio::test]
        async fn convert_should_reject_prefix_completion_on_a_non_final_assistant_message() {
            let server = json_server(text_response()).await;
            let prompt = vec![
                with_options(
                    LanguageModelMessage::Assistant {
                        content: vec![AssistantPart::Text(TextPart {
                            text: ("The answer is").into(),
                            provider_options: None,
                        })],
                        provider_options: None,
                    },
                    json!({ "deepseek": { "prefix": true } }),
                ),
                LanguageModelMessage::user_text("Continue"),
            ];
            let error = beta_chat(&server, "deepseek-chat")
                .do_generate(&options_for(prompt))
                .await
                .unwrap_err();
            assert!(
        error.to_string().contains(
            "DeepSeek assistant prefix completion requires the prefixed assistant message to be the final message."
        ),
        "{error}"
    );
        }
    }

    mod user_messages {
        use super::*;

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should convert messages with only a text part to a string content
        #[tokio::test]
        async fn convert_should_convert_messages_with_only_a_text_part_to_a_string_content() {
            let messages = wire_messages(
                "deepseek-chat",
                vec![LanguageModelMessage::user_text("Hello")],
            )
            .await;
            assert_eq!(messages, [json!({ "role": "user", "content": "Hello" })]);
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should convert image data to an image URL content part
        #[tokio::test]
        async fn convert_should_convert_image_data_to_an_image_url_content_part() {
            let messages = wire_messages(
                "deepseek-chat",
                vec![LanguageModelMessage::User {
                    content: vec![
                        UserPart::Text(TextPart {
                            text: ("Hello").into(),
                            provider_options: None,
                        }),
                        png_data(),
                    ],
                    provider_options: None,
                }],
            )
            .await;
            assert_eq!(
                messages,
                [json!({ "role": "user", "content": [
            { "type": "text", "text": "Hello" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAECAw==" } }
        ] })]
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should convert an image URL to an image URL content part
        #[tokio::test]
        async fn convert_should_convert_an_image_url_to_an_image_url_content_part() {
            let messages = wire_messages(
                "deepseek-chat",
                vec![LanguageModelMessage::User {
                    content: vec![
                        UserPart::Text(TextPart {
                            text: ("Hello").into(),
                            provider_options: None,
                        }),
                        UserPart::File(FilePart {
                            data: FileData::Url {
                                url: ("https://example.com/image.png").into(),
                                original_url: None,
                            },
                            media_type: ("image/png").into(),
                            filename: None,
                            provider_options: None,
                        }),
                    ],
                    provider_options: None,
                }],
            )
            .await;
            assert_eq!(
                messages,
                [json!({ "role": "user", "content": [
            { "type": "text", "text": "Hello" },
            { "type": "image_url", "image_url": { "url": "https://example.com/image.png" } }
        ] })]
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should pass imageDetail to image URL content parts
        #[tokio::test]
        async fn convert_should_pass_image_detail_to_image_url_content_parts() {
            let messages = wire_messages(
                "deepseek-v4-flash-vision-exp",
                vec![LanguageModelMessage::User {
                    content: vec![file_with_options(
                        UserPart::File(FilePart {
                            data: FileData::Url {
                                url: ("https://example.com/image.webp").into(),
                                original_url: None,
                            },
                            media_type: ("image/webp").into(),
                            filename: None,
                            provider_options: None,
                        }),
                        json!({ "deepseek": { "imageDetail": "low" } }),
                    )],
                    provider_options: None,
                }],
            )
            .await;
            assert_eq!(
                messages,
                [json!({ "role": "user", "content": [
            { "type": "image_url", "image_url": { "url": "https://example.com/image.webp", "detail": "low" } }
        ] })]
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should convert inline image data to file_data and preserve its filename
        #[tokio::test]
        async fn convert_should_convert_inline_image_data_to_file_data_and_preserve_its_filename() {
            let part = UserPart::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Base64("AAECAw==".to_string()),
                },
                media_type: "image/jpg".to_string(),
                filename: Some("sample.jpg".to_string()),
                provider_options: Some(
                    provider_namespace("deepseek", json!({ "fileData": true })).unwrap(),
                ),
            });
            let messages = wire_messages(
                "deepseek-v4-flash-vision-exp",
                vec![LanguageModelMessage::User {
                    content: vec![part],
                    provider_options: None,
                }],
            )
            .await;
            assert_eq!(
                messages,
                [json!({ "role": "user", "content": [
            { "type": "file", "file_data": "data:image/jpeg;base64,AAECAw==", "filename": "sample.jpg" }
        ] })]
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should reject imageDetail together with fileData
        #[tokio::test]
        async fn convert_should_reject_image_detail_together_with_file_data() {
            let part = file_with_options(
                png_data(),
                json!({ "deepseek": { "fileData": true, "imageDetail": "high" } }),
            );
            let message = generate_error(
                "deepseek-v4-flash-vision-exp",
                &options_for(vec![LanguageModelMessage::User {
                    content: vec![part],
                    provider_options: None,
                }]),
            )
            .await;
            assert!(
                message.contains("DeepSeek `imageDetail` cannot be combined with `fileData`."),
                "{message}"
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should reject image URLs longer than 8192 characters
        #[tokio::test]
        async fn convert_should_reject_image_urls_longer_than_8192_characters() {
            let url = format!("https://example.com/{}", "a".repeat(8192));
            let message = generate_error(
                "deepseek-v4-flash-vision-exp",
                &options_for(vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Url {
                            url: (url),
                            original_url: None,
                        },
                        media_type: ("image/png").into(),
                        filename: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                }]),
            )
            .await;
            assert!(
                message.contains("DeepSeek image URLs must not exceed 8192 characters."),
                "{message}"
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should reject unsupported image formats
        #[tokio::test]
        async fn convert_should_reject_unsupported_image_formats() {
            let message = generate_error(
                "deepseek-v4-flash-vision-exp",
                &options_for(vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Base64(("AAECAw==").into()),
                        },
                        media_type: ("image/svg+xml").into(),
                        filename: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                }]),
            )
            .await;
            assert!(
                message.contains("DeepSeek supports JPEG, PNG, GIF, and WebP image inputs."),
                "{message}"
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should convert an image provider reference to a file content part
        #[tokio::test]
        async fn convert_should_convert_an_image_provider_reference_to_a_file_content_part() {
            let messages = wire_messages(
                "deepseek-v4-flash-vision-exp",
                vec![LanguageModelMessage::User {
                    content: vec![
                UserPart::Text(TextPart {
                    text: ("Hello").into(),
                    provider_options: None,
                }),
                UserPart::File(FilePart {
                    data: FileData::Reference {
                        reference: serde_json::from_value(
                            json!({ "deepseek": "file-api-deepseek", "openai": "file-openai" }),
                        )
                        .unwrap(),
                    },
                    media_type: ("image/png").into(),
                    filename: None,
                    provider_options: None,
                }),
            ],
                    provider_options: None,
                }],
            )
            .await;
            assert_eq!(
                messages,
                [json!({ "role": "user", "content": [
            { "type": "text", "text": "Hello" },
            { "type": "file", "file_id": "file-api-deepseek" }
        ] })]
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should throw when an image reference has no DeepSeek identifier
        #[tokio::test]
        async fn convert_should_throw_when_an_image_reference_has_no_deepseek_identifier() {
            let message = generate_error(
                "deepseek-v4-flash-vision-exp",
                &options_for(vec![LanguageModelMessage::User {
                    content: vec![UserPart::File(FilePart {
                        data: FileData::Reference {
                            reference: serde_json::from_value(json!({ "openai": "file-openai" }))
                                .unwrap(),
                        },
                        media_type: ("image/png").into(),
                        filename: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                }]),
            )
            .await;
            assert!(message.contains("deepseek"), "{message}");
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › user messages › should warn about unsupported non-image file parts
        #[tokio::test]
        async fn convert_should_warn_about_unsupported_non_image_file_parts() {
            let result = generate_result(
                "deepseek-chat",
                &options_for(vec![LanguageModelMessage::User {
                    content: vec![
                        UserPart::Text(TextPart {
                            text: ("Hello").into(),
                            provider_options: None,
                        }),
                        UserPart::File(FilePart {
                            data: FileData::Data {
                                data: FileBytes::Base64(("AAECAw==").into()),
                            },
                            media_type: ("application/pdf").into(),
                            filename: None,
                            provider_options: None,
                        }),
                    ],
                    provider_options: None,
                }]),
            )
            .await;
            assert_eq!(
                result.request.and_then(|r| r.body).unwrap()["messages"],
                json!([{ "role": "user", "content": "Hello" }])
            );
            assert_eq!(
                warning_values(&result.warnings),
                warning_values(&[Warning::Unsupported {
                    feature: "user message part type: file".to_string(),
                    details: None,
                }])
            );
        }
    }

    mod deepseek_v4_thinking_mode {
        use super::*;

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › deepseek-v4 thinking mode › should preserve reasoning_content from prior turns for deepseek-v4
        #[tokio::test]
        async fn convert_should_preserve_reasoning_content_from_prior_turns_for_deepseek_v4() {
            let mut prompt = vec![LanguageModelMessage::user_text("Hello")];
            prompt.extend(tool_turn(true));
            prompt.push(LanguageModelMessage::user_text("Goodbye"));
            let messages = wire_messages("deepseek-v4-pro", prompt).await;
            assert_eq!(
                messages[1],
                json!({
                    "role": "assistant", "content": "",
                    "reasoning_content": "I think the tool will return the correct value.",
                    "tool_calls": wire_tool_call()
                })
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › deepseek-v4 thinking mode › should preserve reasoning_content from prior turns for the deepseek-flash alias
        #[tokio::test]
        async fn convert_should_preserve_reasoning_content_from_prior_turns_for_the_deepseek_flash_alias()
         {
            let messages = wire_messages(
                "deepseek-flash",
                vec![
                    LanguageModelMessage::user_text("Hello"),
                    LanguageModelMessage::Assistant {
                        content: vec![
                            AssistantPart::Reasoning(ReasoningPart {
                                text: ("Prior-turn reasoning.").into(),
                                provider_options: None,
                            }),
                            AssistantPart::Text(TextPart {
                                text: ("Hi there").into(),
                                provider_options: None,
                            }),
                        ],
                        provider_options: None,
                    },
                    LanguageModelMessage::user_text("Again"),
                ],
            )
            .await;
            assert_eq!(
                messages[1],
                json!({ "role": "assistant", "content": "Hi there", "reasoning_content": "Prior-turn reasoning." })
            );
        }

        /// upstream: @ai-sdk/deepseek src/chat/convert-to-deepseek-chat-messages.test.ts › convertToDeepSeekChatMessages › deepseek-v4 thinking mode › should back-fill empty reasoning_content for deepseek-v4 assistant messages with no reasoning part
        #[tokio::test]
        async fn convert_should_back_fill_empty_reasoning_content_for_deepseek_v4_assistant_messages_with_no_reasoning_part()
         {
            let messages = wire_messages(
                "deepseek-v4-pro",
                vec![
                    LanguageModelMessage::user_text("Hello"),
                    LanguageModelMessage::Assistant {
                        content: vec![AssistantPart::Text(TextPart {
                            text: ("Hi there").into(),
                            provider_options: None,
                        })],
                        provider_options: None,
                    },
                    LanguageModelMessage::user_text("Again"),
                ],
            )
            .await;
            assert_eq!(
                messages[1],
                json!({ "role": "assistant", "content": "Hi there", "reasoning_content": "" })
            );
        }
    }
}
