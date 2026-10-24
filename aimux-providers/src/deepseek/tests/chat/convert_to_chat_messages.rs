//! Port of `src/chat/convert-to-deepseek-chat-messages.test.ts`
//! (`@ai-sdk/deepseek` 3.0.56): the six cases of "message names", "tool calls"
//! and "assistant prefix completion" that the other cases of this file do not
//! already cover.
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

use crate::deepseek::convert::convert_to_deepseek_chat_messages;

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
    }
}
