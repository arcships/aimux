use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt, UserPart};
use aimux_core::shared::{FileBytes, FileData};

use crate::openai_compatible::convert::{MessageSpec, convert_messages};

/// A prompt in its AI SDK JSON form.
fn prompt(value: Value) -> LanguageModelPrompt {
    serde_json::from_value(value).expect("prompt")
}

/// Replace the data of file part `part` of user message `message` with raw
/// bytes: the JSON form only carries base64 strings, upstream passes a
/// `Uint8Array` there.
fn with_uint8_array(
    mut prompt: LanguageModelPrompt,
    message: usize,
    part: usize,
    bytes: &[u8],
) -> LanguageModelPrompt {
    let LanguageModelMessage::User { content, .. } = &mut prompt[message] else {
        panic!("message {message} is not a user message");
    };
    let UserPart::File(file) = &mut content[part] else {
        panic!("part {part} is not a file part");
    };
    file.data = FileData::Data {
        data: FileBytes::Binary(bytes.to_vec()),
    };
    prompt
}

/// `convertToOpenAICompatibleChatMessages(prompt, { providerOptionsKey,
/// supportsMultiPartToolContent })`.
fn convert_with(
    prompt: &LanguageModelPrompt,
    provider_options_key: &str,
    supports_multi_part_tool_content: bool,
) -> Result<Value, AiMuxError> {
    convert_messages(
        prompt,
        &MessageSpec {
            metadata_key: provider_options_key,
            supports_multi_part_tool_content,
        },
    )
    .map(Value::Array)
}

/// `convertToOpenAICompatibleChatMessages(prompt)` with upstream's defaults
/// (`providerOptionsKey: 'google'`, `supportsMultiPartToolContent: false`).
fn convert(prompt: &LanguageModelPrompt) -> Value {
    convert_with(prompt, "google", false).expect("conversion succeeds")
}

/// `expect(() => convert(prompt)).toThrow("'<functionality>' functionality not
/// supported")`: an `UnsupportedFunctionalityError` for that functionality.
fn assert_unsupported(prompt: &LanguageModelPrompt, functionality: &str) {
    match convert_with(prompt, "google", false) {
        Err(AiMuxError::UnsupportedFunctionality(actual)) => {
            assert_eq!(actual, functionality);
        }
        other => panic!("expected UnsupportedFunctionality({functionality:?}), got {other:?}"),
    }
}

/// `toMatchObject`: objects match by subset, arrays element-wise with the
/// same length, other values by equality.
fn assert_match_object(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(actual_map), Value::Object(expected_map)) => {
            for (key, expected_value) in expected_map {
                let actual_value = actual_map
                    .get(key)
                    .unwrap_or_else(|| panic!("missing key {key:?} in {actual}"));
                assert_match_object(actual_value, expected_value);
            }
        }
        (Value::Array(actual_items), Value::Array(expected_items)) => {
            assert_eq!(
                actual_items.len(),
                expected_items.len(),
                "array length differs: {actual} vs {expected}"
            );
            for (actual_item, expected_item) in actual_items.iter().zip(expected_items) {
                assert_match_object(actual_item, expected_item);
            }
        }
        _ => assert_eq!(actual, expected),
    }
}

/// `JSON.stringify(value)` (the JSON maps keep insertion order).
fn stringify(value: &Value) -> String {
    value.to_string()
}

