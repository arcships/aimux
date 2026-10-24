use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    FilePart, LanguageModelMessage, LanguageModelPrompt, UserPart,
};
use aimux_core::shared::{FileBytes, FileData};
use serde_json::{Value, json};

use crate::openai::chat::convert_to_chat_messages::convert_prompt_to_openai_messages_with_mode_fallible;
use crate::openai::language_model_capabilities::SystemMessageMode;

/// The upstream prompt literal (AI SDK V4 JSON shape) as an aimux prompt.
fn prompt(value: Value) -> LanguageModelPrompt {
    serde_json::from_value(value).expect("valid prompt")
}

/// `convertToOpenAIChatMessages({ prompt, systemMessageMode })` with
/// upstream's default mode `system`; returns `result.messages`.
fn convert_with(
    prompt: &LanguageModelPrompt,
    mode: SystemMessageMode,
) -> Result<Value, AiMuxError> {
    convert_prompt_to_openai_messages_with_mode_fallible(prompt, mode).map(Value::Array)
}

fn convert(prompt: &LanguageModelPrompt) -> Result<Value, AiMuxError> {
    convert_with(prompt, SystemMessageMode::System)
}

/// A user message with one binary (`Uint8Array`) file part.
fn binary_file_prompt(
    media_type: &str,
    bytes: &[u8],
    filename: Option<&str>,
) -> LanguageModelPrompt {
    vec![LanguageModelMessage::User {
        content: vec![UserPart::File(FilePart {
            data: FileData::Data {
                data: FileBytes::Binary(bytes.to_vec()),
            },
            media_type: media_type.to_string(),
            filename: filename.map(str::to_string),
            provider_options: None,
        })],
        provider_options: None,
    }]
}

/// `expect(() => ...).toThrow(text)`: the error message contains `text`.
fn assert_throws(result: Result<Value, AiMuxError>, text: &str) {
    let error = result.expect_err("conversion must fail");
    assert!(error.to_string().contains(text), "{error}");
}

fn single_file_prompt(part: Value) -> LanguageModelPrompt {
    prompt(json!([{ "role": "user", "content": [part] }]))
}

