//! Upstream `convert-to-groq-chat-messages.test.ts` (the cases that are not in
//! the integration tests).

use aimux_core::language_model_message::FilePart;
use serde_json::json;

use super::*;

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
