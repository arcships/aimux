use aimux_core::AiMuxError;
use aimux_core::language_model_message::LanguageModelPrompt;
use serde_json::{Value, json};

use crate::bedrock::convert_to_chat_messages::{convert_prompt, convert_prompt_to_bedrock};

/// Upstream prompts are written as `LanguageModelV4Prompt` JSON literals.
fn prompt(value: Value) -> LanguageModelPrompt {
    serde_json::from_value(value).expect("valid prompt")
}

fn to_result((system, messages): (Vec<Value>, Vec<Value>)) -> Value {
    json!({ "system": system, "messages": messages })
}

/// `convertToAmazonBedrockChatMessages(prompt)` (isMistral not provided).
fn convert(value: Value) -> Result<Value, AiMuxError> {
    convert_prompt_to_bedrock(&prompt(value)).map(to_result)
}

/// `convertToAmazonBedrockChatMessages(prompt, isMistral)`.
fn convert_with(value: Value, is_mistral: bool) -> Result<Value, AiMuxError> {
    convert_prompt(&prompt(value), is_mistral).map(to_result)
}

/// The message of an `AI_UnsupportedFunctionalityError`.
/// The `functionality` of an `UnsupportedFunctionalityError`.
///
/// aimux's `UnsupportedFunctionality` carries one string, upstream's
/// `functionality`; it has no separate `message`. Upstream's default message
/// is `'<functionality>' functionality not supported.`, and the mime type
/// errors set their own message around the same functionality
/// (`convert-to-amazon-bedrock-chat-messages.ts:701-743`), so these cases
/// compare the functionality the upstream source throws with.
fn unsupported_functionality(error: AiMuxError) -> String {
    match error {
        AiMuxError::UnsupportedFunctionality(functionality) => functionality,
        other => panic!("expected AI_UnsupportedFunctionalityError, got {other:?}"),
    }
}

