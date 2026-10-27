use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::Resolvable;

use crate::bedrock::{AmazonBedrockProviderSettings, BedrockModel, create_amazon_bedrock};
use crate::test_support::mock_fetch::{Canned, MockFetch};

/// `TEST_PROMPT`: one user text part.
fn test_options() -> CallOptions {
    serde_json::from_value(json!({
        "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }],
        "includeRawChunks": false,
    }))
    .expect("valid call options")
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xEDB8_8320
            };
        }
    }
    !crc
}

/// `codec.encode({ headers, body })` of `@smithy/eventstream-codec`, with
/// string headers.
fn encode(headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut header_bytes = Vec::new();
    for (name, value) in headers {
        header_bytes.push(u8::try_from(name.len()).expect("short header name"));
        header_bytes.extend_from_slice(name.as_bytes());
        header_bytes.push(7);
        header_bytes.extend_from_slice(
            &u16::try_from(value.len())
                .expect("short value")
                .to_be_bytes(),
        );
        header_bytes.extend_from_slice(value.as_bytes());
    }
    let total = u32::try_from(16 + header_bytes.len() + body.len()).expect("small frame");
    let headers_length = u32::try_from(header_bytes.len()).expect("small headers");
    let mut frame = Vec::new();
    frame.extend_from_slice(&total.to_be_bytes());
    frame.extend_from_slice(&headers_length.to_be_bytes());
    let prelude_crc = crc32(&frame);
    frame.extend_from_slice(&prelude_crc.to_be_bytes());
    frame.extend_from_slice(&header_bytes);
    frame.extend_from_slice(body);
    let message_crc = crc32(&frame);
    frame.extend_from_slice(&message_crc.to_be_bytes());
    frame
}

/// `createEvent(eventType, data)`.
fn create_event(event_type: &str, data: &str) -> Vec<u8> {
    encode(
        &[(":message-type", "event"), (":event-type", event_type)],
        data.as_bytes(),
    )
}

/// A model whose every request is answered with `frames` as an
/// `application/vnd.amazon.eventstream` body.
fn model_streaming(model_id: &str, frames: &[Vec<u8>]) -> BedrockModel {
    let mock = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![(
            "content-type".into(),
            "application/vnd.amazon.eventstream".into(),
        )],
        body: frames.concat(),
    }]);
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".into())),
        region: Some("us-east-1".into()),
        base_url: Some("https://bedrock-runtime.us-east-1.amazonaws.com".into()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("valid settings")
    .chat(model_id)
}

/// `convertReadableStreamToArray(stream)`, keeping what the stream threw.
async fn stream_items(model: &BedrockModel) -> Vec<Result<StreamPart, AiMuxError>> {
    model
        .do_stream(&test_options())
        .await
        .expect("stream starts")
        .stream
        .collect()
        .await
}

/// The parts of a stream that must not throw, as AI SDK JSON.
async fn stream_parts(model: &BedrockModel) -> Vec<Value> {
    stream_items(model)
        .await
        .into_iter()
        .map(|item| serde_json::to_value(item.expect("stream does not throw")).unwrap())
        .collect()
}

/// `rejects.toThrow(message)`: the stream throws an error whose message
/// contains `message`.
fn assert_throws(items: &[Result<StreamPart, AiMuxError>], message: &str) {
    let error = items
        .iter()
        .find_map(|item| item.as_ref().err())
        .unwrap_or_else(|| panic!("stream did not throw; expected {message:?}"));
    assert!(
        error.to_string().contains(message),
        "expected the error to contain {message:?}, got {error}"
    );
}

/// `expect.objectContaining(expected)`: every expected key is present and
/// equal.
fn object_containing(actual: &Value, expected: &Value) -> bool {
    expected
        .as_object()
        .expect("expected object")
        .iter()
        .all(|(key, value)| actual.get(key) == Some(value))
}

/// `expect(parts).toEqual(expect.arrayContaining(expected))` of
/// `objectContaining` elements.
fn assert_contains_all(parts: &[Value], expected: &[Value]) {
    for wanted in expected {
        assert!(
            parts.iter().any(|part| object_containing(part, wanted)),
            "no part matches {wanted} in {parts:?}"
        );
    }
}

/// `expect(actual).toMatchObject(expected)`.
fn assert_match_object(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            for (key, value) in expected {
                let found = actual
                    .get(key)
                    .unwrap_or_else(|| panic!("missing {key}; expected {value}"));
                assert_match_object(found, value);
            }
        }
        (Value::Array(actual), Value::Array(expected)) => {
            assert_eq!(actual.len(), expected.len());
            for (found, value) in actual.iter().zip(expected) {
                assert_match_object(found, value);
            }
        }
        _ => assert_eq!(actual, expected),
    }
}