mod system_messages {
    use super::*;

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › system messages › should forward system messages
    #[test]
    fn should_forward_system_messages() {
        let result = convert(&prompt(json!([
            { "role": "system", "content": "You are a helpful assistant." }
        ])))
        .unwrap();

        assert_eq!(
            result,
            json!([{ "role": "system", "content": "You are a helpful assistant." }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › system messages › should convert system messages to developer messages when requested
    #[test]
    fn should_convert_system_messages_to_developer_messages_when_requested() {
        let result = convert_with(
            &prompt(json!([
                { "role": "system", "content": "You are a helpful assistant." }
            ])),
            SystemMessageMode::Developer,
        )
        .unwrap();

        assert_eq!(
            result,
            json!([{ "role": "developer", "content": "You are a helpful assistant." }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › system messages › should add a prompt cache breakpoint to a system message
    #[test]
    fn should_add_a_prompt_cache_breakpoint_to_a_system_message() {
        let result = convert(&prompt(json!([{
            "role": "system",
            "content": "You are a helpful assistant.",
            "providerOptions": {
                "openai": { "promptCacheBreakpoint": { "mode": "explicit" } },
            },
        }])))
        .unwrap();

        assert_eq!(
            result,
            json!([{
                "role": "system",
                "content": [{
                    "type": "text",
                    "text": "You are a helpful assistant.",
                    "prompt_cache_breakpoint": { "mode": "explicit" },
                }],
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › system messages › should remove system messages when requested
    #[test]
    fn should_remove_system_messages_when_requested() {
        let result = convert_with(
            &prompt(json!([
                { "role": "system", "content": "You are a helpful assistant." }
            ])),
            SystemMessageMode::Remove,
        )
        .unwrap();

        assert_eq!(result, json!([]));
    }
}

mod user_messages {
    use super::*;

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › should convert messages with only a text part to a string content
    #[test]
    fn should_convert_messages_with_only_a_text_part_to_a_string_content() {
        let result = convert(&prompt(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Hello" }] }
        ])))
        .unwrap();

        assert_eq!(result, json!([{ "role": "user", "content": "Hello" }]));
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › should add prompt cache breakpoints to supported content blocks
    #[test]
    fn should_add_prompt_cache_breakpoints_to_supported_content_blocks() {
        let breakpoint = json!({ "mode": "explicit" });
        let options = json!({ "openai": { "promptCacheBreakpoint": breakpoint } });
        let result = convert(&prompt(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "Hello", "providerOptions": options },
                {
                    "type": "file",
                    "mediaType": "image/png",
                    "data": { "type": "url", "url": "https://example.com/image.png" },
                    "providerOptions": options,
                },
                {
                    "type": "file",
                    "mediaType": "audio/wav",
                    "data": { "type": "data", "data": "AAECAw==" },
                    "providerOptions": options,
                },
                {
                    "type": "file",
                    "mediaType": "application/pdf",
                    "data": { "type": "reference", "reference": { "openai": "file-pdf-123" } },
                    "providerOptions": options,
                },
            ],
        }])))
        .unwrap();

        assert_eq!(
            result,
            json!([{
                "role": "user",
                "content": [
                    { "type": "text", "text": "Hello", "prompt_cache_breakpoint": breakpoint },
                    {
                        "type": "image_url",
                        "image_url": { "url": "https://example.com/image.png" },
                        "prompt_cache_breakpoint": breakpoint,
                    },
                    {
                        "type": "input_audio",
                        "input_audio": { "data": "AAECAw==", "format": "wav" },
                        "prompt_cache_breakpoint": breakpoint,
                    },
                    {
                        "type": "file",
                        "file": { "file_id": "file-pdf-123" },
                        "prompt_cache_breakpoint": breakpoint,
                    },
                ],
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › should convert messages with image parts
    #[test]
    fn should_convert_messages_with_image_parts() {
        let result = convert(&prompt(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "Hello" },
                {
                    "type": "file",
                    "mediaType": "image/png",
                    "data": { "type": "data", "data": "AAECAw==" },
                },
            ],
        }])))
        .unwrap();

        assert_eq!(
            result,
            json!([{
                "role": "user",
                "content": [
                    { "type": "text", "text": "Hello" },
                    {
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,AAECAw==" },
                    },
                ],
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › should convert messages with Uint8Array image parts to data URLs
    #[test]
    fn should_convert_messages_with_uint8_array_image_parts_to_data_urls() {
        let result = convert(&binary_file_prompt(
            "image/jpeg",
            &[0xff, 0xd8, 0xff, 0xe0],
            None,
        ))
        .unwrap();

        assert_eq!(
            result,
            json!([{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": { "url": "data:image/jpeg;base64,/9j/4A==" },
                }],
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › should add image detail when specified through extension
    #[test]
    fn should_add_image_detail_when_specified_through_extension() {
        let result = convert(&single_file_prompt(json!({
            "type": "file",
            "mediaType": "image/png",
            "data": { "type": "data", "data": "AAECAw==" },
            "providerOptions": { "openai": { "imageDetail": "low" } },
        })))
        .unwrap();

        assert_eq!(
            result,
            json!([{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": { "url": "data:image/png;base64,AAECAw==", "detail": "low" },
                }],
            }])
        );
    }

    mod file_parts {
        use super::*;

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should throw for unsupported mime types
        #[test]
        fn should_throw_for_unsupported_mime_types() {
            assert_throws(
                convert(&single_file_prompt(json!({
                    "type": "file",
                    "data": { "type": "data", "data": "AAECAw==" },
                    "mediaType": "application/something",
                }))),
                "file part media type application/something",
            );
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should throw for URL data
        #[test]
        fn should_throw_for_url_data() {
            assert_throws(
                convert(&single_file_prompt(json!({
                    "type": "file",
                    "data": { "type": "url", "url": "https://example.com/foo.wav" },
                    "mediaType": "audio/wav",
                }))),
                "audio file parts with URLs",
            );
        }

        fn audio(media_type: &str, format: &str) {
            let result = convert(&single_file_prompt(json!({
                "type": "file",
                "data": { "type": "data", "data": "AAECAw==" },
                "mediaType": media_type,
            })))
            .unwrap();

            assert_eq!(
                result,
                json!([{
                    "role": "user",
                    "content": [{
                        "type": "input_audio",
                        "input_audio": { "data": "AAECAw==", "format": format },
                    }],
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should add audio content for audio/wav file parts
        #[test]
        fn should_add_audio_content_for_audio_wav_file_parts() {
            audio("audio/wav", "wav");
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should add audio content for audio/mpeg file parts
        #[test]
        fn should_add_audio_content_for_audio_mpeg_file_parts() {
            audio("audio/mpeg", "mp3");
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should add audio content for audio/mp3 file parts
        #[test]
        fn should_add_audio_content_for_audio_mp3_file_parts() {
            audio("audio/mp3", "mp3");
        }

        fn pdf_result() -> Value {
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "file": {
                        "filename": "document.pdf",
                        "file_data": "data:application/pdf;base64,AQIDBAU=",
                    },
                }],
            }])
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should convert messages with PDF file parts
        #[test]
        fn should_convert_messages_with_pdf_file_parts() {
            let result = convert(&single_file_prompt(json!({
                "type": "file",
                "mediaType": "application/pdf",
                "data": { "type": "data", "data": "AQIDBAU=" },
                "filename": "document.pdf",
            })))
            .unwrap();

            assert_eq!(result, pdf_result());
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should convert messages with binary PDF file parts
        #[test]
        fn should_convert_messages_with_binary_pdf_file_parts() {
            let result = convert(&binary_file_prompt(
                "application/pdf",
                &[1, 2, 3, 4, 5],
                Some("document.pdf"),
            ))
            .unwrap();

            assert_eq!(result, pdf_result());
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should convert messages with PDF file parts using provider reference
        #[test]
        fn should_convert_messages_with_pdf_file_parts_using_provider_reference() {
            let result = convert(&single_file_prompt(json!({
                "type": "file",
                "mediaType": "application/pdf",
                "data": { "type": "reference", "reference": { "openai": "file-pdf-12345" } },
            })))
            .unwrap();

            assert_eq!(
                result,
                json!([{
                    "role": "user",
                    "content": [{ "type": "file", "file": { "file_id": "file-pdf-12345" } }],
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should convert messages with image parts using provider reference
        #[test]
        fn should_convert_messages_with_image_parts_using_provider_reference() {
            let result = convert(&single_file_prompt(json!({
                "type": "file",
                "mediaType": "image/png",
                "data": { "type": "reference", "reference": { "openai": "file-img-12345" } },
            })))
            .unwrap();

            assert_eq!(
                result,
                json!([{
                    "role": "user",
                    "content": [{ "type": "file", "file": { "file_id": "file-img-12345" } }],
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should throw when provider reference does not contain openai
        #[test]
        fn should_throw_when_provider_reference_does_not_contain_openai() {
            assert_throws(
                convert(&single_file_prompt(json!({
                    "type": "file",
                    "mediaType": "application/pdf",
                    "data": { "type": "reference", "reference": { "anthropic": "file-xyz" } },
                }))),
                "No provider reference found for provider 'openai'. Available providers: anthropic",
            );
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should use default filename for PDF file parts when not provided
        #[test]
        fn should_use_default_filename_for_pdf_file_parts_when_not_provided() {
            let result = convert(&single_file_prompt(json!({
                "type": "file",
                "mediaType": "application/pdf",
                "data": { "type": "data", "data": "AQIDBAU=" },
            })))
            .unwrap();

            assert_eq!(
                result,
                json!([{
                    "role": "user",
                    "content": [{
                        "type": "file",
                        "file": {
                            "filename": "part-0.pdf",
                            "file_data": "data:application/pdf;base64,AQIDBAU=",
                        },
                    }],
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should throw error for unsupported file types
        #[test]
        fn should_throw_error_for_unsupported_file_types() {
            assert_throws(
                convert(&single_file_prompt(json!({
                    "type": "file",
                    "mediaType": "text/plain",
                    "data": { "type": "data", "data": "AQIDBAU=" },
                }))),
                "file part media type text/plain",
            );
        }

        /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › should throw error for file URLs
        #[test]
        fn should_throw_error_for_file_urls() {
            assert_throws(
                convert(&single_file_prompt(json!({
                    "type": "file",
                    "mediaType": "application/pdf",
                    "data": { "type": "url", "url": "https://example.com/document.pdf" },
                }))),
                "PDF file parts with URLs",
            );
        }

        mod top_level_only_media_type_resolution {
            use super::*;

            const PNG_BASE64: &str = "iVBORw0KGgo=";

            fn first_content(media_type: &str, data: Value) -> Value {
                let result = convert(&single_file_prompt(json!({
                    "type": "file",
                    "mediaType": media_type,
                    "data": data,
                })))
                .unwrap();
                result[0]["content"][0].clone()
            }

            /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
            #[test]
            fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
                assert_eq!(
                    first_content("image", json!({ "type": "data", "data": PNG_BASE64 })),
                    json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
                    })
                );
            }

            /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › top-level-only media type resolution › normalizes image/* wildcard via detection
            #[test]
            fn normalizes_image_wildcard_via_detection() {
                assert_eq!(
                    first_content("image/*", json!({ "type": "data", "data": PNG_BASE64 })),
                    json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
                    })
                );
            }

            /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › top-level-only media type resolution › passes through URL source for top-level-only image (provider accepts raw URL)
            #[test]
            fn passes_through_url_source_for_top_level_only_image() {
                assert_eq!(
                    first_content(
                        "image",
                        json!({ "type": "url", "url": "https://example.com/x.png" })
                    ),
                    json!({
                        "type": "image_url",
                        "image_url": { "url": "https://example.com/x.png" },
                    })
                );
            }

            /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › top-level-only media type resolution › throws for top-level-only application (PDF requires full resolution) with URL source
            #[test]
            fn throws_for_top_level_only_application_with_url_source() {
                let error = convert(&single_file_prompt(json!({
                    "type": "file",
                    "mediaType": "application",
                    "data": { "type": "url", "url": "https://example.com/x.pdf" },
                })))
                .expect_err("conversion must fail");
                // toThrow(/media type "application".*not passed as inline bytes/)
                let message = error.to_string();
                let at = message
                    .find("media type \"application\"")
                    .unwrap_or_else(|| panic!("{message}"));
                assert!(
                    message[at..].contains("not passed as inline bytes"),
                    "{message}"
                );
            }

            /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › user messages › file parts › top-level-only media type resolution › preserves full image/png pass-through
            #[test]
            fn preserves_full_image_png_pass_through() {
                assert_eq!(
                    first_content("image/png", json!({ "type": "data", "data": PNG_BASE64 })),
                    json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:image/png;base64,{PNG_BASE64}") },
                    })
                );
            }
        }
    }
}

mod tool_calls {
    use super::*;

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › tool calls › should add a prompt cache breakpoint to assistant text content
    #[test]
    fn should_add_a_prompt_cache_breakpoint_to_assistant_text_content() {
        let result = convert(&prompt(json!([{
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Cached assistant content",
                "providerOptions": {
                    "openai": { "promptCacheBreakpoint": { "mode": "explicit" } },
                },
            }],
        }])))
        .unwrap();

        assert_eq!(
            result,
            json!([{
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "Cached assistant content",
                    "prompt_cache_breakpoint": { "mode": "explicit" },
                }],
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › tool calls › should add a prompt cache breakpoint to tool text content
    #[test]
    fn should_add_a_prompt_cache_breakpoint_to_tool_text_content() {
        let result = convert(&prompt(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "cached-tool",
                "toolName": "cached-tool",
                "output": {
                    "type": "text",
                    "value": "Cached tool content",
                    "providerOptions": {
                        "openai": { "promptCacheBreakpoint": { "mode": "explicit" } },
                    },
                },
            }],
        }])))
        .unwrap();

        assert_eq!(
            result,
            json!([{
                "role": "tool",
                "content": [{
                    "type": "text",
                    "text": "Cached tool content",
                    "prompt_cache_breakpoint": { "mode": "explicit" },
                }],
                "tool_call_id": "cached-tool",
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › tool calls › should stringify arguments to tool calls
    #[test]
    fn should_stringify_arguments_to_tool_calls() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [{
                    "type": "tool-call",
                    "input": { "foo": "bar123" },
                    "toolCallId": "quux",
                    "toolName": "thwomp",
                }],
            },
            {
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "quux",
                    "toolName": "thwomp",
                    "output": { "type": "json", "value": { "oof": "321rab" } },
                }],
            },
        ])))
        .unwrap();

        assert_eq!(
            result,
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "type": "function",
                        "id": "quux",
                        "function": {
                            "name": "thwomp",
                            "arguments": json!({ "foo": "bar123" }).to_string(),
                        },
                    }],
                },
                {
                    "role": "tool",
                    "content": json!({ "oof": "321rab" }).to_string(),
                    "tool_call_id": "quux",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › tool calls › should send empty string content for assistant messages with no tool calls
    #[test]
    fn should_send_empty_string_content_for_assistant_messages_with_no_tool_calls() {
        let result = convert(&prompt(json!([
            { "role": "assistant", "content": [{ "type": "text", "text": "" }] }
        ])))
        .unwrap();

        assert_eq!(result, json!([{ "content": "", "role": "assistant" }]));
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › tool calls › should default missing tool call input to an empty object
    #[test]
    fn should_default_missing_tool_call_input_to_an_empty_object() {
        // `input: undefined` is `null` in the Rust prompt (`input` is a `Value`).
        let result = convert(&prompt(json!([{
            "role": "assistant",
            "content": [{
                "type": "tool-call",
                "toolCallId": "quux",
                "toolName": "thwomp",
                "input": null,
            }],
        }])))
        .unwrap();

        assert_eq!(
            result,
            json!([{
                "content": null,
                "role": "assistant",
                "tool_calls": [{
                    "function": { "arguments": "{}", "name": "thwomp" },
                    "id": "quux",
                    "type": "function",
                }],
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › tool calls › should normalize malformed tool call input and preserve the tool error
    #[test]
    fn should_normalize_malformed_tool_call_input_and_preserve_the_tool_error() {
        let result = convert(&prompt(json!([
            {
                "role": "assistant",
                "content": [{
                    "type": "tool-call",
                    "toolCallId": "quux",
                    "toolName": "thwomp",
                    "input": "{\"foo\":\"bar\"",
                }],
            },
            {
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "quux",
                    "toolName": "thwomp",
                    "output": {
                        "type": "error-text",
                        "value": "Invalid input: JSON parsing failed",
                    },
                }],
            },
        ])))
        .unwrap();

        assert_eq!(
            result,
            json!([
                {
                    "content": null,
                    "role": "assistant",
                    "tool_calls": [{
                        "function": { "arguments": "{}", "name": "thwomp" },
                        "id": "quux",
                        "type": "function",
                    }],
                },
                {
                    "content": "Invalid input: JSON parsing failed",
                    "role": "tool",
                    "tool_call_id": "quux",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/convert-to-openai-chat-messages.test.ts › tool calls › should handle different tool output types
    #[test]
    fn should_handle_different_tool_output_types() {
        let result = convert(&prompt(json!([{
            "role": "tool",
            "content": [
                {
                    "type": "tool-result",
                    "toolCallId": "text-tool",
                    "toolName": "text-tool",
                    "output": { "type": "text", "value": "Hello world" },
                },
                {
                    "type": "tool-result",
                    "toolCallId": "error-tool",
                    "toolName": "error-tool",
                    "output": { "type": "error-text", "value": "Something went wrong" },
                },
            ],
        }])))
        .unwrap();

        assert_eq!(
            result,
            json!([
                { "content": "Hello world", "role": "tool", "tool_call_id": "text-tool" },
                {
                    "content": "Something went wrong",
                    "role": "tool",
                    "tool_call_id": "error-tool",
                },
            ])
        );
    }
}

// ── Older ports (previously the inline tests of `src/openai/convert.rs`) ──
mod older_convert {
    use aimux_core::language_model_message::ToolCallPart;

    use crate::openai::chat::convert_to_chat_messages::*;
    use crate::openai::language_model_capabilities::SystemMessageMode;
    use aimux_core::language_model_message::AssistantPart;
    use aimux_core::language_model_message::LanguageModelMessage;
    use serde_json::Value;
    use serde_json::json;

    fn replayed_arguments(input: Value) -> Value {
        let prompt = vec![LanguageModelMessage::Assistant {
            content: vec![AssistantPart::ToolCall(ToolCallPart {
                tool_call_id: "call-1".to_string(),
                tool_name: "echo".to_string(),
                input,
                provider_executed: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let messages = convert_prompt_to_openai_messages_with_mode_fallible(
            &prompt,
            SystemMessageMode::System,
        )
        .unwrap();
        messages[0]["tool_calls"][0]["function"]["arguments"].clone()
    }

    /// partial: `should stringify arguments to tool calls`; full port in src/openai/tests/chat/convert_to_chat_messages.rs.
    /// partial: `should default missing tool call input to an empty object`; full port in src/openai/tests/chat/convert_to_chat_messages.rs.
    /// partial: `should normalize malformed tool call input and preserve the tool error`; full port in src/openai/tests/chat/convert_to_chat_messages.rs.
    #[test]
    fn replayed_tool_arguments_follow_upstream() {
        assert_eq!(
            replayed_arguments(json!({ "city": "Tokyo" })),
            json!(r#"{"city":"Tokyo"}"#)
        );
        for input in [json!("Tokyo"), json!(7), json!(["Tokyo"]), Value::Null] {
            assert_eq!(replayed_arguments(input), json!("{}"));
        }
    }
}