mod system_messages {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › system messages › should throw an error if a system message is provided after a non-system message
    #[test]
    fn should_throw_an_error_if_a_system_message_is_provided_after_a_non_system_message() {
        assert!(
            convert(json!([
                { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                { "role": "system", "content": "World" },
            ]))
            .is_err()
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › system messages › should set isSystemCachePoint when system message has cache point
    #[test]
    fn should_set_is_system_cache_point_when_system_message_has_cache_point() {
        let result = convert(json!([{
            "role": "system",
            "content": "Hello",
            "providerOptions": { "bedrock": { "cachePoint": { "type": "default" } } },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "system": [{ "text": "Hello" }, { "cachePoint": { "type": "default" } }],
                "messages": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › system messages › should add cache point with 5m TTL to system message
    #[test]
    fn should_add_cache_point_with_5m_ttl_to_system_message() {
        let result = convert(json!([{
            "role": "system",
            "content": "Hello",
            "providerOptions": {
                "bedrock": { "cachePoint": { "type": "default", "ttl": "5m" } },
            },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "system": [
                    { "text": "Hello" },
                    { "cachePoint": { "type": "default", "ttl": "5m" } },
                ],
                "messages": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › system messages › should add cache point with 1h TTL to system message
    #[test]
    fn should_add_cache_point_with_1h_ttl_to_system_message() {
        let result = convert(json!([{
            "role": "system",
            "content": "Hello",
            "providerOptions": {
                "bedrock": { "cachePoint": { "type": "default", "ttl": "1h" } },
            },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "system": [
                    { "text": "Hello" },
                    { "cachePoint": { "type": "default", "ttl": "1h" } },
                ],
                "messages": [],
            })
        );
    }
}

mod user_messages {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert image parts with S3 URLs
    #[test]
    fn should_convert_image_parts_with_s3_urls() {
        let result = convert(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "Describe the image" },
                {
                    "type": "file",
                    "data": { "type": "url", "url": "s3://my-test-bucket/path/to/image.png" },
                    "mediaType": "image/png",
                },
            ],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [
                    { "text": "Describe the image" },
                    {
                        "image": {
                            "format": "png",
                            "source": {
                                "s3Location": { "uri": "s3://my-test-bucket/path/to/image.png" },
                            },
                        },
                    },
                ],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert messages with video parts
    #[test]
    fn should_convert_messages_with_video_parts() {
        let result = convert(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "Describe the video" },
                {
                    "type": "file",
                    "data": { "type": "data", "data": "AAECAw==" },
                    "mediaType": "video/mp4",
                },
            ],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [
                    { "text": "Describe the video" },
                    { "video": { "format": "mp4", "source": { "bytes": "AAECAw==" } } },
                ],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert video parts with S3 URLs
    #[test]
    fn should_convert_video_parts_with_s3_urls() {
        let result = convert(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "Describe the video" },
                {
                    "type": "file",
                    "data": { "type": "url", "url": "s3://my-test-bucket/path/to/video.mp4" },
                    "mediaType": "video/mp4",
                },
            ],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [
                    { "text": "Describe the video" },
                    {
                        "video": {
                            "format": "mp4",
                            "source": {
                                "s3Location": { "uri": "s3://my-test-bucket/path/to/video.mp4" },
                            },
                        },
                    },
                ],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should throw for unsupported video mime types
    #[test]
    fn should_throw_for_unsupported_video_mime_types() {
        let error = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "base64data" },
                "mediaType": "video/unsupported",
            }],
        }]))
        .unwrap_err();

        assert_eq!(
            unsupported_functionality(error),
            "video mime type: video/unsupported"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert messages with document parts
    #[test]
    fn should_convert_messages_with_document_parts() {
        let result = convert(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "Hello" },
                {
                    "type": "file",
                    "data": { "type": "data", "data": "AAECAw==" },
                    "mediaType": "application/pdf",
                },
            ],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "content": [
                    { "text": "Hello" },
                    {
                        "document": {
                            "format": "pdf",
                            "name": "document-1",
                            "source": { "bytes": "AAECAw==" },
                        },
                    },
                ],
                "role": "user",
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should strip file extension when filename is provided
    #[test]
    fn should_strip_file_extension_when_filename_is_provided() {
        let result = convert(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "Hello" },
                {
                    "type": "file",
                    "data": { "type": "data", "data": "AAECAw==" },
                    "mediaType": "application/pdf",
                    "filename": "custom-filename.pdf",
                },
            ],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "content": [
                    { "text": "Hello" },
                    {
                        "document": {
                            "format": "pdf",
                            "name": "custom-filename",
                            "source": { "bytes": "AAECAw==" },
                        },
                    },
                ],
                "role": "user",
            }])
        );
    }

    // Not an upstream case: `sanitizeDocumentName` cuts at the first dot
    // (provider-utils `stripFileExtension`), so a multi-dot filename loses
    // every extension segment.
    #[test]
    fn strips_every_extension_segment_of_a_multi_dot_filename() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "AAECAw==" },
                "mediaType": "application/pdf",
                "filename": "report.v2.final.pdf",
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"][0]["content"][0]["document"]["name"],
            "report"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should preserve filename without extension when provided
    #[test]
    fn should_preserve_filename_without_extension_when_provided() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "AAECAw==" },
                "mediaType": "application/pdf",
                "filename": "custom-filename",
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "content": [{
                    "document": {
                        "format": "pdf",
                        "name": "custom-filename",
                        "source": { "bytes": "AAECAw==" },
                    },
                }],
                "role": "user",
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should sanitize document filenames to Bedrock-compatible names
    #[test]
    fn should_sanitize_document_filenames_to_bedrock_compatible_names() {
        let long_name = format!("{}.txt", "a".repeat(201));
        let filenames = [
            "John's report.txt",
            "invoice #123.txt",
            "a&b.txt",
            "report,2026.txt",
            "résumé.txt",
            "분기보고서.txt",
            "Report -  Final.txt",
            "a\tb.txt",
            long_name.as_str(),
            ".txt",
            "report (final) [v2]_draft.txt",
        ];

        let result = convert(json!([{
            "role": "user",
            "content": filenames
                .iter()
                .map(|filename| json!({
                    "type": "file",
                    "data": { "type": "data", "data": "base64data" },
                    "mediaType": "application/pdf",
                    "filename": filename,
                }))
                .collect::<Vec<_>>(),
        }]))
        .unwrap();

        let names: Vec<Value> = result["messages"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|content| content["document"]["name"].clone())
            .collect();
        assert_eq!(
            Value::Array(names),
            json!([
                "Johns report",
                "invoice 123",
                "ab",
                "report2026",
                "rsum",
                "document-1",
                "Report - Final",
                "a b",
                "a".repeat(200),
                "document-2",
                "report (final) [v2]draft",
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should sanitize filenames for text document data
    #[test]
    fn should_sanitize_filenames_for_text_document_data() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "text", "text": "Hello" },
                "mediaType": "text/plain",
                "filename": "John's  report.txt",
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"][0]["content"][0],
            json!({
                "document": {
                    "format": "txt",
                    "name": "Johns report",
                    "source": { "bytes": "SGVsbG8=" },
                },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should add cache point to user message content when specified
    #[test]
    fn should_add_cache_point_to_user_message_content_when_specified() {
        let result = convert(json!([{
            "role": "user",
            "content": [{ "type": "text", "text": "Hello" }],
            "providerOptions": { "bedrock": { "cachePoint": { "type": "default" } } },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "role": "user",
                    "content": [{ "text": "Hello" }, { "cachePoint": { "type": "default" } }],
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should add cache point with 5m TTL to user message
    #[test]
    fn should_add_cache_point_with_5m_ttl_to_user_message() {
        let result = convert(json!([{
            "role": "user",
            "content": [{ "type": "text", "text": "Hello" }],
            "providerOptions": {
                "bedrock": { "cachePoint": { "type": "default", "ttl": "5m" } },
            },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "role": "user",
                    "content": [
                        { "text": "Hello" },
                        { "cachePoint": { "type": "default", "ttl": "5m" } },
                    ],
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should add cache point with 1h TTL to user message
    #[test]
    fn should_add_cache_point_with_1h_ttl_to_user_message() {
        let result = convert(json!([{
            "role": "user",
            "content": [{ "type": "text", "text": "Hello" }],
            "providerOptions": {
                "bedrock": { "cachePoint": { "type": "default", "ttl": "1h" } },
            },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "role": "user",
                    "content": [
                        { "text": "Hello" },
                        { "cachePoint": { "type": "default", "ttl": "1h" } },
                    ],
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert text part to guardContent when guardContent provider option is true
    #[test]
    fn should_convert_text_part_to_guard_content_when_guard_content_option_is_true() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "text",
                "text": "Grounding text",
                "providerOptions": { "amazonBedrock": { "guardContent": true } },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [{ "guardContent": { "text": { "text": "Grounding text" } } }],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert text part to guardContent with qualifiers
    #[test]
    fn should_convert_text_part_to_guard_content_with_qualifiers() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "text",
                "text": "Grounding text",
                "providerOptions": {
                    "amazonBedrock": {
                        "guardContent": true,
                        "guardContentQualifiers": ["grounding_source"],
                    },
                },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [{
                    "guardContent": {
                        "text": { "text": "Grounding text", "qualifiers": ["grounding_source"] },
                    },
                }],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert text part to guardContent with multiple qualifiers
    #[test]
    fn should_convert_text_part_to_guard_content_with_multiple_qualifiers() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "text",
                "text": "Query text",
                "providerOptions": {
                    "amazonBedrock": {
                        "guardContent": true,
                        "guardContentQualifiers": ["grounding_source", "query"],
                    },
                },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [{
                    "guardContent": {
                        "text": {
                            "text": "Query text",
                            "qualifiers": ["grounding_source", "query"],
                        },
                    },
                }],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert text part as normal text when guardContent is false
    #[test]
    fn should_convert_text_part_as_normal_text_when_guard_content_is_false() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "text",
                "text": "Normal text",
                "providerOptions": { "amazonBedrock": { "guardContent": false } },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{ "role": "user", "content": [{ "text": "Normal text" }] }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert text part as normal text when no provider options
    #[test]
    fn should_convert_text_part_as_normal_text_when_no_provider_options() {
        let result = convert(json!([{
            "role": "user",
            "content": [{ "type": "text", "text": "Normal text" }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{ "role": "user", "content": [{ "text": "Normal text" }] }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert image part to guardContent when guardContent provider option is true
    #[test]
    fn should_convert_image_part_to_guard_content_when_guard_content_option_is_true() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "AAECAw==" },
                "mediaType": "image/png",
                "providerOptions": { "amazonBedrock": { "guardContent": true } },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [{
                    "guardContent": {
                        "image": { "format": "png", "source": { "bytes": "AAECAw==" } },
                    },
                }],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert image part as normal image when guardContent is false
    #[test]
    fn should_convert_image_part_as_normal_image_when_guard_content_is_false() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "AAECAw==" },
                "mediaType": "image/png",
                "providerOptions": { "amazonBedrock": { "guardContent": false } },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [{ "image": { "format": "png", "source": { "bytes": "AAECAw==" } } }],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert image part as normal image when no provider options
    #[test]
    fn should_convert_image_part_as_normal_image_when_no_provider_options() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "AAECAw==" },
                "mediaType": "image/png",
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "user",
                "content": [{ "image": { "format": "png", "source": { "bytes": "AAECAw==" } } }],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should throw for file parts with provider references
    #[test]
    fn should_throw_for_file_parts_with_provider_references() {
        let error = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "reference", "reference": { "bedrock": "file-ref-123" } },
                "mediaType": "image/png",
            }],
        }]))
        .unwrap_err();

        assert_eq!(
            unsupported_functionality(error),
            "file parts with provider references"
        );
    }
}

mod assistant_messages {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should preserve the order of provider-executed tool calls and results
    #[test]
    fn should_preserve_the_order_of_provider_executed_tool_calls_and_results() {
        let result = convert(json!([
            {
                "role": "user",
                "content": [{ "type": "text", "text": "Add 2 and 2, then add 3 and 3." }],
            },
            {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "Running the additions." },
                    {
                        "type": "tool-call",
                        "toolCallId": "call-1",
                        "toolName": "add",
                        "input": { "a": 2, "b": 2 },
                        "providerExecuted": true,
                    },
                    {
                        "type": "tool-result",
                        "toolCallId": "call-1",
                        "toolName": "add",
                        "output": { "type": "json", "value": { "sum": 4 } },
                    },
                    {
                        "type": "tool-call",
                        "toolCallId": "call-2",
                        "toolName": "add",
                        "input": { "a": 3, "b": 3 },
                        "providerExecuted": true,
                    },
                    {
                        "type": "tool-result",
                        "toolCallId": "call-2",
                        "toolName": "add",
                        "output": { "type": "json", "value": { "sum": 6 } },
                    },
                    { "type": "text", "text": "The sums are 4 and 6." },
                ],
            },
            {
                "role": "user",
                "content": [{ "type": "text", "text": "Now add those sums together." }],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "system": [],
                "messages": [
                    { "role": "user", "content": [{ "text": "Add 2 and 2, then add 3 and 3." }] },
                    {
                        "role": "assistant",
                        "content": [
                            { "text": "Running the additions." },
                            {
                                "toolUse": {
                                    "toolUseId": "call-1",
                                    "name": "add",
                                    "input": { "a": 2, "b": 2 },
                                },
                            },
                        ],
                    },
                    {
                        "role": "user",
                        "content": [{
                            "toolResult": {
                                "toolUseId": "call-1",
                                "content": [{ "text": "{\"sum\":4}" }],
                            },
                        }],
                    },
                    {
                        "role": "assistant",
                        "content": [{
                            "toolUse": {
                                "toolUseId": "call-2",
                                "name": "add",
                                "input": { "a": 3, "b": 3 },
                            },
                        }],
                    },
                    {
                        "role": "user",
                        "content": [{
                            "toolResult": {
                                "toolUseId": "call-2",
                                "content": [{ "text": "{\"sum\":6}" }],
                            },
                        }],
                    },
                    { "role": "assistant", "content": [{ "text": "The sums are 4 and 6." }] },
                    { "role": "user", "content": [{ "text": "Now add those sums together." }] },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should combine a trailing provider-executed tool result with the next user message
    #[test]
    fn should_combine_a_trailing_provider_executed_tool_result_with_the_next_user_message() {
        let result = convert(json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "call-1",
                        "toolName": "add",
                        "input": { "a": 2, "b": 2 },
                        "providerExecuted": true,
                    },
                    {
                        "type": "tool-result",
                        "toolCallId": "call-1",
                        "toolName": "add",
                        "output": { "type": "json", "value": { "sum": 4 } },
                    },
                ],
            },
            { "role": "user", "content": [{ "type": "text", "text": "Now double that." }] },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "system": [],
                "messages": [
                    {
                        "role": "assistant",
                        "content": [{
                            "toolUse": {
                                "toolUseId": "call-1",
                                "name": "add",
                                "input": { "a": 2, "b": 2 },
                            },
                        }],
                    },
                    {
                        "role": "user",
                        "content": [
                            {
                                "toolResult": {
                                    "toolUseId": "call-1",
                                    "content": [{ "text": "{\"sum\":4}" }],
                                },
                            },
                            { "text": "Now double that." },
                        ],
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should remove trailing whitespace from last assistant message when there is no further user message
    #[test]
    fn should_remove_trailing_whitespace_from_last_assistant_message() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
            {
                "role": "assistant",
                "content": [{ "type": "text", "text": "assistant content  " }],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "user content" }] },
                    { "role": "assistant", "content": [{ "text": "assistant content" }] },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should remove trailing whitespace from last assistant message with multi-part content when there is no further user message
    #[test]
    fn should_remove_trailing_whitespace_from_last_assistant_message_with_multi_part_content() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
            {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "assistant " },
                    { "type": "text", "text": "content  " },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "user content" }] },
                    {
                        "role": "assistant",
                        "content": [{ "text": "assistant " }, { "text": "content" }],
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should keep trailing whitespace from assistant message when there is a further user message
    #[test]
    fn should_keep_trailing_whitespace_from_assistant_message_when_there_is_a_further_user_message()
    {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
            {
                "role": "assistant",
                "content": [{ "type": "text", "text": "assistant content  " }],
            },
            { "role": "user", "content": [{ "type": "text", "text": "user content 2" }] },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "user content" }] },
                    { "role": "assistant", "content": [{ "text": "assistant content  " }] },
                    { "role": "user", "content": [{ "text": "user content 2" }] },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should combine multiple sequential assistant messages into a single message
    #[test]
    fn should_combine_multiple_sequential_assistant_messages_into_a_single_message() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Hi!" }] },
            { "role": "assistant", "content": [{ "type": "text", "text": "Hello" }] },
            { "role": "assistant", "content": [{ "type": "text", "text": "World" }] },
            { "role": "assistant", "content": [{ "type": "text", "text": "!" }] },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Hi!" }] },
                    {
                        "role": "assistant",
                        "content": [{ "text": "Hello" }, { "text": "World" }, { "text": "!" }],
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should add cache point to assistant message content when specified
    #[test]
    fn should_add_cache_point_to_assistant_message_content_when_specified() {
        let result = convert(json!([{
            "role": "assistant",
            "content": [{ "type": "text", "text": "Hello" }],
            "providerOptions": { "bedrock": { "cachePoint": { "type": "default" } } },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "role": "assistant",
                    "content": [{ "text": "Hello" }, { "cachePoint": { "type": "default" } }],
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should add cache point with 5m TTL to assistant message
    #[test]
    fn should_add_cache_point_with_5m_ttl_to_assistant_message() {
        let result = convert(json!([{
            "role": "assistant",
            "content": [{ "type": "text", "text": "Hello" }],
            "providerOptions": {
                "bedrock": { "cachePoint": { "type": "default", "ttl": "5m" } },
            },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "role": "assistant",
                    "content": [
                        { "text": "Hello" },
                        { "cachePoint": { "type": "default", "ttl": "5m" } },
                    ],
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should add cache point with 1h TTL to assistant message
    #[test]
    fn should_add_cache_point_with_1h_ttl_to_assistant_message() {
        let result = convert(json!([{
            "role": "assistant",
            "content": [{ "type": "text", "text": "Hello" }],
            "providerOptions": {
                "bedrock": { "cachePoint": { "type": "default", "ttl": "1h" } },
            },
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "role": "assistant",
                    "content": [
                        { "text": "Hello" },
                        { "cachePoint": { "type": "default", "ttl": "1h" } },
                    ],
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should add cache point to assistant content part when specified
    #[test]
    fn should_add_cache_point_to_assistant_content_part_when_specified() {
        let result = convert(json!([{
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Hello" },
                {
                    "type": "text",
                    "text": "cached",
                    "providerOptions": {
                        "bedrock": { "cachePoint": { "type": "default", "ttl": "1h" } },
                    },
                },
                { "type": "text", "text": "World" },
            ],
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "role": "assistant",
                    "content": [
                        { "text": "Hello" },
                        { "text": "cached" },
                        { "cachePoint": { "type": "default", "ttl": "1h" } },
                        { "text": "World" },
                    ],
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should properly convert reasoning content type
    #[test]
    fn should_properly_convert_reasoning_content_type() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [{
                    "type": "reasoning",
                    "text": "This is my step-by-step reasoning process",
                    "providerOptions": { "bedrock": { "signature": "test-signature" } },
                }],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
                    {
                        "role": "assistant",
                        "content": [{
                            "reasoningContent": {
                                "reasoningText": {
                                    "text": "This is my step-by-step reasoning process",
                                    "signature": "test-signature",
                                },
                            },
                        }],
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should properly convert redacted-reasoning content type
    #[test]
    fn should_properly_convert_redacted_reasoning_content_type() {
        let reasoning_data = "Redacted reasoning information";
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [{
                    "type": "reasoning",
                    "text": "",
                    "providerOptions": { "bedrock": { "redactedData": reasoning_data } },
                }],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
                    {
                        "role": "assistant",
                        "content": [{
                            "reasoningContent": { "redactedReasoning": { "data": reasoning_data } },
                        }],
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should replay reasoning redacted as `redactedContent`
    #[test]
    fn should_replay_reasoning_redacted_as_redacted_content() {
        let redacted_content = "encrypted-reasoning-payload";
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [{
                    "type": "reasoning",
                    "text": "",
                    "providerOptions": { "bedrock": { "redactedContent": redacted_content } },
                }],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
                    {
                        "role": "assistant",
                        "content": [{ "reasoningContent": { "redactedContent": redacted_content } }],
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should omit assistant message reasoning parts signed by a foreign provider
    #[test]
    fn should_omit_assistant_message_reasoning_parts_signed_by_a_foreign_provider() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning",
                        "text": "Anthropic-signed reasoning replayed to Bedrock",
                        "providerOptions": { "anthropic": { "signature": "anthropic-signature" } },
                    },
                    { "type": "text", "text": "final answer" },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
                    { "role": "assistant", "content": [{ "text": "final answer" }] },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should preserve assistant message reasoning parts with amazonBedrock providerOptions
    #[test]
    fn should_preserve_assistant_message_reasoning_parts_with_amazon_bedrock_provider_options() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning",
                        "text": "Bedrock-signed reasoning round-tripped to Bedrock",
                        "providerOptions": { "amazonBedrock": { "signature": "bedrock-signature" } },
                    },
                    { "type": "text", "text": "final answer" },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
                    {
                        "role": "assistant",
                        "content": [
                            {
                                "reasoningContent": {
                                    "reasoningText": {
                                        "text": "Bedrock-signed reasoning round-tripped to Bedrock",
                                        "signature": "bedrock-signature",
                                    },
                                },
                            },
                            { "text": "final answer" },
                        ],
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should not trim reasoning text when a signature is present
    #[test]
    fn should_not_trim_reasoning_text_when_a_signature_is_present() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [{
                    "type": "reasoning",
                    "text": "This is my reasoning with trailing space    ",
                    "providerOptions": { "bedrock": { "signature": "test-signature" } },
                }],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "content": [{ "text": "Explain your reasoning" }], "role": "user" },
                    {
                        "content": [{
                            "reasoningContent": {
                                "reasoningText": {
                                    "signature": "test-signature",
                                    "text": "This is my reasoning with trailing space    ",
                                },
                            },
                        }],
                        "role": "assistant",
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should omit reasoning content without signature
    #[test]
    fn should_omit_reasoning_content_without_signature() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    { "type": "reasoning", "text": "This is my reasoning with trailing space    " },
                    { "type": "text", "text": "final answer" },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "content": [{ "text": "Explain your reasoning" }], "role": "user" },
                    { "content": [{ "text": "final answer" }], "role": "assistant" },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should omit an assistant message when unsigned reasoning is its only content
    #[test]
    fn should_omit_an_assistant_message_when_unsigned_reasoning_is_its_only_content() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Think hard then answer" }] },
            {
                "role": "assistant",
                "content": [{ "type": "reasoning", "text": "Let me consider the options" }],
            },
            { "role": "user", "content": [{ "type": "text", "text": "Hello?" }] },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Think hard then answer" }] },
                    { "role": "user", "content": [{ "text": "Hello?" }] },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should omit an assistant message when only a cache point remains after filtering unsigned reasoning
    #[test]
    fn should_omit_an_assistant_message_when_only_a_cache_point_remains() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Think hard then answer" }] },
            {
                "role": "assistant",
                "content": [{ "type": "reasoning", "text": "Let me consider the options" }],
                "providerOptions": { "bedrock": { "cachePoint": { "type": "default" } } },
            },
            { "role": "user", "content": [{ "type": "text", "text": "Hello?" }] },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Think hard then answer" }] },
                    { "role": "user", "content": [{ "text": "Hello?" }] },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should omit multiple reasoning parts without signatures
    #[test]
    fn should_omit_multiple_reasoning_parts_without_signatures() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    { "type": "reasoning", "text": "First reasoning with trailing space    " },
                    { "type": "reasoning", "text": "Second reasoning with trailing space    " },
                    { "type": "text", "text": "final answer" },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "content": [{ "text": "Explain your reasoning" }], "role": "user" },
                    { "content": [{ "text": "final answer" }], "role": "assistant" },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should omit unsigned reasoning while preserving tool calls in multi-turn tool use
    #[test]
    fn should_omit_unsigned_reasoning_while_preserving_tool_calls_in_multi_turn_tool_use() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "What is the weather?" }] },
            {
                "role": "assistant",
                "content": [
                    { "type": "reasoning", "text": "I should call the weather tool." },
                    {
                        "type": "tool-call",
                        "toolCallId": "call-1",
                        "toolName": "getWeather",
                        "input": { "city": "SF" },
                    },
                ],
            },
            {
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "call-1",
                    "toolName": "getWeather",
                    "output": { "type": "text", "value": "Sunny, 72F" },
                }],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "content": [{ "text": "What is the weather?" }], "role": "user" },
                    {
                        "content": [{
                            "toolUse": {
                                "input": { "city": "SF" },
                                "name": "getWeather",
                                "toolUseId": "call-1",
                            },
                        }],
                        "role": "assistant",
                    },
                    {
                        "content": [{
                            "toolResult": {
                                "content": [{ "text": "Sunny, 72F" }],
                                "toolUseId": "call-1",
                            },
                        }],
                        "role": "user",
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should preserve reasoning text with signature in multi-turn tool use
    #[test]
    fn should_preserve_reasoning_text_with_signature_in_multi_turn_tool_use() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "What is the weather?" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning",
                        "text": "Let me check the weather API.\n",
                        "providerOptions": { "bedrock": { "signature": "sig-abc123" } },
                    },
                    {
                        "type": "tool-call",
                        "toolCallId": "call-1",
                        "toolName": "getWeather",
                        "input": { "city": "SF" },
                    },
                ],
            },
            {
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "call-1",
                    "toolName": "getWeather",
                    "output": { "type": "text", "value": "Sunny, 72F" },
                }],
            },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning",
                        "text": "The weather is sunny and warm.\n",
                        "providerOptions": { "bedrock": { "signature": "sig-def456" } },
                    },
                    { "type": "text", "text": "It is sunny and 72F in SF." },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "content": [{ "text": "What is the weather?" }], "role": "user" },
                    {
                        "content": [
                            {
                                "reasoningContent": {
                                    "reasoningText": {
                                        "signature": "sig-abc123",
                                        "text": "Let me check the weather API.\n",
                                    },
                                },
                            },
                            {
                                "toolUse": {
                                    "input": { "city": "SF" },
                                    "name": "getWeather",
                                    "toolUseId": "call-1",
                                },
                            },
                        ],
                        "role": "assistant",
                    },
                    {
                        "content": [{
                            "toolResult": {
                                "content": [{ "text": "Sunny, 72F" }],
                                "toolUseId": "call-1",
                            },
                        }],
                        "role": "user",
                    },
                    {
                        "content": [
                            {
                                "reasoningContent": {
                                    "reasoningText": {
                                        "signature": "sig-def456",
                                        "text": "The weather is sunny and warm.\n",
                                    },
                                },
                            },
                            { "text": "It is sunny and 72F in SF." },
                        ],
                        "role": "assistant",
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should handle a mix of text and reasoning content types
    #[test]
    fn should_handle_a_mix_of_text_and_reasoning_content_types() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "My answer is 42." },
                    {
                        "type": "reasoning",
                        "text": "I calculated this by analyzing the meaning of life",
                        "providerOptions": { "bedrock": { "signature": "reasoning-process" } },
                    },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
                    {
                        "role": "assistant",
                        "content": [
                            { "text": "My answer is 42." },
                            {
                                "reasoningContent": {
                                    "reasoningText": {
                                        "text": "I calculated this by analyzing the meaning of life",
                                        "signature": "reasoning-process",
                                    },
                                },
                            },
                        ],
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should filter out empty text blocks in assistant messages
    #[test]
    fn should_filter_out_empty_text_blocks_in_assistant_messages() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
            {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "\n\n" },
                    { "type": "tool-call", "toolCallId": "call-123", "toolName": "test", "input": {} },
                    { "type": "text", "text": "  " },
                    { "type": "text", "text": "actual content" },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Hello" }] },
                    {
                        "role": "assistant",
                        "content": [
                            { "toolUse": { "toolUseId": "call-123", "name": "test", "input": {} } },
                            { "text": "actual content" },
                        ],
                    },
                ],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should wrap non-object (invalid) tool call input in an object
    #[test]
    fn should_wrap_non_object_invalid_tool_call_input_in_an_object() {
        let result = convert(json!([{
            "role": "assistant",
            "content": [{
                "type": "tool-call",
                "toolCallId": "call-1",
                "toolName": "cityAttractions",
                // malformed JSON the model produced, kept as a raw string
                "input": "{ \"city\": \"San Francisco\", }",
            }],
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "role": "assistant",
                    "content": [{
                        "toolUse": {
                            "toolUseId": "call-1",
                            "name": "cityAttractions",
                            "input": { "rawInvalidInput": "{ \"city\": \"San Francisco\", }" },
                        },
                    }],
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should strip invalid characters from tool call names
    #[test]
    fn should_strip_invalid_characters_from_tool_call_names() {
        let result = convert(json!([{
            "role": "assistant",
            "content": [
                { "type": "tool-call", "toolCallId": "call-1", "toolName": "$READFILE", "input": {} },
                {
                    "type": "tool-call",
                    "toolCallId": "call-2",
                    "toolName": "exchange_delivered_order_items<|channel|>",
                    "input": {},
                },
                { "type": "tool-call", "toolCallId": "call-3", "toolName": "$", "input": {} },
            ],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"],
            json!([{
                "role": "assistant",
                "content": [
                    { "toolUse": { "toolUseId": "call-1", "name": "READFILE", "input": {} } },
                    {
                        "toolUse": {
                            "toolUseId": "call-2",
                            "name": "exchange_delivered_order_itemschannel",
                            "input": {},
                        },
                    },
                    { "toolUse": { "toolUseId": "call-3", "name": "_", "input": {} } },
                ],
            }])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › assistant messages › should preserve empty text blocks when reasoning blocks are present
    #[test]
    fn should_preserve_empty_text_blocks_when_reasoning_blocks_are_present() {
        let result = convert(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning",
                        "text": "thinking...",
                        "providerOptions": { "bedrock": { "signature": "sig-1" } },
                    },
                    { "type": "text", "text": "" },
                    {
                        "type": "reasoning",
                        "text": "more thinking...",
                        "providerOptions": { "bedrock": { "signature": "sig-2" } },
                    },
                    { "type": "text", "text": "response text" },
                    { "type": "tool-call", "toolCallId": "call-123", "toolName": "test", "input": {} },
                ],
            },
        ]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [
                    { "role": "user", "content": [{ "text": "Hello" }] },
                    {
                        "role": "assistant",
                        "content": [
                            {
                                "reasoningContent": {
                                    "reasoningText": { "text": "thinking...", "signature": "sig-1" },
                                },
                            },
                            { "text": "" },
                            {
                                "reasoningContent": {
                                    "reasoningText": {
                                        "text": "more thinking...",
                                        "signature": "sig-2",
                                    },
                                },
                            },
                            { "text": "response text" },
                            { "toolUse": { "toolUseId": "call-123", "name": "test", "input": {} } },
                        ],
                    },
                ],
                "system": [],
            })
        );
    }
}

mod tool_messages {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should convert tool result with content array containing image
    #[test]
    fn should_convert_tool_result_with_content_array_containing_image() {
        let result = convert(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "call-123",
                "toolName": "image-generator",
                "output": {
                    "type": "content",
                    "value": [{
                        "type": "file",
                        "data": { "type": "data", "data": "base64data" },
                        "mediaType": "image/jpeg",
                    }],
                },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": "call-123",
                        "content": [
                            { "image": { "format": "jpeg", "source": { "bytes": "base64data" } } },
                        ],
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should convert tool result images with S3 URLs
    #[test]
    fn should_convert_tool_result_images_with_s3_urls() {
        let result = convert(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "call-123",
                "toolName": "image-generator",
                "output": {
                    "type": "content",
                    "value": [{
                        "type": "file",
                        "data": { "type": "url", "url": "s3://my-test-bucket/path/to/image.png" },
                        "mediaType": "image/png",
                    }],
                },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": "call-123",
                        "content": [{
                            "image": {
                                "format": "png",
                                "source": {
                                    "s3Location": { "uri": "s3://my-test-bucket/path/to/image.png" },
                                },
                            },
                        }],
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should convert tool result with content array containing video
    #[test]
    fn should_convert_tool_result_with_content_array_containing_video() {
        let result = convert(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "call-123",
                "toolName": "video-analyzer",
                "output": {
                    "type": "content",
                    "value": [{
                        "type": "file",
                        "data": { "type": "data", "data": "base64data" },
                        "mediaType": "video/mp4",
                    }],
                },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": "call-123",
                        "content": [
                            { "video": { "format": "mp4", "source": { "bytes": "base64data" } } },
                        ],
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should convert tool result videos with S3 URLs
    #[test]
    fn should_convert_tool_result_videos_with_s3_urls() {
        let result = convert(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "call-123",
                "toolName": "video-analyzer",
                "output": {
                    "type": "content",
                    "value": [{
                        "type": "file",
                        "data": { "type": "url", "url": "s3://my-test-bucket/path/to/video.mp4" },
                        "mediaType": "video/mp4",
                    }],
                },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": "call-123",
                        "content": [{
                            "video": {
                                "format": "mp4",
                                "source": {
                                    "s3Location": { "uri": "s3://my-test-bucket/path/to/video.mp4" },
                                },
                            },
                        }],
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should convert tool result with content array containing PDF
    #[test]
    fn should_convert_tool_result_with_content_array_containing_pdf() {
        let result = convert(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "call-123",
                "toolName": "document-reader",
                "output": {
                    "type": "content",
                    "value": [{
                        "type": "file",
                        "data": { "type": "data", "data": "base64data" },
                        "mediaType": "application/pdf",
                        "filename": "tool's  result.pdf",
                    }],
                },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": "call-123",
                        "content": [{
                            "document": {
                                "format": "pdf",
                                "name": "tools result",
                                "source": { "bytes": "base64data" },
                            },
                        }],
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should throw error for unsupported image format in tool result content
    #[test]
    fn should_throw_error_for_unsupported_image_format_in_tool_result_content() {
        let error = convert(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "call-123",
                "toolName": "image-generator",
                "output": {
                    "type": "content",
                    "value": [{
                        "type": "file",
                        "data": { "type": "data", "data": "base64data" },
                        "mediaType": "image/avif",
                    }],
                },
            }],
        }]))
        .unwrap_err();

        assert_eq!(
            unsupported_functionality(error),
            "image mime type: image/avif"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should throw error for unsupported mime type in tool result file content
    #[test]
    fn should_throw_error_for_unsupported_mime_type_in_tool_result_file_content() {
        let error = convert(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "call-123",
                "toolName": "image-generator",
                "output": {
                    "type": "content",
                    "value": [{
                        "type": "file",
                        "data": { "type": "data", "data": "base64data" },
                        "mediaType": "unsupported/mime-type",
                    }],
                },
            }],
        }]))
        .unwrap_err();

        assert_eq!(
            unsupported_functionality(error),
            "file mime type: unsupported/mime-type"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should fallback to stringified result when content is undefined
    #[test]
    fn should_fallback_to_stringified_result_when_content_is_undefined() {
        let result = convert(json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "call-123",
                "toolName": "calculator",
                "output": { "type": "json", "value": { "value": 42 } },
            }],
        }]))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": "call-123",
                        "content": [{ "text": "{\"value\":42}" }],
                    },
                }],
            })
        );
    }
}

mod additional_file_format_tests {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › additional file format tests › should throw an error for unsupported file mime type in user message content
    #[test]
    fn should_throw_an_error_for_unsupported_file_mime_type_in_user_message_content() {
        let error = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "base64data" },
                "mediaType": "application/rtf",
            }],
        }]))
        .unwrap_err();

        assert_eq!(
            unsupported_functionality(error),
            "file mime type: application/rtf"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › additional file format tests › should handle xlsx files correctly
    #[test]
    fn should_handle_xlsx_files_correctly() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "base64data" },
                "mediaType": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            }],
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "content": [{
                        "document": {
                            "format": "xlsx",
                            "name": "document-1",
                            "source": { "bytes": "base64data" },
                        },
                    }],
                    "role": "user",
                }],
                "system": [],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › additional file format tests › should handle docx files correctly
    #[test]
    fn should_handle_docx_files_correctly() {
        let result = convert(json!([{
            "role": "user",
            "content": [{
                "type": "file",
                "data": { "type": "data", "data": "base64data" },
                "mediaType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            }],
        }]))
        .unwrap();

        assert_eq!(
            result,
            json!({
                "messages": [{
                    "content": [{
                        "document": {
                            "format": "docx",
                            "name": "document-1",
                            "source": { "bytes": "base64data" },
                        },
                    }],
                    "role": "user",
                }],
                "system": [],
            })
        );
    }
}

mod mistral_tool_call_id_normalization {
    use super::*;

    fn calculator_result(tool_call_id: &str) -> Value {
        json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": tool_call_id,
                "toolName": "calculator",
                "output": { "type": "text", "value": "The result is 42" },
            }],
        }])
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › Mistral tool call ID normalization › should normalize tool call IDs in tool results when isMistral is true
    #[test]
    fn should_normalize_tool_call_ids_in_tool_results_when_is_mistral_is_true() {
        let result =
            convert_with(calculator_result("tooluse_bpe71yCfRu2b5i-nKGDr5g"), true).unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": "8eHypBDcw",
                        "content": [{ "text": "The result is 42" }],
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › Mistral tool call ID normalization › should normalize tool call IDs in tool calls when isMistral is true
    #[test]
    fn should_normalize_tool_call_ids_in_tool_calls_when_is_mistral_is_true() {
        let result = convert_with(
            json!([{
                "role": "assistant",
                "content": [{
                    "type": "tool-call",
                    "toolCallId": "tooluse_xyz123ABC456-def",
                    "toolName": "test-tool",
                    "input": { "query": "test" },
                }],
            }]),
            true,
        )
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "assistant",
                "content": [{
                    "toolUse": {
                        "toolUseId": "De61BW1Dz",
                        "name": "test-tool",
                        "input": { "query": "test" },
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › Mistral tool call ID normalization › should not normalize tool call IDs when isMistral is false
    #[test]
    fn should_not_normalize_tool_call_ids_when_is_mistral_is_false() {
        let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
        let result = convert_with(calculator_result(original_id), false).unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": original_id,
                        "content": [{ "text": "The result is 42" }],
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › Mistral tool call ID normalization › should default to not normalizing when isMistral is not provided
    #[test]
    fn should_default_to_not_normalizing_when_is_mistral_is_not_provided() {
        let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
        let result = convert(calculator_result(original_id)).unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "toolResult": {
                        "toolUseId": original_id,
                        "content": [{ "text": "The result is 42" }],
                    },
                }],
            })
        );
    }
}

mod top_level_only_media_type_resolution {
    use super::*;

    /// base64 of `[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]`.
    const PNG_BYTES: &str = "iVBORw0KGgo=";
    /// base64 of `[0x25, 0x50, 0x44, 0x46, 0x2d, 0x31, 0x2e, 0x34]`.
    const PDF_BYTES: &str = "JVBERi0xLjQ=";
    /// base64 of `[0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70]`.
    const MP4_BYTES: &str = "AAAAGGZ0eXA=";

    fn single_file(data: Value, media_type: &str) -> Value {
        json!([{
            "role": "user",
            "content": [{ "type": "file", "data": data, "mediaType": media_type }],
        }])
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › top-level-only mediaType resolution › should detect subtype from inline bytes when mediaType is top-level-only (image)
    #[test]
    fn should_detect_subtype_from_inline_bytes_when_media_type_is_top_level_only_image() {
        let result = convert(single_file(
            json!({ "type": "data", "data": PNG_BYTES }),
            "image",
        ))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{ "image": { "format": "png", "source": { "bytes": "iVBORw0KGgo=" } } }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › top-level-only mediaType resolution › should detect subtype from inline bytes when mediaType is top-level-only (video)
    #[test]
    fn should_detect_subtype_from_inline_bytes_when_media_type_is_top_level_only_video() {
        let result = convert(single_file(
            json!({ "type": "data", "data": MP4_BYTES }),
            "video",
        ))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{ "video": { "format": "mp4", "source": { "bytes": "AAAAGGZ0eXA=" } } }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › top-level-only mediaType resolution › should detect subtype from inline bytes when mediaType is top-level-only (application/pdf)
    #[test]
    fn should_detect_subtype_from_inline_bytes_when_media_type_is_top_level_only_pdf() {
        let result = convert(single_file(
            json!({ "type": "data", "data": PDF_BYTES }),
            "application",
        ))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "document": {
                        "format": "pdf",
                        "name": "document-1",
                        "source": { "bytes": "JVBERi0xLjQ=" },
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › top-level-only mediaType resolution › should route to document slot for non-image top-level type via getTopLevelMediaType
    #[test]
    fn should_route_to_document_slot_for_non_image_top_level_type() {
        let result = convert(single_file(
            json!({ "type": "data", "data": "base64data" }),
            "text/plain",
        ))
        .unwrap();

        assert_eq!(
            result["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "document": {
                        "format": "txt",
                        "name": "document-1",
                        "source": { "bytes": "base64data" },
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › top-level-only mediaType resolution › should throw UnsupportedFunctionalityError for URL data (File URL)
    #[test]
    fn should_throw_unsupported_functionality_error_for_url_data_file_url() {
        let error = convert(single_file(
            json!({ "type": "url", "url": "https://example.com/image.png" }),
            "image",
        ))
        .unwrap_err();

        assert_eq!(unsupported_functionality(error), "File URL data");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › top-level-only mediaType resolution › should throw UnsupportedFunctionalityError for unsupported full image mediaType
    #[test]
    fn should_throw_unsupported_functionality_error_for_unsupported_full_image_media_type() {
        let error = convert(single_file(
            json!({ "type": "data", "data": "base64data" }),
            "image/avif",
        ))
        .unwrap_err();

        assert_eq!(
            unsupported_functionality(error),
            "image mime type: image/avif"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › top-level-only mediaType resolution › should throw UnsupportedFunctionalityError when top-level-only bytes cannot be detected
    #[test]
    fn should_throw_unsupported_functionality_error_when_top_level_only_bytes_cannot_be_detected() {
        // base64 of `[0x00, 0x01, 0x02]`.
        let error = convert(single_file(
            json!({ "type": "data", "data": "AAEC" }),
            "image",
        ))
        .unwrap_err();

        let message = error.to_string();
        assert!(
            regex::Regex::new(r#"media type "image".*could not be auto-detected"#)
                .unwrap()
                .is_match(&message),
            "{message}"
        );
    }
}

// ── Older ports (previously `tests/bedrock_convert_test.rs`) ──
//
// Pure-function tests for the Amazon Bedrock provider's conversion layer.
//
// Translated from the TS test files:
// - `convert-to-amazon-bedrock-chat-messages.test.ts` (64 cases)
// - `amazon-bedrock-prepare-tools.test.ts` (23 cases incl. `it.each`)
// - `convert-amazon-bedrock-usage.test.ts` (9 cases)

mod bedrock_convert {
    use aimux_core::language_model_message::{
        AssistantPart, FilePart, LanguageModelMessage, ReasoningPart, TextPart, ToolCallPart,
        ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
    };
    use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions, provider_namespace};
    use serde_json::{Value, json};

    use crate::bedrock::convert_to_chat_messages::convert_prompt_to_bedrock;

    // ── prompt builders ─────────────────────────────────────────────────────────

    fn user(content: Vec<UserPart>) -> LanguageModelMessage {
        LanguageModelMessage::User {
            content,
            provider_options: None,
        }
    }

    fn assistant(content: Vec<AssistantPart>) -> LanguageModelMessage {
        LanguageModelMessage::Assistant {
            content,
            provider_options: None,
        }
    }

    fn system_msg(text: &str) -> LanguageModelMessage {
        LanguageModelMessage::System {
            content: text.to_string(),
            provider_options: None,
        }
    }

    fn tool_msg(content: Vec<ToolPart>) -> LanguageModelMessage {
        LanguageModelMessage::Tool {
            content,
            provider_options: None,
        }
    }

    fn user_text(text: &str) -> UserPart {
        UserPart::Text(TextPart {
            text: text.to_string(),
            provider_options: None,
        })
    }

    fn assistant_text(text: &str) -> AssistantPart {
        AssistantPart::Text(TextPart {
            text: text.to_string(),
            provider_options: None,
        })
    }

    /// A file part with inline base64 data, passed through verbatim.
    fn file_base64(
        data: &str,
        media_type: &str,
        filename: Option<&str>,
        provider_options: Option<SharedProviderOptions>,
    ) -> UserPart {
        UserPart::File(FilePart {
            data: FileData::Data {
                data: FileBytes::Base64(data.to_string()),
            },
            media_type: media_type.to_string(),
            filename: filename.map(std::string::ToString::to_string),
            provider_options,
        })
    }

    fn text_with_cache(text: &str, cache_type: &str, ttl: Option<&str>) -> TextPart {
        let mut cp = serde_json::Map::new();
        cp.insert("type".to_string(), json!(cache_type));
        if let Some(t) = ttl {
            cp.insert("ttl".to_string(), json!(t));
        }
        TextPart {
            text: text.to_string(),
            provider_options: Some(
                provider_namespace("bedrock", json!({ "cachePoint": Value::Object(cp) })).unwrap(),
            ),
        }
    }

    fn reasoning(text: &str, signature: Option<&str>) -> AssistantPart {
        AssistantPart::Reasoning(ReasoningPart {
            text: text.to_string(),
            provider_options: signature.map(|signature| {
                provider_namespace("amazonBedrock", json!({ "signature": signature })).unwrap()
            }),
        })
    }

    fn tool_call(id: &str, name: &str, input: Value) -> AssistantPart {
        AssistantPart::ToolCall(ToolCallPart {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            input,
            provider_executed: None,
            provider_options: None,
        })
    }

    fn tool_result(id: &str, output: ToolResultOutput) -> ToolPart {
        ToolPart::ToolResult(ToolResultPart {
            tool_call_id: id.to_string(),
            tool_name: "test".to_string(),
            output,
            provider_options: None,
        })
    }

    // ════════════════════════════════════════════════════════════════════════════
    // convert-to-amazon-bedrock-chat-messages
    // ════════════════════════════════════════════════════════════════════════════

    // ── system messages ─────────────────────────────────────────────────────────

    mod system_messages {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › system messages › should combine multiple leading system messages into a single system message
        #[test]
        fn system_combine_multiple_leading() {
            let (system, _) =
                convert_prompt_to_bedrock(&vec![system_msg("Hello"), system_msg("World")]).unwrap();
            assert_eq!(
                Value::Array(system),
                json!([{ "text": "Hello" }, { "text": "World" }])
            );
        }
    }

    mod user_messages {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should extract the system message
        #[test]
        fn system_extract_single() {
            let (system, _) = convert_prompt_to_bedrock(&vec![system_msg("Hello")]).unwrap();
            assert_eq!(Value::Array(system), json!([{ "text": "Hello" }]));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should convert messages with image parts
        #[test]
        fn user_convert_image_parts() {
            let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
                user_text("Hello"),
                file_base64("AAECAw==", "image/png", None, None),
            ])])
            .unwrap();
            assert_eq!(
                Value::Array(messages),
                json!([{
                    "role": "user",
                    "content": [
                        { "text": "Hello" },
                        { "image": { "format": "png", "source": { "bytes": "AAECAw==" } } },
                    ]
                }])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should use consistent document names for prompt cache effectiveness
        #[test]
        fn user_consistent_document_names() {
            let (_, messages) = convert_prompt_to_bedrock(&vec![
                user(vec![
                    file_base64("AAECAw==", "application/pdf", None, None),
                    file_base64("BAUGBw==", "application/pdf", None, None),
                ]),
                assistant(vec![assistant_text("OK")]),
                user(vec![file_base64("AAECAw==", "application/pdf", None, None)]),
            ])
            .unwrap();
            assert_eq!(
                Value::Array(messages),
                json!([
                    { "role": "user", "content": [
                        { "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" } } },
                        { "document": { "format": "pdf", "name": "document-2", "source": { "bytes": "BAUGBw==" } } },
                    ]},
                    { "role": "assistant", "content": [{ "text": "OK" }] },
                    { "role": "user", "content": [
                        { "document": { "format": "pdf", "name": "document-3", "source": { "bytes": "AAECAw==" } } },
                    ]},
                ])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › user messages › should add cache point to user content part when specified
        #[test]
        fn user_content_part_cache_point() {
            let (system, messages) = convert_prompt_to_bedrock(&vec![user(vec![
                user_text("Hello"),
                UserPart::Text(text_with_cache("cached", "default", Some("5m"))),
                user_text("World"),
            ])])
            .unwrap();
            assert!(system.is_empty());
            assert_eq!(
                Value::Array(messages),
                json!([{
                    "role": "user",
                    "content": [
                        { "text": "Hello" },
                        { "text": "cached" },
                        { "cachePoint": { "type": "default", "ttl": "5m" } },
                        { "text": "World" },
                    ]
                }])
            );
        }
    }

    mod tool_messages {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › tool messages › should convert tool result with content array containing text
        #[test]
        fn tool_result_content_text() {
            let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
                "call-123",
                ToolResultOutput::Content {
                    value: vec![ToolResultContent::Text(TextPart {
                        text: "The result is 42".to_string(),
                        provider_options: None,
                    })],
                },
            )])])
            .unwrap();
            assert_eq!(
                messages[0],
                json!({
                    "role": "user",
                    "content": [{
                        "toolResult": {
                            "toolUseId": "call-123",
                            "content": [{ "text": "The result is 42" }]
                        }
                    }]
                })
            );
        }
    }

    mod citations {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › citations › should handle citations enabled for PDF
        #[test]
        fn citations_enabled_for_pdf() {
            let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
                "AAECAw==",
                "application/pdf",
                None,
                Some(
                    provider_namespace("bedrock", json!({ "citations": { "enabled": true } }))
                        .unwrap(),
                ),
            )])])
            .unwrap();
            assert_eq!(
                messages[0]["content"][0],
                json!({
                    "document": {
                        "format": "pdf",
                        "name": "document-1",
                        "source": { "bytes": "AAECAw==" },
                        "citations": { "enabled": true }
                    }
                })
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › citations › should handle citations disabled for PDF
        #[test]
        fn citations_disabled_for_pdf() {
            let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
                "AAECAw==",
                "application/pdf",
                None,
                Some(
                    provider_namespace("bedrock", json!({ "citations": { "enabled": false } }))
                        .unwrap(),
                ),
            )])])
            .unwrap();
            assert_eq!(
                messages[0]["content"][0],
                json!({
                    "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" } }
                })
            );
            assert!(
                messages[0]["content"][0]["document"]
                    .get("citations")
                    .is_none()
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › citations › should handle no citations specified for PDF (default)
        #[test]
        fn citations_default_for_pdf() {
            let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
                "AAECAw==",
                "application/pdf",
                None,
                None,
            )])])
            .unwrap();
            assert_eq!(
                messages[0]["content"][0],
                json!({
                    "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" } }
                })
            );
            assert!(
                messages[0]["content"][0]["document"]
                    .get("citations")
                    .is_none()
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › citations › should handle multiple PDFs with different citation settings
        #[test]
        fn citations_multiple_pdfs() {
            let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
                file_base64(
                    "AAECAw==",
                    "application/pdf",
                    None,
                    Some(
                        provider_namespace("bedrock", json!({ "citations": { "enabled": true } }))
                            .unwrap(),
                    ),
                ),
                file_base64(
                    "BAUGBw==",
                    "application/pdf",
                    None,
                    Some(
                        provider_namespace("bedrock", json!({ "citations": { "enabled": false } }))
                            .unwrap(),
                    ),
                ),
            ])])
            .unwrap();
            assert_eq!(
                messages[0]["content"],
                json!([
                    { "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" }, "citations": { "enabled": true } } },
                    { "document": { "format": "pdf", "name": "document-2", "source": { "bytes": "BAUGBw==" } } },
                ])
            );
        }
    }