mod amazon_bedrock_chat_language_model_do_stream {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-event-stream.test.ts › AmazonBedrockChatLanguageModel doStream › surfaces event stream decoding failures
    #[tokio::test]
    async fn surfaces_event_stream_decoding_failures() {
        let mut corrupted_frame = create_event("contentBlockDelta", "corrupted");
        let last = corrupted_frame.len() - 1;
        corrupted_frame[last] ^= 0xff;
        let model = model_streaming(
            "anthropic.claude-3-haiku-20240307-v1:0",
            &[
                corrupted_frame,
                create_event(
                    "contentBlockDelta",
                    &json!({ "contentBlockIndex": 0, "delta": { "text": "later" } }).to_string(),
                ),
            ],
        );

        assert_throws(&stream_items(&model).await, "The message checksum");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-event-stream.test.ts › AmazonBedrockChatLanguageModel doStream › rejects a truncated event stream frame at EOF
    #[tokio::test]
    async fn rejects_a_truncated_event_stream_frame_at_eof() {
        let text_frame = create_event(
            "contentBlockDelta",
            &json!({ "contentBlockIndex": 0, "delta": { "text": "partial result" } }).to_string(),
        );
        let message_stop_frame = create_event(
            "messageStop",
            &json!({ "stopReason": "end_turn" }).to_string(),
        );
        let truncated = message_stop_frame[..message_stop_frame.len() - 1].to_vec();
        let model = model_streaming(
            "anthropic.claude-3-haiku-20240307-v1:0",
            &[text_frame, truncated],
        );

        assert_throws(
            &stream_items(&model).await,
            "Incomplete Amazon Bedrock event-stream frame",
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-event-stream.test.ts › AmazonBedrockChatLanguageModel doStream › allows EOF at a complete frame boundary without messageStop
    #[tokio::test]
    async fn allows_eof_at_a_complete_frame_boundary_without_message_stop() {
        let model = model_streaming(
            "anthropic.claude-3-haiku-20240307-v1:0",
            &[create_event(
                "contentBlockDelta",
                &json!({ "contentBlockIndex": 0, "delta": { "text": "complete frame" } })
                    .to_string(),
            )],
        );

        assert_contains_all(
            &stream_parts(&model).await,
            &[
                json!({ "type": "text-delta", "delta": "complete frame" }),
                json!({ "type": "finish", "finishReason": { "unified": "other" } }),
            ],
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-event-stream.test.ts › AmazonBedrockChatLanguageModel doStream › surfaces $exceptionType frames as stream errors
    #[tokio::test]
    async fn surfaces_exception_type_frames_as_stream_errors() {
        for (exception_type, status_code, is_retryable) in [
            ("internalServerException", 500, true),
            ("modelStreamErrorException", 424, true),
            ("serviceUnavailableException", 503, true),
            ("throttlingException", 429, true),
            ("validationException", 400, false),
        ] {
            let exception = json!({ "message": format!("Modeled exception: {exception_type}") });
            let exception_frame = encode(
                &[
                    (":message-type", "exception"),
                    (":exception-type", exception_type),
                    (":content-type", "application/json"),
                ],
                exception.to_string().as_bytes(),
            );
            let model =
                model_streaming("anthropic.claude-3-haiku-20240307-v1:0", &[exception_frame]);

            let parts: Vec<StreamPart> = stream_items(&model)
                .await
                .into_iter()
                .map(|item| item.expect("stream does not throw"))
                .collect();
            let error_part = &parts[parts.len() - 2];

            let StreamPart::Error { error } = error_part else {
                panic!("Expected an error part, got {error_part:?}");
            };
            // `isProviderStreamError`: the provider's typed stream error.
            let AiMuxError::ApiCall(error) = error else {
                panic!("expected a provider stream error, got {error:?}");
            };
            assert_match_object(
                &json!({
                    "message": error.message,
                    "type": error.provider_code,
                    "statusCode": error.status_code,
                    "isRetryable": error.is_retryable,
                    "data": error.data,
                }),
                &json!({
                    "message": exception["message"],
                    "type": exception_type,
                    "statusCode": status_code,
                    "isRetryable": is_retryable,
                    "data": { exception_type: exception },
                }),
            );
            assert_match_object(
                &serde_json::to_value(&parts[parts.len() - 1]).unwrap(),
                &json!({ "type": "finish", "finishReason": { "unified": "error" } }),
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-event-stream.test.ts › AmazonBedrockChatLanguageModel doStream › streams reasoning redacted as `redactedContent` for replay
    #[tokio::test]
    async fn streams_reasoning_redacted_as_redacted_content_for_replay() {
        let model = model_streaming(
            "us.openai.gpt-5.6-luna",
            &[
                create_event(
                    "contentBlockDelta",
                    &json!({
                        "contentBlockIndex": 0,
                        "delta": { "reasoningContent": { "redactedContent": "encrypted-reasoning-" } },
                    })
                    .to_string(),
                ),
                create_event(
                    "contentBlockDelta",
                    &json!({
                        "contentBlockIndex": 0,
                        "delta": { "reasoningContent": { "redactedContent": "payload" } },
                    })
                    .to_string(),
                ),
                create_event(
                    "contentBlockStop",
                    &json!({ "contentBlockIndex": 0 }).to_string(),
                ),
                create_event(
                    "contentBlockDelta",
                    &json!({ "contentBlockIndex": 1, "delta": { "text": "The answer is 42." } })
                        .to_string(),
                ),
                create_event(
                    "contentBlockStop",
                    &json!({ "contentBlockIndex": 1 }).to_string(),
                ),
                create_event("messageStop", &json!({ "stopReason": "end_turn" }).to_string()),
            ],
        );

        let parts = stream_parts(&model).await;

        let errors: Vec<&Value> = parts
            .iter()
            .filter(|part| part["type"] == json!("error"))
            .collect();
        assert_eq!(errors, Vec::<&Value>::new());
        assert_contains_all(
            &parts,
            &[
                json!({ "type": "reasoning-start", "id": "0" }),
                json!({
                    "type": "reasoning-end",
                    "id": "0",
                    "providerMetadata": {
                        "amazonBedrock": { "redactedContent": "encrypted-reasoning-payload" },
                        "bedrock": { "redactedContent": "encrypted-reasoning-payload" },
                    },
                }),
                json!({ "type": "text-delta", "delta": "The answer is 42." }),
            ],
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-event-stream.test.ts › AmazonBedrockChatLanguageModel doStream › keeps multiple redacted reasoning blocks separate
    #[tokio::test]
    async fn keeps_multiple_redacted_reasoning_blocks_separate() {
        let model = model_streaming(
            "us.openai.gpt-5.6-luna",
            &[
                create_event(
                    "contentBlockDelta",
                    &json!({
                        "contentBlockIndex": 0,
                        "delta": { "reasoningContent": { "redactedContent": "first-payload" } },
                    })
                    .to_string(),
                ),
                create_event(
                    "contentBlockStop",
                    &json!({ "contentBlockIndex": 0 }).to_string(),
                ),
                create_event(
                    "contentBlockDelta",
                    &json!({
                        "contentBlockIndex": 1,
                        "delta": { "reasoningContent": { "redactedContent": "second-payload" } },
                    })
                    .to_string(),
                ),
                create_event(
                    "contentBlockStop",
                    &json!({ "contentBlockIndex": 1 }).to_string(),
                ),
                create_event(
                    "contentBlockDelta",
                    &json!({ "contentBlockIndex": 2, "delta": { "text": "The answer is 42." } })
                        .to_string(),
                ),
                create_event(
                    "contentBlockStop",
                    &json!({ "contentBlockIndex": 2 }).to_string(),
                ),
                create_event(
                    "messageStop",
                    &json!({ "stopReason": "end_turn" }).to_string(),
                ),
            ],
        );

        let parts = stream_parts(&model).await;

        let errors: Vec<&Value> = parts
            .iter()
            .filter(|part| part["type"] == json!("error"))
            .collect();
        assert_eq!(errors, Vec::<&Value>::new());
        let reasoning_ends: Vec<&Value> = parts
            .iter()
            .filter(|part| part["type"] == json!("reasoning-end"))
            .collect();
        assert_eq!(
            reasoning_ends,
            vec![
                &json!({
                    "type": "reasoning-end",
                    "id": "0",
                    "providerMetadata": {
                        "amazonBedrock": { "redactedContent": "first-payload" },
                        "bedrock": { "redactedContent": "first-payload" },
                    },
                }),
                &json!({
                    "type": "reasoning-end",
                    "id": "1",
                    "providerMetadata": {
                        "amazonBedrock": { "redactedContent": "second-payload" },
                        "bedrock": { "redactedContent": "second-payload" },
                    },
                }),
            ]
        );
    }
}
