//! Upstream `convert-to-groq-chat-messages.test.ts`: direct calls, plus the
//! older cases (formerly `tests/groq_test.rs`) checked through the
//! `do_generate` request body on a wiremock `MockServer` ([`super::support`]).

use aimux_core::language_model_message::FilePart;
use serde_json::json;

use aimux_core::language_model_message::{
    LanguageModelMessage, LanguageModelPrompt, TextPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData};
use serde_json::Value;

use crate::groq::convert_to_chat_messages::convert_to_groq_chat_messages;

use super::support::*;

/// A text part of a user message.
fn text_part(text: &str) -> UserPart {
    UserPart::Text(TextPart {
        text: text.to_string(),
        provider_options: None,
    })
}

/// A file part of a user message.
fn file_part(data: FileData, media_type: &str) -> UserPart {
    UserPart::File(FilePart {
        data,
        media_type: media_type.to_string(),
        filename: None,
        provider_options: None,
    })
}

/// A prompt holding one user message with `content`.
fn user_prompt(content: Vec<UserPart>) -> LanguageModelPrompt {
    vec![LanguageModelMessage::User {
        content,
        provider_options: None,
    }]
}

mod user_messages {
    use super::*;

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › user messages › should convert messages with image parts from Uint8Array
    #[test]
    fn should_convert_messages_with_image_parts_from_uint8array() {
        let prompt = user_prompt(vec![
            text_part("Hi"),
            file_part(
                FileData::Data {
                    data: FileBytes::Binary(vec![0, 1, 2, 3]),
                },
                "image/png",
            ),
        ]);

        let result = convert_to_groq_chat_messages(&prompt).expect("convertible prompt");

        assert_eq!(
            json!(result),
            json!([
                {
                    "content": [
                        {
                            "text": "Hi",
                            "type": "text",
                        },
                        {
                            "image_url": {
                                "url": "data:image/png;base64,AAECAw==",
                            },
                            "type": "image_url",
                        },
                    ],
                    "role": "user",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › user messages › should convert messages with image parts
    #[tokio::test]
    async fn image_parts_from_base64() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
            content: vec![
                UserPart::Text(TextPart {
                    text: "Hello".into(),
                    provider_options: None,
                }),
                UserPart::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64("AAECAw==".into()),
                    },
                    media_type: "image/png".into(),
                    filename: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body["messages"],
            json!([{
                "content": [
                    { "text": "Hello", "type": "text" },
                    { "image_url": { "url": "data:image/png;base64,AAECAw==" }, "type": "image_url" }
                ],
                "role": "user"
            }])
        );
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › user messages › should convert messages with only a text part to a string content
    #[tokio::test]
    async fn single_text_becomes_string() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body["messages"],
            json!([{ "content": "Hello", "role": "user" }])
        );
    }
}

mod top_level_only_media_type_resolution {
    use super::*;

    const PNG_BASE64: &str = "iVBORw0KGgo=";

    /// The image part of the single user message of `prompt`.
    fn first_image_part(prompt: &LanguageModelPrompt) -> Value {
        let result = convert_to_groq_chat_messages(prompt).expect("convertible prompt");
        result[0]["content"][0].clone()
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › top-level-only media type resolution › passes full image/png through unchanged for inline data
    #[test]
    fn passes_full_image_png_through_unchanged_for_inline_data() {
        let prompt = user_prompt(vec![file_part(
            FileData::Data {
                data: FileBytes::Base64(PNG_BASE64.to_string()),
            },
            "image/png",
        )]);

        assert_eq!(
            first_image_part(&prompt),
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
    #[test]
    fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
        let prompt = user_prompt(vec![file_part(
            FileData::Data {
                data: FileBytes::Base64(PNG_BASE64.to_string()),
            },
            "image",
        )]);

        assert_eq!(
            first_image_part(&prompt),
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › top-level-only media type resolution › passes through URL source for top-level-only image
    #[test]
    fn passes_through_url_source_for_top_level_only_image() {
        let prompt = user_prompt(vec![file_part(
            FileData::Url {
                url: "https://example.com/x.png".to_string(),
                original_url: None,
            },
            "image",
        )]);

        assert_eq!(
            first_image_part(&prompt),
            json!({
                "type": "image_url",
                "image_url": { "url": "https://example.com/x.png" },
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › top-level-only media type resolution › normalizes image/* wildcard via detection
    #[test]
    fn normalizes_image_wildcard_via_detection() {
        let prompt = user_prompt(vec![file_part(
            FileData::Data {
                data: FileBytes::Base64(PNG_BASE64.to_string()),
            },
            "image/*",
        )]);

        assert_eq!(
            first_image_part(&prompt),
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
            })
        );
    }
}

mod tool_calls {
    use super::*;

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should stringify arguments to tool calls
    #[tokio::test]
    async fn tool_call_arguments_stringified() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "quux".into(),
                    tool_name: "thwomp".into(),
                    input: json!({"foo":"bar123"}),
                    provider_executed: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "quux".into(),
                    output: ToolResultOutput::Json {
                        value: json!({"oof":"321rab"}),
                        provider_options: None,
                    },
                    tool_name: "thwomp".into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
        ];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body["messages"],
            json!([
                {
                    "content": "",
                    "role": "assistant",
                    "tool_calls": [{
                        "function": { "arguments": r#"{"foo":"bar123"}"#, "name": "thwomp" },
                        "id": "quux",
                        "type": "function"
                    }]
                },
                { "content": r#"{"oof":"321rab"}"#, "role": "tool", "tool_call_id": "quux" }
            ])
        );
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should send reasoning if present
    #[tokio::test]
    async fn reasoning_in_assistant_message() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Assistant {
            content: vec![
                AssistantPart::Reasoning(ReasoningPart {
                    text: "I think the tool will return the correct value.".into(),
                    provider_options: None,
                }),
                AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "quux".into(),
                    tool_name: "thwomp".into(),
                    input: json!({"foo":"bar123"}),
                    provider_executed: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body["messages"],
            json!([{
                "content": "",
                "reasoning": "I think the tool will return the correct value.",
                "role": "assistant",
                "tool_calls": [{
                    "function": { "arguments": r#"{"foo":"bar123"}"#, "name": "thwomp" },
                    "id": "quux",
                    "type": "function"
                }]
            }])
        );
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should not include reasoning field when no reasoning content is present
    #[tokio::test]
    async fn no_reasoning_field_when_absent() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Assistant {
            content: vec![AssistantPart::Text(TextPart {
                text: "Hello, how can I help you?".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body["messages"],
            json!([{ "content": "Hello, how can I help you?", "role": "assistant" }])
        );
    }

    /// Groq does not support provider file references.
    /// partial: `should throw for file parts with provider references`; upstream checks the message `'file parts with provider references' functionality not supported`, and aimux's `UnsupportedFunctionality` carries only the functionality name (the message field is tracked in #275).
    #[tokio::test]
    async fn file_reference_unsupported() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Reference {
                    reference: [("groq".into(), "file-ref-123".into())].into(),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        // upstream convert-to-groq-chat-messages.ts: a provider reference
        // throws UnsupportedFunctionalityError.
        let result = model.do_generate(&default_options(prompt)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            aimux_core::AiMuxError::UnsupportedFunctionality(_)
        ));
        assert!(
            err.to_string()
                .contains("file parts with provider references"),
            "error should name the unsupported functionality: {err}"
        );
    }
}