    mod top_level_only_media_type_resolution {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/convert-to-amazon-bedrock-chat-messages.test.ts › top-level-only mediaType resolution › should pass through a full image mediaType unchanged
        #[test]
        fn media_type_pass_through_full_image() {
            let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
                "iVBORw0KGgo=",
                "image/png",
                None,
                None,
            )])])
            .unwrap();
            assert_eq!(
                Value::Array(messages),
                json!([{
                    "role": "user",
                    "content": [{ "image": { "format": "png", "source": { "bytes": "iVBORw0KGgo=" } } }]
                }])
            );
        }
    }

    // ── user messages ───────────────────────────────────────────────────────────

    /// partial: `should convert messages with document parts`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn user_convert_document_parts() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
            user_text("Hello"),
            file_base64("AAECAw==", "application/pdf", None, None),
        ])])
        .unwrap();
        assert_eq!(
            messages[0]["content"],
            json!([
                { "text": "Hello" },
                { "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" } } },
            ])
        );
    }

    /// partial: `should strip file extension when filename is provided`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn user_strip_file_extension() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
            user_text("Hello"),
            file_base64(
                "AAECAw==",
                "application/pdf",
                Some("custom-filename.pdf"),
                None,
            ),
        ])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][1],
            json!({ "document": { "format": "pdf", "name": "custom-filename", "source": { "bytes": "AAECAw==" } } })
        );
    }

    /// partial: `should preserve filename without extension when provided`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn user_preserve_filename_without_extension() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
            "AAECAw==",
            "application/pdf",
            Some("custom-filename"),
            None,
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][0],
            json!({ "document": { "format": "pdf", "name": "custom-filename", "source": { "bytes": "AAECAw==" } } })
        );
    }

    // ── assistant messages ──────────────────────────────────────────────────────

    /// partial: `should remove trailing whitespace from last assistant message when there is no further user message`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_trim_trailing_whitespace_last() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("user content")]),
            assistant(vec![assistant_text("assistant content  ")]),
        ])
        .unwrap();
        assert_eq!(
            Value::Array(messages),
            json!([
                { "role": "user", "content": [{ "text": "user content" }] },
                { "role": "assistant", "content": [{ "text": "assistant content" }] },
            ])
        );
    }

    /// partial: `should remove trailing whitespace from last assistant message with multi-part content when there is no further user message`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_trim_trailing_whitespace_multi_part() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("user content")]),
            assistant(vec![
                assistant_text("assistant "),
                assistant_text("content  "),
            ]),
        ])
        .unwrap();
        assert_eq!(
            messages[1]["content"],
            json!([{ "text": "assistant " }, { "text": "content" }])
        );
    }

    /// partial: `should keep trailing whitespace from assistant message when there is a further user message`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_keep_trailing_whitespace_with_further_user() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("user content")]),
            assistant(vec![assistant_text("assistant content  ")]),
            user(vec![user_text("user content 2")]),
        ])
        .unwrap();
        assert_eq!(
            messages[1]["content"],
            json!([{ "text": "assistant content  " }])
        );
    }

    /// partial: `should combine multiple sequential assistant messages into a single message`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_combine_sequential() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Hi!")]),
            assistant(vec![assistant_text("Hello")]),
            assistant(vec![assistant_text("World")]),
            assistant(vec![assistant_text("!")]),
        ])
        .unwrap();
        assert_eq!(
            Value::Array(messages),
            json!([
                { "role": "user", "content": [{ "text": "Hi!" }] },
                { "role": "assistant", "content": [{ "text": "Hello" }, { "text": "World" }, { "text": "!" }] },
            ])
        );
    }

    /// partial: `should add cache point to assistant content part when specified`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_content_part_cache_point() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![assistant(vec![
            assistant_text("Hello"),
            AssistantPart::Text(text_with_cache("cached", "default", Some("1h"))),
            assistant_text("World"),
        ])])
        .unwrap();
        assert_eq!(
            Value::Array(messages),
            json!([{
                "role": "assistant",
                "content": [
                    { "text": "Hello" },
                    { "text": "cached" },
                    { "cachePoint": { "type": "default", "ttl": "1h" } },
                    { "text": "World" },
                ]
            }])
        );
    }

    /// partial: `should properly convert reasoning content type`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_reasoning_with_signature() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Explain your reasoning")]),
            assistant(vec![reasoning(
                "This is my step-by-step reasoning process",
                Some("test-signature"),
            )]),
        ])
        .unwrap();
        assert_eq!(
            messages[1]["content"],
            json!([{
                "reasoningContent": {
                    "reasoningText": {
                        "text": "This is my step-by-step reasoning process",
                        "signature": "test-signature"
                    }
                }
            }])
        );
    }

    /// partial: `should preserve assistant message reasoning parts with amazonBedrock providerOptions`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_preserve_amazon_bedrock_reasoning() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Explain your reasoning")]),
            assistant(vec![
                reasoning(
                    "Bedrock-signed reasoning round-tripped to Bedrock",
                    Some("bedrock-signature"),
                ),
                assistant_text("final answer"),
            ]),
        ])
        .unwrap();
        assert_eq!(
            messages[1]["content"],
            json!([
                { "reasoningContent": { "reasoningText": {
                    "text": "Bedrock-signed reasoning round-tripped to Bedrock",
                    "signature": "bedrock-signature"
                }}},
                { "text": "final answer" },
            ])
        );
    }

    /// partial: `should not trim reasoning text when a signature is present`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_no_trim_reasoning_with_signature() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Explain your reasoning")]),
            assistant(vec![reasoning(
                "This is my reasoning with trailing space    ",
                Some("test-signature"),
            )]),
        ])
        .unwrap();
        assert_eq!(
            messages[1]["content"][0]["reasoningContent"]["reasoningText"]["text"],
            json!("This is my reasoning with trailing space    ")
        );
    }

    /// partial: `should omit reasoning content without signature`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_omit_reasoning_without_signature() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Explain your reasoning")]),
            assistant(vec![
                reasoning("This is my reasoning with trailing space    ", None),
                assistant_text("final answer"),
            ]),
        ])
        .unwrap();
        assert_eq!(messages[1]["content"], json!([{ "text": "final answer" }]));
    }

    /// partial: `should omit multiple reasoning parts without signatures`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_omit_multiple_reasoning_without_signatures() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Explain your reasoning")]),
            assistant(vec![
                reasoning("First reasoning with trailing space    ", None),
                reasoning("Second reasoning with trailing space    ", None),
                assistant_text("final answer"),
            ]),
        ])
        .unwrap();
        assert_eq!(messages[1]["content"], json!([{ "text": "final answer" }]));
    }

    /// partial: `should omit unsigned reasoning while preserving tool calls in multi-turn tool use`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_omit_unsigned_reasoning_preserving_tool_calls() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("What is the weather?")]),
            assistant(vec![
                reasoning("I should call the weather tool.", None),
                tool_call("call-1", "getWeather", json!({ "city": "SF" })),
            ]),
            tool_msg(vec![tool_result(
                "call-1",
                ToolResultOutput::Text {
                    value: "Sunny, 72F".to_string(),
                    provider_options: None,
                },
            )]),
        ])
        .unwrap();
        assert_eq!(
            Value::Array(messages),
            json!([
                { "role": "user", "content": [{ "text": "What is the weather?" }] },
                { "role": "assistant", "content": [{
                    "toolUse": { "input": { "city": "SF" }, "name": "getWeather", "toolUseId": "call-1" }
                }]},
                { "role": "user", "content": [{
                    "toolResult": { "toolUseId": "call-1", "content": [{ "text": "Sunny, 72F" }] }
                }]},
            ])
        );
    }

    /// partial: `should preserve reasoning text with signature in multi-turn tool use`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_preserve_reasoning_multi_turn() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("What is the weather?")]),
            assistant(vec![
                reasoning("Let me check the weather API.\n", Some("sig-abc123")),
                tool_call("call-1", "getWeather", json!({ "city": "SF" })),
            ]),
            tool_msg(vec![tool_result(
                "call-1",
                ToolResultOutput::Text {
                    value: "Sunny, 72F".to_string(),
                    provider_options: None,
                },
            )]),
            assistant(vec![
                reasoning("The weather is sunny and warm.\n", Some("sig-def456")),
                assistant_text("It is sunny and 72F in SF."),
            ]),
        ])
        .unwrap();
        assert_eq!(
            Value::Array(messages),
            json!([
                { "role": "user", "content": [{ "text": "What is the weather?" }] },
                { "role": "assistant", "content": [
                    { "reasoningContent": { "reasoningText": {
                        "text": "Let me check the weather API.\n", "signature": "sig-abc123"
                    }}},
                    { "toolUse": { "input": { "city": "SF" }, "name": "getWeather", "toolUseId": "call-1" } },
                ]},
                { "role": "user", "content": [{
                    "toolResult": { "toolUseId": "call-1", "content": [{ "text": "Sunny, 72F" }] }
                }]},
                { "role": "assistant", "content": [
                    { "reasoningContent": { "reasoningText": {
                        "text": "The weather is sunny and warm.\n", "signature": "sig-def456"
                    }}},
                    { "text": "It is sunny and 72F in SF." },
                ]},
            ])
        );
    }

    /// partial: `should handle a mix of text and reasoning content types`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_mix_text_and_reasoning() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Explain your reasoning")]),
            assistant(vec![
                assistant_text("My answer is 42."),
                reasoning(
                    "I calculated this by analyzing the meaning of life",
                    Some("reasoning-process"),
                ),
            ]),
        ])
        .unwrap();
        assert_eq!(
            messages[1]["content"],
            json!([
                { "text": "My answer is 42." },
                { "reasoningContent": { "reasoningText": {
                    "text": "I calculated this by analyzing the meaning of life",
                    "signature": "reasoning-process"
                }}},
            ])
        );
    }

    /// partial: `should filter out empty text blocks in assistant messages`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_filter_empty_text_blocks() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Hello")]),
            assistant(vec![
                assistant_text("\n\n"),
                tool_call("call-123", "test", json!({})),
                assistant_text("  "),
                assistant_text("actual content"),
            ]),
        ])
        .unwrap();
        assert_eq!(
            messages[1]["content"],
            json!([
                { "toolUse": { "toolUseId": "call-123", "name": "test", "input": {} } },
                { "text": "actual content" },
            ])
        );
    }

    /// partial: `should wrap non-object (invalid) tool call input in an object`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_wrap_non_object_tool_input() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![assistant(vec![tool_call(
            "call-1",
            "cityAttractions",
            Value::String("{ \"city\": \"San Francisco\", }".to_string()),
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"],
            json!([{
                "toolUse": {
                    "toolUseId": "call-1",
                    "name": "cityAttractions",
                    "input": { "rawInvalidInput": "{ \"city\": \"San Francisco\", }" }
                }
            }])
        );
    }

    /// partial: `should strip invalid characters from tool call names`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_strip_invalid_tool_name_chars() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![assistant(vec![
            tool_call("call-1", "$READFILE", json!({})),
            tool_call(
                "call-2",
                "exchange_delivered_order_items<|channel|>",
                json!({}),
            ),
            tool_call("call-3", "$", json!({})),
        ])])
        .unwrap();
        assert_eq!(
            messages[0]["content"],
            json!([
                { "toolUse": { "toolUseId": "call-1", "name": "READFILE", "input": {} } },
                { "toolUse": { "toolUseId": "call-2", "name": "exchange_delivered_order_itemschannel", "input": {} } },
                { "toolUse": { "toolUseId": "call-3", "name": "_", "input": {} } },
            ])
        );
    }

    /// partial: `should preserve empty text blocks when reasoning blocks are present`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn assistant_preserve_empty_text_with_reasoning() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![
            user(vec![user_text("Hello")]),
            assistant(vec![
                reasoning("thinking...", Some("sig-1")),
                assistant_text(""),
                reasoning("more thinking...", Some("sig-2")),
                assistant_text("response text"),
                tool_call("call-123", "test", json!({})),
            ]),
        ])
        .unwrap();
        assert_eq!(
            messages[1]["content"],
            json!([
                { "reasoningContent": { "reasoningText": { "text": "thinking...", "signature": "sig-1" } } },
                { "text": "" },
                { "reasoningContent": { "reasoningText": { "text": "more thinking...", "signature": "sig-2" } } },
                { "text": "response text" },
                { "toolUse": { "toolUseId": "call-123", "name": "test", "input": {} } },
            ])
        );
    }

    // ── tool messages ───────────────────────────────────────────────────────────

    /// partial: `should convert tool result with content array containing image`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn tool_result_content_image() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
            "call-123",
            ToolResultOutput::Content {
                value: vec![ToolResultContent::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64("base64data".to_string()),
                    },
                    media_type: "image/jpeg".to_string(),
                    filename: None,
                    provider_options: None,
                })],
            },
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][0],
            json!({
                "toolResult": {
                    "toolUseId": "call-123",
                    "content": [{ "image": { "format": "jpeg", "source": { "bytes": "base64data" } } }]
                }
            })
        );
    }

    /// partial: `should convert tool result with content array containing PDF`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn tool_result_content_pdf() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
            "call-123",
            ToolResultOutput::Content {
                value: vec![ToolResultContent::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64("base64data".to_string()),
                    },
                    media_type: "application/pdf".to_string(),
                    filename: Some("tool-result.pdf".to_string()),
                    provider_options: None,
                })],
            },
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][0],
            json!({
                "toolResult": {
                    "toolUseId": "call-123",
                    "content": [{
                        "document": { "format": "pdf", "name": "tool-result", "source": { "bytes": "base64data" } }
                    }]
                }
            })
        );
    }

    // NOT PORTED (TS: "should throw error for unsupported image format in tool result
    // content" and "should throw error for unsupported mime type in tool result
    // file content").

    /// partial: `should fallback to stringified result when content is undefined`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    /// (json output)
    #[test]
    fn tool_result_json_output() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
            "call-123",
            ToolResultOutput::Json {
                value: json!({ "value": 42 }),
                provider_options: None,
            },
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][0],
            json!({
                "toolResult": {
                    "toolUseId": "call-123",
                    "content": [{ "text": "{\"value\":42}" }]
                }
            })
        );
    }

    // ── citations ───────────────────────────────────────────────────────────────

    // ── additional file format tests ────────────────────────────────────────────

    /// partial: `should handle xlsx files correctly`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn file_format_xlsx() {
        let (system, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
            "base64data",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            None,
            None,
        )])])
        .unwrap();
        assert!(system.is_empty());
        assert_eq!(
            messages[0]["content"][0],
            json!({ "document": { "format": "xlsx", "name": "document-1", "source": { "bytes": "base64data" } } })
        );
    }

    /// partial: `should handle docx files correctly`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn file_format_docx() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
            "base64data",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            None,
            None,
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][0],
            json!({ "document": { "format": "docx", "name": "document-1", "source": { "bytes": "base64data" } } })
        );
    }

    // ── Mistral tool call ID normalization ──────────────────────────────────────

    /// partial: `should not normalize tool call IDs when isMistral is false`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn mistral_no_normalize_when_false() {
        let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
        let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
            original_id,
            ToolResultOutput::Text {
                value: "The result is 42".to_string(),
                provider_options: None,
            },
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][0]["toolResult"]["toolUseId"],
            json!(original_id)
        );
    }

    /// partial: `should default to not normalizing when isMistral is not provided`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn mistral_default_no_normalize() {
        let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
        let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
            original_id,
            ToolResultOutput::Text {
                value: "The result is 42".to_string(),
                provider_options: None,
            },
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][0]["toolResult"]["toolUseId"],
            json!(original_id)
        );
    }

    // ── top-level-only mediaType resolution ─────────────────────────────────────

    /// partial: `should route to document slot for non-image top-level type via getTopLevelMediaType`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn media_type_route_to_document_text_plain() {
        let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
            "base64data",
            "text/plain",
            None,
            None,
        )])])
        .unwrap();
        assert_eq!(
            messages[0]["content"][0],
            json!({ "document": { "format": "txt", "name": "document-1", "source": { "bytes": "base64data" } } })
        );
    }
}