mod user_messages {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with only a text part to a string content
    #[test]
    fn should_convert_messages_with_only_a_text_part_to_a_string_content() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [{ "type": "text", "text": "Hello" }],
            },
        ])));

        assert_eq!(result, json!([{ "role": "user", "content": "Hello" }]));
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with image parts
    #[test]
    fn should_convert_messages_with_image_parts() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "Hello" },
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Hello" },
                        {
                            "type": "image_url",
                            "image_url": { "url": "data:image/png;base64,AAECAw==" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with image parts from Uint8Array
    #[test]
    fn should_convert_messages_with_image_parts_from_uint8_array() {
        let result = convert(&with_uint8_array(
            prompt(json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Hi" },
                        {
                            "type": "file",
                            "data": { "type": "data", "data": "<Uint8Array>" },
                            "mediaType": "image/png",
                        },
                    ],
                },
            ])),
            0,
            1,
            &[0, 1, 2, 3],
        ));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Hi" },
                        {
                            "type": "image_url",
                            "image_url": { "url": "data:image/png;base64,AAECAw==" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should handle URL-based images
    #[test]
    fn should_handle_url_based_images() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "url", "url": "https://example.com/image.jpg" },
                        "mediaType": "image/*",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "image_url",
                            "image_url": { "url": "https://example.com/image.jpg" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert video data parts to video_url data URLs
    #[test]
    fn should_convert_video_data_parts_to_video_url_data_urls() {
        let result = convert(&with_uint8_array(
            prompt(json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Describe this video" },
                        {
                            "type": "file",
                            "data": { "type": "data", "data": "<Uint8Array>" },
                            "mediaType": "video/mp4",
                            "providerOptions": {
                                "openaiCompatible": {
                                    "fps": 1,
                                },
                            },
                        },
                    ],
                },
            ])),
            0,
            1,
            &[0, 1, 2, 3],
        ));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Describe this video" },
                        {
                            "type": "video_url",
                            "video_url": { "url": "data:video/mp4;base64,AAECAw==" },
                            "fps": 1,
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert base64-encoded video data parts
    #[test]
    fn should_convert_base64_encoded_video_data_parts() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "video/webm",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "video_url",
                            "video_url": { "url": "data:video/webm;base64,AAECAw==" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should pass through video URLs
    #[test]
    fn should_pass_through_video_urls() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "url", "url": "https://example.com/video.mp4" },
                        "mediaType": "video/*",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "video_url",
                            "video_url": { "url": "https://example.com/video.mp4" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with audio/wav parts
    #[test]
    fn should_convert_messages_with_audio_wav_parts() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "Transcribe this audio" },
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "audio/wav",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Transcribe this audio" },
                        {
                            "type": "input_audio",
                            "input_audio": { "data": "AAECAw==", "format": "wav" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with audio/mp3 parts
    #[test]
    fn should_convert_messages_with_audio_mp3_parts() {
        let result = convert(&with_uint8_array(
            prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "data": { "type": "data", "data": "<Uint8Array>" },
                            "mediaType": "audio/mp3",
                        },
                    ],
                },
            ])),
            0,
            0,
            &[0, 1, 2, 3],
        ));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_audio",
                            "input_audio": { "data": "AAECAw==", "format": "mp3" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with audio/mpeg parts to mp3 format
    #[test]
    fn should_convert_messages_with_audio_mpeg_parts_to_mp3_format() {
        let result = convert(&with_uint8_array(
            prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "data": { "type": "data", "data": "<Uint8Array>" },
                            "mediaType": "audio/mpeg",
                        },
                    ],
                },
            ])),
            0,
            0,
            &[0, 1, 2, 3],
        ));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_audio",
                            "input_audio": { "data": "AAECAw==", "format": "mp3" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should throw error for audio parts with URLs
    #[test]
    fn should_throw_error_for_audio_parts_with_urls() {
        assert_unsupported(
            &prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "data": { "type": "url", "url": "https://example.com/audio.wav" },
                            "mediaType": "audio/wav",
                        },
                    ],
                },
            ])),
            "audio file parts with URLs",
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should throw error for unsupported audio format
    #[test]
    fn should_throw_error_for_unsupported_audio_format() {
        assert_unsupported(
            &with_uint8_array(
                prompt(json!([
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "file",
                                "data": { "type": "data", "data": "<Uint8Array>" },
                                "mediaType": "audio/ogg",
                            },
                        ],
                    },
                ])),
                0,
                0,
                &[0, 1, 2, 3],
            ),
            "audio media type audio/ogg",
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with PDF parts
    #[test]
    fn should_convert_messages_with_pdf_parts() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "Summarize this PDF" },
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "application/pdf",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Summarize this PDF" },
                        {
                            "type": "file",
                            "file": {
                                "filename": "document.pdf",
                                "file_data": "data:application/pdf;base64,AAECAw==",
                            },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with PDF parts using provided filename
    #[test]
    fn should_convert_messages_with_pdf_parts_using_provided_filename() {
        let result = convert(&with_uint8_array(
            prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "data": { "type": "data", "data": "<Uint8Array>" },
                            "mediaType": "application/pdf",
                            "filename": "report.pdf",
                        },
                    ],
                },
            ])),
            0,
            0,
            &[0, 1, 2, 3],
        ));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "file": {
                                "filename": "report.pdf",
                                "file_data": "data:application/pdf;base64,AAECAw==",
                            },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should throw error for PDF parts with URLs
    #[test]
    fn should_throw_error_for_pdf_parts_with_urls() {
        assert_unsupported(
            &prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "data": { "type": "url", "url": "https://example.com/document.pdf" },
                            "mediaType": "application/pdf",
                        },
                    ],
                },
            ])),
            "PDF file parts with URLs",
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with base64-encoded text/markdown parts
    #[test]
    fn should_convert_messages_with_base64_encoded_text_markdown_parts() {
        use base64::Engine;

        let markdown_text = "# Hello World\n\nThis is **markdown** content.";
        let base64_data = base64::engine::general_purpose::STANDARD.encode(markdown_text);

        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "Summarize this document" },
                    {
                        "type": "file",
                        "data": { "type": "data", "data": base64_data },
                        "mediaType": "text/markdown",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Summarize this document" },
                        {
                            "type": "text",
                            "text": markdown_text,
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert messages with text/plain parts from Uint8Array
    #[test]
    fn should_convert_messages_with_text_plain_parts_from_uint8_array() {
        let result = convert(&with_uint8_array(
            prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "data": { "type": "data", "data": "<Uint8Array>" },
                            "mediaType": "text/plain",
                        },
                    ],
                },
            ])),
            0,
            0,
            "Plain text content".as_bytes(),
        ));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Plain text content",
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should decode base64 string data for text/* file parts
    #[test]
    fn should_decode_base64_string_data_for_text_file_parts() {
        use base64::Engine;

        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": {
                            "type": "data",
                            "data": base64::engine::general_purpose::STANDARD
                                .encode("Plain text content"),
                        },
                        "mediaType": "text/plain",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Plain text content",
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should convert text file URL to string
    #[test]
    fn should_convert_text_file_url_to_string() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "url", "url": "https://example.com/readme.md" },
                        "mediaType": "text/markdown",
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "https://example.com/readme.md",
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should throw error for unsupported file types
    #[test]
    fn should_throw_error_for_unsupported_file_types() {
        assert_unsupported(
            &with_uint8_array(
                prompt(json!([
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "file",
                                "data": { "type": "data", "data": "<Uint8Array>" },
                                "mediaType": "application/zip",
                            },
                        ],
                    },
                ])),
                0,
                0,
                &[0, 1, 2, 3],
            ),
            "file part media type application/zip",
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › user messages › should throw error for file parts with provider references
    #[test]
    fn should_throw_error_for_file_parts_with_provider_references() {
        assert_unsupported(
            &prompt(json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "file",
                            "data": {
                                "type": "reference",
                                "reference": { "openaiCompatible": "file-123" },
                            },
                            "mediaType": "image/png",
                        },
                    ],
                },
            ])),
            "file parts with provider references",
        );
    }
}

mod tool_calls {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › tool calls › should stringify arguments to tool calls
    #[test]
    fn should_stringify_arguments_to_tool_calls() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "input": { "foo": "bar123" },
                        "toolCallId": "quux",
                        "toolName": "thwomp",
                    },
                ],
            },
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "quux",
                        "toolName": "thwomp",
                        "output": { "type": "json", "value": { "oof": "321rab" } },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "quux",
                            "function": {
                                "name": "thwomp",
                                "arguments": stringify(&json!({ "foo": "bar123" })),
                            },
                        },
                    ],
                },
                {
                    "role": "tool",
                    "content": stringify(&json!({ "oof": "321rab" })),
                    "tool_call_id": "quux",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › tool calls › should send empty string content for assistant messages with no tool calls
    #[test]
    fn should_send_empty_string_content_for_assistant_messages_with_no_tool_calls() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [{ "type": "text", "text": "" }],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "content": "",
                    "role": "assistant",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › tool calls › should handle text output type in tool results
    #[test]
    fn should_handle_text_output_type_in_tool_results() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "input": { "query": "weather" },
                        "toolCallId": "call-1",
                        "toolName": "getWeather",
                    },
                ],
            },
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "call-1",
                        "toolName": "getWeather",
                        "output": { "type": "text", "value": "It is sunny today" },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "call-1",
                            "function": {
                                "name": "getWeather",
                                "arguments": stringify(&json!({ "query": "weather" })),
                            },
                        },
                    ],
                },
                {
                    "role": "tool",
                    "content": "It is sunny today",
                    "tool_call_id": "call-1",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › tool calls › should stringify multi-part tool content by default
    #[test]
    fn should_stringify_multi_part_tool_content_by_default() {
        let value = json!([
            { "type": "text", "text": "image result" },
            {
                "type": "file",
                "data": { "type": "data", "data": "iVBORw0KGgo=" },
                "mediaType": "image/png",
            },
        ]);

        let result = convert(&prompt(json!([
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "call-1",
                        "toolName": "useImage",
                        "output": { "type": "content", "value": value },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "tool",
                    "tool_call_id": "call-1",
                    "content": stringify(&value),
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › tool calls › should convert multi-part tool content when supported
    #[test]
    fn should_convert_multi_part_tool_content_when_supported() {
        let result = convert_with(
            &prompt(json!([
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "call-1",
                            "toolName": "useImage",
                            "output": {
                                "type": "content",
                                "value": [
                                    { "type": "text", "text": "image result" },
                                    {
                                        "type": "file",
                                        "data": { "type": "data", "data": "iVBORw0KGgo=" },
                                        "mediaType": "image/png",
                                    },
                                    {
                                        "type": "file",
                                        "data": {
                                            "type": "url",
                                            "url": "https://example.com/image.png",
                                        },
                                        "mediaType": "image/png",
                                    },
                                ],
                            },
                        },
                    ],
                },
            ])),
            "google",
            true,
        )
        .expect("conversion succeeds");

        assert_eq!(
            result,
            json!([
                {
                    "role": "tool",
                    "tool_call_id": "call-1",
                    "content": [
                        { "type": "text", "text": "image result" },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": "data:image/png;base64,iVBORw0KGgo=",
                            },
                        },
                        {
                            "type": "image_url",
                            "image_url": { "url": "https://example.com/image.png" },
                        },
                    ],
                },
            ])
        );
    }
}

mod provider_specific_metadata_merging {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should merge system message metadata
    #[test]
    fn should_merge_system_message_metadata() {
        let result = convert(&prompt(json!([
            {
                "role": "system",
                "content": "You are a helpful assistant.",
                "providerOptions": {
                    "openaiCompatible": {
                        "cacheControl": { "type": "ephemeral" },
                    },
                },
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "system",
                    "content": "You are a helpful assistant.",
                    "cacheControl": { "type": "ephemeral" },
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should merge user message content metadata
    #[test]
    fn should_merge_user_message_content_metadata() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "Hello",
                        "providerOptions": {
                            "openaiCompatible": {
                                "cacheControl": { "type": "ephemeral" },
                            },
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": "Hello",
                    "cacheControl": { "type": "ephemeral" },
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should prioritize content-level metadata when merging
    #[test]
    fn should_prioritize_content_level_metadata_when_merging() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "providerOptions": {
                    "openaiCompatible": {
                        "messageLevel": true,
                    },
                },
                "content": [
                    {
                        "type": "text",
                        "text": "Hello",
                        "providerOptions": {
                            "openaiCompatible": {
                                "contentLevel": true,
                            },
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": "Hello",
                    "contentLevel": true,
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle tool calls with metadata
    #[test]
    fn should_handle_tool_calls_with_metadata() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "calculator",
                        "input": { "x": 1, "y": 2 },
                        "providerOptions": {
                            "openaiCompatible": {
                                "cacheControl": { "type": "ephemeral" },
                            },
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call1",
                            "type": "function",
                            "function": {
                                "name": "calculator",
                                "arguments": stringify(&json!({ "x": 1, "y": 2 })),
                            },
                            "cacheControl": { "type": "ephemeral" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle image content with metadata
    #[test]
    fn should_handle_image_content_with_metadata() {
        let image_url = "https://example.com/image.jpg";
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "url", "url": image_url },
                        "mediaType": "image/*",
                        "providerOptions": {
                            "openaiCompatible": {
                                "cacheControl": { "type": "ephemeral" },
                            },
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "image_url",
                            "image_url": { "url": image_url },
                            "cacheControl": { "type": "ephemeral" },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should omit non-openaiCompatible metadata
    #[test]
    fn should_omit_non_openai_compatible_metadata() {
        let result = convert(&prompt(json!([
            {
                "role": "system",
                "content": "Hello",
                "providerOptions": {
                    "someOtherProvider": {
                        "shouldBeIgnored": true,
                    },
                },
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "system",
                    "content": "Hello",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle a user message with multiple content parts (text + image)
    #[test]
    fn should_handle_a_user_message_with_multiple_content_parts_text_image() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "Hello from part 1",
                        "providerOptions": {
                            "openaiCompatible": { "sentiment": "positive" },
                            "leftoverKey": { "foo": "some leftover data" },
                        },
                    },
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                        "providerOptions": {
                            "openaiCompatible": { "alt_text": "A sample image" },
                        },
                    },
                ],
                "providerOptions": {
                    "openaiCompatible": { "priority": "high" },
                },
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "priority": "high", // hoisted from message-level providerOptions
                    "content": [
                        {
                            "type": "text",
                            "text": "Hello from part 1",
                            "sentiment": "positive", // hoisted from part-level openaiCompatible
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": "data:image/png;base64,AAECAw==",
                            },
                            "alt_text": "A sample image",
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle a user message with multiple text parts (flattening disabled)
    #[test]
    fn should_handle_a_user_message_with_multiple_text_parts_flattening_disabled() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "Part 1" },
                    { "type": "text", "text": "Part 2" },
                ],
            },
        ])));

        // Because there are multiple text parts, the converter won't flatten them
        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Part 1" },
                        { "type": "text", "text": "Part 2" },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle an assistant message with text plus multiple tool calls
    #[test]
    fn should_handle_an_assistant_message_with_text_plus_multiple_tool_calls() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "Checking that now..." },
                    {
                        "type": "tool-call",
                        "toolCallId": "call1",
                        "toolName": "searchTool",
                        "input": { "query": "Weather" },
                        "providerOptions": {
                            "openaiCompatible": { "function_call_reason": "user request" },
                        },
                    },
                    { "type": "text", "text": "Almost there..." },
                    {
                        "type": "tool-call",
                        "toolCallId": "call2",
                        "toolName": "mapsTool",
                        "input": { "location": "Paris" },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "content": "Checking that now...Almost there...",
                    "tool_calls": [
                        {
                            "id": "call1",
                            "type": "function",
                            "function": {
                                "name": "searchTool",
                                "arguments": stringify(&json!({ "query": "Weather" })),
                            },
                            "function_call_reason": "user request",
                        },
                        {
                            "id": "call2",
                            "type": "function",
                            "function": {
                                "name": "mapsTool",
                                "arguments": stringify(&json!({ "location": "Paris" })),
                            },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle a single tool role message with multiple tool-result parts
    #[test]
    fn should_handle_a_single_tool_role_message_with_multiple_tool_result_parts() {
        let result = convert(&prompt(json!([
            {
                "role": "tool",
                "providerOptions": {
                    // this just gets omitted as we prioritize content-level metadata
                    "openaiCompatible": { "responseTier": "detailed" },
                },
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "call123",
                        "toolName": "calculator",
                        "output": { "type": "json", "value": { "stepOne": "data chunk 1" } },
                    },
                    {
                        "type": "tool-result",
                        "toolCallId": "call123",
                        "toolName": "calculator",
                        "providerOptions": {
                            "openaiCompatible": { "partial": true },
                        },
                        "output": { "type": "json", "value": { "stepTwo": "data chunk 2" } },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "tool",
                    "tool_call_id": "call123",
                    "content": stringify(&json!({ "stepOne": "data chunk 1" })),
                },
                {
                    "role": "tool",
                    "tool_call_id": "call123",
                    "content": stringify(&json!({ "stepTwo": "data chunk 2" })),
                    "partial": true,
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle multiple content parts with multiple metadata layers
    #[test]
    fn should_handle_multiple_content_parts_with_multiple_metadata_layers() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "providerOptions": {
                    "openaiCompatible": { "messageLevel": "global-metadata" },
                    "leftoverForMessage": { "x": 123 },
                },
                "content": [
                    {
                        "type": "text",
                        "text": "Part A",
                        "providerOptions": {
                            "openaiCompatible": { "textPartLevel": "localized" },
                            "leftoverForText": { "info": "text leftover" },
                        },
                    },
                    {
                        "type": "file",
                        // Buffer.from([9, 8, 7, 6]).toString('base64')
                        "data": { "type": "data", "data": "CQgHBg==" },
                        "mediaType": "image/png",
                        "providerOptions": {
                            "openaiCompatible": { "imagePartLevel": "image-data" },
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "user",
                    "messageLevel": "global-metadata",
                    "content": [
                        {
                            "type": "text",
                            "text": "Part A",
                            "textPartLevel": "localized",
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": "data:image/png;base64,CQgHBg==",
                            },
                            "imagePartLevel": "image-data",
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle different tool metadata vs. message-level metadata
    #[test]
    fn should_handle_different_tool_metadata_vs_message_level_metadata() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "providerOptions": {
                    "openaiCompatible": { "globalPriority": "high" },
                },
                "content": [
                    { "type": "text", "text": "Initiating tool calls..." },
                    {
                        "type": "tool-call",
                        "toolCallId": "callXYZ",
                        "toolName": "awesomeTool",
                        "input": { "param": "someValue" },
                        "providerOptions": {
                            "openaiCompatible": {
                                "toolPriority": "critical",
                            },
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "globalPriority": "high",
                    "content": "Initiating tool calls...",
                    "tool_calls": [
                        {
                            "id": "callXYZ",
                            "type": "function",
                            "function": {
                                "name": "awesomeTool",
                                "arguments": stringify(&json!({ "param": "someValue" })),
                            },
                            "toolPriority": "critical",
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › provider-specific metadata merging › should handle metadata collisions and overwrites in tool calls
    #[test]
    fn should_handle_metadata_collisions_and_overwrites_in_tool_calls() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "providerOptions": {
                    "openaiCompatible": {
                        "cacheControl": { "type": "default" },
                        "sharedKey": "assistantLevel",
                    },
                },
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "collisionToolCall",
                        "toolName": "collider",
                        "input": { "num": 42 },
                        "providerOptions": {
                            "openaiCompatible": {
                                "cacheControl": { "type": "ephemeral" }, // overwrites top-level
                                "sharedKey": "toolLevel",
                            },
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "cacheControl": { "type": "default" },
                    "sharedKey": "assistantLevel",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "collisionToolCall",
                            "type": "function",
                            "function": {
                                "name": "collider",
                                "arguments": stringify(&json!({ "num": 42 })),
                            },
                            "cacheControl": { "type": "ephemeral" },
                            "sharedKey": "toolLevel",
                        },
                    ],
                },
            ])
        );
    }
}

mod google_gemini_thought_signatures_open_ai_compatibility {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › Google Gemini thought signatures (OpenAI compatibility) › should serialize thought signature from a custom provider namespace
    #[test]
    fn should_serialize_thought_signature_from_a_custom_provider_namespace() {
        let result = convert_with(
            &prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "function-call-1",
                            "toolName": "check_flight",
                            "input": { "flight": "AA100" },
                            "providerOptions": {
                                "myGateway": {
                                    "thoughtSignature": "<Custom Signature>",
                                },
                            },
                        },
                    ],
                },
            ])),
            "myGateway",
            false,
        )
        .expect("conversion succeeds");

        assert_match_object(
            &result[0],
            &json!({
                "tool_calls": [
                    {
                        "extra_content": {
                            "google": { "thought_signature": "<Custom Signature>" },
                        },
                    },
                ],
            }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › Google Gemini thought signatures (OpenAI compatibility) › should fall back to the google namespace
    #[test]
    fn should_fall_back_to_the_google_namespace() {
        let result = convert_with(
            &prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "function-call-1",
                            "toolName": "check_flight",
                            "input": { "flight": "AA100" },
                            "providerOptions": {
                                "google": {
                                    "thoughtSignature": "<Google Signature>",
                                },
                            },
                        },
                    ],
                },
            ])),
            "myGateway",
            false,
        )
        .expect("conversion succeeds");

        assert_match_object(
            &result[0],
            &json!({
                "tool_calls": [
                    {
                        "extra_content": {
                            "google": { "thought_signature": "<Google Signature>" },
                        },
                    },
                ],
            }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › Google Gemini thought signatures (OpenAI compatibility) › should prefer the custom provider namespace over google
    #[test]
    fn should_prefer_the_custom_provider_namespace_over_google() {
        let result = convert_with(
            &prompt(json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "function-call-1",
                            "toolName": "check_flight",
                            "input": { "flight": "AA100" },
                            "providerOptions": {
                                "myGateway": {
                                    "thoughtSignature": "<Custom Signature>",
                                },
                                "google": {
                                    "thoughtSignature": "<Google Signature>",
                                },
                            },
                        },
                    ],
                },
            ])),
            "myGateway",
            false,
        )
        .expect("conversion succeeds");

        assert_match_object(
            &result[0],
            &json!({
                "tool_calls": [
                    {
                        "extra_content": {
                            "google": { "thought_signature": "<Custom Signature>" },
                        },
                    },
                ],
            }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › Google Gemini thought signatures (OpenAI compatibility) › should serialize thought signature to extra_content for single tool call
    #[test]
    fn should_serialize_thought_signature_to_extra_content_for_single_tool_call() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "function-call-1",
                        "toolName": "check_flight",
                        "input": { "flight": "AA100" },
                        "providerOptions": {
                            "google": {
                                "thoughtSignature": "<Signature A>",
                            },
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "function-call-1",
                            "type": "function",
                            "function": {
                                "name": "check_flight",
                                "arguments": stringify(&json!({ "flight": "AA100" })),
                            },
                            "extra_content": {
                                "google": {
                                    "thought_signature": "<Signature A>",
                                },
                            },
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › Google Gemini thought signatures (OpenAI compatibility) › should handle sequential tool calls with separate signatures (Turn 1 Step 3 scenario)
    #[test]
    fn should_handle_sequential_tool_calls_with_separate_signatures() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "Check flight status for AA100 and book a taxi 2 hours before if delayed.",
                    },
                ],
            },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "function-call-1",
                        "toolName": "check_flight",
                        "input": { "flight": "AA100" },
                        "providerOptions": {
                            "google": {
                                "thoughtSignature": "<Signature A>",
                            },
                        },
                    },
                ],
            },
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "function-call-1",
                        "toolName": "check_flight",
                        "output": {
                            "type": "json",
                            "value": { "status": "delayed", "departure_time": "12 PM" },
                        },
                    },
                ],
            },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "function-call-2",
                        "toolName": "book_taxi",
                        "input": { "time": "10 AM" },
                        "providerOptions": {
                            "google": {
                                "thoughtSignature": "<Signature B>",
                            },
                        },
                    },
                ],
            },
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "function-call-2",
                        "toolName": "book_taxi",
                        "output": { "type": "json", "value": { "booking_status": "success" } },
                    },
                ],
            },
        ])));

        // Verify both signatures are preserved
        assert_eq!(
            result[1],
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "function-call-1",
                        "type": "function",
                        "function": {
                            "name": "check_flight",
                            "arguments": stringify(&json!({ "flight": "AA100" })),
                        },
                        "extra_content": {
                            "google": {
                                "thought_signature": "<Signature A>",
                            },
                        },
                    },
                ],
            })
        );

        assert_eq!(
            result[3],
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "function-call-2",
                        "type": "function",
                        "function": {
                            "name": "book_taxi",
                            "arguments": stringify(&json!({ "time": "10 AM" })),
                        },
                        "extra_content": {
                            "google": {
                                "thought_signature": "<Signature B>",
                            },
                        },
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › Google Gemini thought signatures (OpenAI compatibility) › should handle parallel tool calls with signature only on first call
    #[test]
    fn should_handle_parallel_tool_calls_with_signature_only_on_first_call() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "function-call-paris",
                        "toolName": "get_current_temperature",
                        "input": { "location": "Paris" },
                        "providerOptions": {
                            "google": {
                                "thoughtSignature": "<Signature A>",
                            },
                        },
                    },
                    {
                        "type": "tool-call",
                        "toolCallId": "function-call-london",
                        "toolName": "get_current_temperature",
                        "input": { "location": "London" },
                        // No signature on parallel function call
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "function-call-paris",
                            "type": "function",
                            "function": {
                                "name": "get_current_temperature",
                                "arguments": stringify(&json!({ "location": "Paris" })),
                            },
                            "extra_content": {
                                "google": {
                                    "thought_signature": "<Signature A>",
                                },
                            },
                        },
                        {
                            "id": "function-call-london",
                            "type": "function",
                            "function": {
                                "name": "get_current_temperature",
                                "arguments": stringify(&json!({ "location": "London" })),
                            },
                            // No extra_content for second parallel call
                        },
                    ],
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › Google Gemini thought signatures (OpenAI compatibility) › should not include extra_content when no thought signature is present
    #[test]
    fn should_not_include_extra_content_when_no_thought_signature_is_present() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call-1",
                        "toolName": "some_tool",
                        "input": { "param": "value" },
                    },
                ],
            },
        ])));

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call-1",
                            "type": "function",
                            "function": {
                                "name": "some_tool",
                                "arguments": stringify(&json!({ "param": "value" })),
                            },
                        },
                    ],
                },
            ])
        );
    }
}

mod top_level_only_media_type_resolution {
    use super::*;

    const PNG_BASE64: &str = "iVBORw0KGgo=";

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › top-level-only media type resolution › passes full image/png through unchanged for inline data
    #[test]
    fn passes_full_image_png_through_unchanged_for_inline_data() {
        let result = convert(&prompt(json!([
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
        ])));

        assert_eq!(
            result[0]["content"][0],
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
    #[test]
    fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
        let result = convert(&prompt(json!([
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
        ])));

        assert_eq!(
            result[0]["content"][0],
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › top-level-only media type resolution › passes through URL source for top-level-only image (provider accepts raw URL)
    #[test]
    fn passes_through_url_source_for_top_level_only_image() {
        let result = convert(&prompt(json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "mediaType": "image",
                        "data": {
                            "type": "url",
                            "url": "https://example.com/x.png",
                        },
                    },
                ],
            },
        ])));

        assert_eq!(
            result[0]["content"][0],
            json!({
                "type": "image_url",
                "image_url": { "url": "https://example.com/x.png" },
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/convert-to-openai-compatible-chat-messages.test.ts › top-level-only media type resolution › normalizes image/* wildcard via detection
    #[test]
    fn normalizes_image_wildcard_via_detection() {
        let result = convert(&prompt(json!([
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
        ])));

        assert_eq!(
            result[0]["content"][0],
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
            })
        );
    }
}