// ── Older ports (previously `tests/bedrock_remaining_test.rs`) ──
//
// Remaining Amazon Bedrock upstream tests, restored from the tool-types branch.
//
// Pinned translation sources (`amazon-bedrock/src/`):
// - `normalize-tool-call-id.test.ts`
// - `convert-amazon-bedrock-usage.test.ts` (raw echo)
// - `amazon-bedrock-provider.test.ts` (factory/auth variants)
// - `convert-to-amazon-bedrock-chat-messages.test.ts` (Mistral IDs)
// - `amazon-bedrock-chat-language-model.test.ts` (request/response cases)

mod bedrock_remaining {
    use crate::bedrock::tests::remaining_support::*;

    use serde_json::{Value, json};

    use aimux_core::language_model_message::LanguageModelPrompt;

    use crate::bedrock::chat_language_model::build_request_body_checked;

    /// partial: `should normalize tool call IDs in tool results when isMistral is true`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn convert_mistral_normalize_tool_result_id() {
        let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
        let prompt = vec![tool_msg(vec![tool_result(
            original_id.to_string(),
            json!({ "type": "text", "value": "The result is 42" }),
        )])];
        let mut opts = default_options(prompt);
        opts.tools = Some(vec![test_tool().into()]);
        let (body, _, _, _) =
            build_request_body_checked("mistral.mistral-large-2402-v1:0", &opts, None).unwrap();
        let messages = &body["messages"];
        assert_eq!(
            messages[0]["content"][0]["toolResult"]["toolUseId"],
            json!("8eHypBDcw")
        );
    }

    /// partial: `should normalize tool call IDs in tool calls when isMistral is true`; full port in src/bedrock/tests/convert_to_chat_messages.rs.
    #[test]
    fn convert_mistral_normalize_tool_call_id() {
        let original_id = "tooluse_xyz123ABC456-def";
        let prompt = vec![assistant_msg(vec![tool_call(
            original_id.to_string(),
            "test-tool".to_string(),
            json!({ "query": "test" }),
        )])];
        let mut opts = default_options(prompt);
        opts.tools = Some(vec![test_tool().into()]);
        let (body, _, _, _) =
            build_request_body_checked("mistral.mistral-large-2402-v1:0", &opts, None).unwrap();
        let messages = &body["messages"];
        assert_eq!(
            messages[0]["content"][0]["toolUse"]["toolUseId"],
            json!("De61BW1Dz")
        );
    }

    /// TS: "should throw for file parts with provider references" and "should
    /// throw UnsupportedFunctionalityError for URL data (File URL)" — the
    /// functionality names of convert-to-amazon-bedrock-chat-messages.ts:238-290
    /// and :612-650. A URL is checked for `s3:` before its media type.
    #[test]
    fn convert_unsupported_file_functionality_names() {
        let unsupported = |content: Value| {
            let prompt: LanguageModelPrompt = serde_json::from_value(content).unwrap();
            match crate::bedrock::convert_to_chat_messages::convert_prompt_to_bedrock(&prompt) {
                Err(aimux_core::AiMuxError::UnsupportedFunctionality(functionality)) => {
                    functionality
                }
                other => panic!("expected UnsupportedFunctionality, got {other:?}"),
            }
        };
        let user_file = |data: Value, media_type: &str| json!([{"role": "user", "content": [{"type": "file", "data": data, "mediaType": media_type}]}]);
        let tool_file = |data: Value, media_type: &str| {
            json!([{"role": "tool", "content": [{
                "type": "tool-result", "toolCallId": "call-1", "toolName": "t",
                "output": {"type": "content", "value": [{"type": "file", "data": data, "mediaType": media_type}]}
            }]}])
        };

        assert_eq!(
            unsupported(user_file(
                json!({"type": "reference", "reference": {"bedrock": "file-ref-123"}}),
                "image/png"
            )),
            "file parts with provider references"
        );
        assert_eq!(
            unsupported(user_file(
                json!({"type": "url", "url": "https://example.com/image.png"}),
                "image"
            )),
            "File URL data"
        );
        assert_eq!(
            unsupported(user_file(
                json!({"type": "url", "url": "s3://bucket/file.pdf"}),
                "application/pdf"
            )),
            "File URL data"
        );
        assert_eq!(
            unsupported(tool_file(
                json!({"type": "url", "url": "https://example.com/image.png"}),
                "image"
            )),
            "tool result file data of type \"url\""
        );
        assert_eq!(
            unsupported(tool_file(
                json!({"type": "url", "url": "s3://bucket/file.pdf"}),
                "application/pdf"
            )),
            "tool result file data of type \"url\""
        );
        assert_eq!(
            unsupported(tool_file(
                json!({"type": "text", "text": "hi"}),
                "text/plain"
            )),
            "tool result file data of type \"text\""
        );
    }
}
