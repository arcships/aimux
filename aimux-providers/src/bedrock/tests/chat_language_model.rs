use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::StreamResult;
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt, Resolvable,
};

use crate::bedrock::{
    AmazonBedrockChatModelFamily, AmazonBedrockChatModelSettings, AmazonBedrockProvider,
    AmazonBedrockProviderSettings, BedrockModel, create_amazon_bedrock,
};
use crate::test_support::mock_fetch::{Canned, MockFetch, RouteFetch, Seen};

/// `modelId` of the upstream test file.
const MODEL_ID: &str = "anthropic.claude-3-haiku-20240307-v1:0";
/// `baseUrl` of the upstream test file.
const BASE_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";

/// `TEST_PROMPT`: a system message and one user text part.
fn test_prompt() -> Value {
    json!([
        { "role": "system", "content": "System Prompt" },
        { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
    ])
}

/// Call options from their AI SDK JSON (`LanguageModelV4CallOptions`).
fn call_options(options: Value) -> CallOptions {
    serde_json::from_value(options).expect("valid call options")
}

/// `{ prompt: TEST_PROMPT, ...extra }`.
fn options_with(extra: Value) -> CallOptions {
    let mut options = json!({ "prompt": test_prompt() });
    if let (Some(target), Value::Object(extra)) = (options.as_object_mut(), extra) {
        target.extend(extra);
    }
    call_options(options)
}

// ── AWS event-stream frames ──────────────────────────────────────────────────

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

/// One AWS event-stream frame with string headers (the `EventStreamCodec.encode`
/// of `@smithy/eventstream-codec`).
fn encode_frame(headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
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

/// The frame the Converse stream carries for one upstream chunk
/// `{ <type>: <payload> }`: an `event` frame, or an `exception` frame for an
/// `…Exception` type (the decoder wraps either payload under its type again).
fn chunk_frame(chunk: &Value) -> Vec<u8> {
    let (kind, payload) = chunk
        .as_object()
        .and_then(|object| object.iter().next())
        .expect("a chunk is a one-key object");
    let body = serde_json::to_vec(payload).expect("serializable payload");
    if kind.ends_with("Exception") {
        encode_frame(
            &[
                (":message-type", "exception"),
                (":exception-type", kind),
                (":content-type", "application/json"),
            ],
            &body,
        )
    } else {
        encode_frame(
            &[
                (":message-type", "event"),
                (":event-type", kind),
                (":content-type", "application/json"),
            ],
            &body,
        )
    }
}

/// The event-stream body of a list of upstream chunks.
fn event_stream_body(chunks: &[Value]) -> Vec<u8> {
    chunks.iter().flat_map(chunk_frame).collect()
}

/// The chunks of a `__fixtures__/*.chunks.txt` file (one JSON chunk per line).
fn fixture_chunks(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture chunk is JSON"))
        .collect()
}

// ── Mock responses and transports ────────────────────────────────────────────

/// A `createTestServer` `stream-chunks` response: its default headers, then
/// `headers`.
fn stream_response(body: Vec<u8>, headers: &[(&str, &str)]) -> Canned {
    let mut all: Vec<(String, String)> = vec![
        ("content-type".into(), "text/event-stream".into()),
        ("cache-control".into(), "no-cache".into()),
        ("connection".into(), "keep-alive".into()),
    ];
    for (name, value) in headers {
        all.push(((*name).into(), (*value).into()));
    }
    Canned {
        status: 200,
        headers: all,
        body,
    }
}

/// A `stream-chunks` response carrying `chunks` as event-stream frames.
fn stream_chunks(chunks: &[Value]) -> Canned {
    stream_response(event_stream_body(chunks), &[])
}

/// `injectFetchHeaders`: a transport that sets `headers` on every request
/// (over the ones already there) before handing it to `inner`.
struct InjectFetchHeaders {
    headers: Vec<(&'static str, &'static str)>,
    inner: FetchFunction,
}

#[async_trait]
impl Fetch for InjectFetchHeaders {
    async fn fetch(&self, mut request: FetchRequest) -> Result<FetchResponse, FetchError> {
        for (name, value) in &self.headers {
            request.headers.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        self.inner.fetch(request).await
    }
}

fn inject_fetch_headers(
    headers: Vec<(&'static str, &'static str)>,
    inner: FetchFunction,
) -> FetchFunction {
    Arc::new(InjectFetchHeaders { headers, inner })
}

/// `fakeFetchWithAuth`: `injectFetchHeaders({ 'x-amz-auth': 'test-auth' })`
/// over the mock server.
fn fake_fetch_with_auth(mock: &Arc<MockFetch>) -> FetchFunction {
    inject_fetch_headers(vec![("x-amz-auth", "test-auth")], mock.transport())
}

/// A provider on `BASE_URL` whose requests go through `fetch`. An API key
/// stands in for the upstream model config's absent signing layer, so
/// nothing is signed.
fn provider(fetch: FetchFunction, headers: Option<HeaderMapOpt>) -> AmazonBedrockProvider {
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".into())),
        region: Some("us-east-1".into()),
        base_url: Some(BASE_URL.into()),
        headers,
        fetch: Some(fetch),
        ..Default::default()
    })
    .expect("valid settings")
}

/// The chat model `model_id` over `fetch`.
fn chat_model(model_id: &str, fetch: FetchFunction) -> BedrockModel {
    provider(fetch, None).chat(model_id)
}

/// The upstream `model`: `MODEL_ID` over `fakeFetchWithAuth`.
fn model(mock: &Arc<MockFetch>) -> BedrockModel {
    chat_model(MODEL_ID, fake_fetch_with_auth(mock))
}

/// The JSON body of the `index`th request the mock saw.
fn request_body(seen: &[Seen], index: usize) -> Value {
    seen.get(index).expect("request was sent").json_body()
}

// ── Stream output ────────────────────────────────────────────────────────────

/// `convertReadableStreamToArray`, keeping errors the stream threw.
async fn stream_items(result: StreamResult) -> Vec<Result<StreamPart, AiMuxError>> {
    result.stream.collect().await
}

/// Every part of a stream that must not throw.
async fn stream_parts(result: StreamResult) -> Vec<StreamPart> {
    stream_items(result)
        .await
        .into_iter()
        .map(|item| item.expect("stream does not throw"))
        .collect()
}

/// The upstream form of an error part's error: a provider stream error
/// (`message`, `type`, `statusCode`, `isRetryable`, `data`) for an API call
/// error, `{ message }` for anything else.
fn error_json(error: &AiMuxError) -> Value {
    match error {
        AiMuxError::ApiCall(error) => {
            let mut object = serde_json::Map::new();
            object.insert("message".into(), json!(error.message));
            if let Some(kind) = &error.provider_code {
                object.insert("type".into(), json!(kind));
            }
            if let Some(status) = error.status_code {
                object.insert("statusCode".into(), json!(status));
            }
            object.insert("isRetryable".into(), json!(error.is_retryable));
            if let Some(data) = &error.data {
                object.insert("data".into(), data.clone());
            }
            Value::Object(object)
        }
        other => json!({ "message": other.to_string() }),
    }
}

/// A stream part as the AI SDK JSON the upstream snapshots print.
fn part_json(part: &StreamPart) -> Value {
    match part {
        StreamPart::Error { error } => json!({ "type": "error", "error": error_json(error) }),
        other => serde_json::to_value(other).expect("serializable part"),
    }
}

fn parts_json(parts: &[StreamPart]) -> Value {
    Value::Array(parts.iter().map(part_json).collect())
}

/// The first part of type `kind`, as JSON.
fn find_part(parts: &[StreamPart], kind: &str) -> Option<Value> {
    parts
        .iter()
        .map(part_json)
        .find(|part| part["type"] == json!(kind))
}

/// `expect(actual).toMatchObject(expected)`: every key of an expected object
/// is present and matches; arrays match element by element.
fn assert_match_object(actual: &Value, expected: &Value) {
    fn check(actual: &Value, expected: &Value, path: &str) {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => {
                for (key, value) in expected {
                    let at = format!("{path}.{key}");
                    let Some(found) = actual.get(key) else {
                        panic!("missing {at}; expected {value}");
                    };
                    check(found, value, &at);
                }
            }
            (Value::Array(actual), Value::Array(expected)) => {
                assert_eq!(actual.len(), expected.len(), "length of {path}");
                for (index, (found, value)) in actual.iter().zip(expected).enumerate() {
                    check(found, value, &format!("{path}[{index}]"));
                }
            }
            _ => assert_eq!(actual, expected, "at {path}"),
        }
    }
    check(actual, expected, "$");
}

mod supported_urls {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › supportedUrls › should support S3 URLs for image and video parts
    #[tokio::test]
    async fn should_support_s3_urls_for_image_and_video_parts() {
        let mock = MockFetch::new(vec![]);
        let urls = model(&mock).supported_urls().await;

        let patterns: std::collections::BTreeMap<&str, Vec<&str>> = urls
            .0
            .iter()
            .map(|(media_type, patterns)| {
                (
                    media_type.as_str(),
                    patterns.iter().map(regex::Regex::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            patterns,
            [("image/*", vec!["^s3://"]), ("video/*", vec!["^s3://"])]
                .into_iter()
                .collect()
        );
    }
}

mod application_inference_profile_reasoning {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › application inference profile reasoning › returns reasoning for an Anthropic application inference profile ARN when budgetTokens is configured
    #[tokio::test]
    async fn returns_reasoning_for_an_anthropic_application_inference_profile_arn() {
        let application_profile_arn =
            "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/custom-profile";
        let fetch = RouteFetch::new(|seen| {
            let request_body = seen.json_body();
            let content = if request_body["additionalModelRequestFields"]["thinking"].is_null() {
                json!([{ "text": "OK" }])
            } else {
                json!([
                    {
                        "reasoningContent": {
                            "reasoningText": {
                                "text": "The response should be OK.",
                                "signature": "test-signature",
                            },
                        },
                    },
                    { "text": "OK" },
                ])
            };
            Canned::json(&json!({
                "output": { "message": { "role": "assistant", "content": content } },
                "stopReason": "end_turn",
                "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 },
            }))
        });
        let application_profile_model = chat_model(application_profile_arn, fetch.transport());

        let result = application_profile_model
            .do_generate(&options_with(json!({
                "maxOutputTokens": 1100,
                "providerOptions": {
                    "bedrock": {
                        "reasoningConfig": { "type": "enabled", "budgetTokens": 1024 },
                    },
                },
            })))
            .await
            .expect("generate succeeds");

        let content = serde_json::to_value(&result.content).expect("serializable content");
        assert!(
            content
                .as_array()
                .expect("content array")
                .iter()
                .any(|part| {
                    part["type"] == json!("reasoning")
                        && part["text"] == json!("The response should be OK.")
                        && part["providerMetadata"]["amazonBedrock"]
                            == json!({ "signature": "test-signature" })
                }),
            "no reasoning part in {content}"
        );
        let request_body = request_body(&fetch.seen(), 0);
        assert_eq!(
            request_body["additionalModelRequestFields"]["thinking"],
            json!({ "type": "enabled", "budget_tokens": 1024 })
        );
        let warnings = serde_json::to_value(&result.warnings).expect("serializable warnings");
        assert!(
            !warnings
                .as_array()
                .expect("warnings array")
                .iter()
                .any(|warning| {
                    warning["type"] == json!("unsupported")
                        && warning["feature"] == json!("budgetTokens")
                }),
            "unexpected budgetTokens warning in {warnings}"
        );
    }
}

mod application_inference_profile_structured_output {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › application inference profile structured output › uses native structured output in %s mode when the Anthropic family is declared
    #[tokio::test]
    async fn uses_native_structured_output_in_mode_when_the_anthropic_family_is_declared() {
        for structured_output_mode in ["outputFormat", "auto"] {
            let application_profile_arn = "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/custom-profile";
            let fetch = RouteFetch::new(|_| {
                Canned::json(&json!({
                    "output": {
                        "message": {
                            "role": "assistant",
                            "content": [{ "text": "{\"name\":\"Test\"}" }],
                        },
                    },
                    "stopReason": "end_turn",
                    "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 },
                }))
            });
            let application_profile_model = provider(fetch.transport(), None).chat_with_settings(
                application_profile_arn,
                &AmazonBedrockChatModelSettings {
                    model_family: Some(AmazonBedrockChatModelFamily::Anthropic),
                },
            );

            application_profile_model
                .do_generate(&options_with(json!({
                    "responseFormat": {
                        "type": "json",
                        "schema": {
                            "type": "object",
                            "properties": { "name": { "type": "string" } },
                            "required": ["name"],
                        },
                    },
                    "providerOptions": {
                        "amazonBedrock": { "structuredOutputMode": structured_output_mode },
                    },
                })))
                .await
                .expect("generate succeeds");

            let request_body = request_body(&fetch.seen(), 0);
            assert_eq!(
                request_body.get("toolConfig"),
                None,
                "{structured_output_mode}"
            );
            assert_eq!(
                request_body["additionalModelRequestFields"]["output_config"]["format"],
                json!({
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": { "name": { "type": "string" } },
                        "required": ["name"],
                    },
                }),
                "{structured_output_mode}"
            );
        }
    }
}

mod do_generate_request_metadata {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate request metadata › should return the request body
    #[tokio::test]
    async fn should_return_the_request_body() {
        let fixture: Value =
            serde_json::from_str(include_str!("fixtures/amazon-bedrock-text.json")).unwrap();
        let mock = MockFetch::new(vec![Canned::json(&fixture)]);

        let result = model(&mock)
            .do_generate(&options_with(json!({})))
            .await
            .expect("generate succeeds");

        assert_eq!(
            result.request.and_then(|request| request.body),
            Some(json!({
                "additionalModelResponseFieldPaths": [
                    "/delta/stop_sequence"
                ],
                "messages": [
                    {
                        "content": [
                            {
                                "text": "Hello"
                            }
                        ],
                        "role": "user"
                    }
                ],
                "system": [
                    {
                        "text": "System Prompt"
                    }
                ]
            }))
        );
    }
}

mod request_url {
    use super::*;

    mod arn_model_ids_containing_a_slash {
        use super::*;

        const INFERENCE_PROFILE_ARN: &str =
            "arn:aws:bedrock:eu-west-1:474668406012:inference-profile/eu.amazon.nova-lite-v1:0";

        /// `encodeURIComponent(inferenceProfileArn)`.
        const ENCODED_INFERENCE_PROFILE_ARN: &str = "arn%3Aaws%3Abedrock%3Aeu-west-1%3A474668406012%3Ainference-profile%2Feu.amazon.nova-lite-v1%3A0";

        /// `createInferenceProfileModel`: answers the encoded Converse routes
        /// and an `UnknownOperationException` body anywhere else.
        fn create_inference_profile_model() -> BedrockModel {
            let fetch = RouteFetch::new(|seen| {
                let url = &seen.url;
                let is_encoded_route = url.contains(ENCODED_INFERENCE_PROFILE_ARN);
                let unknown_operation_response = json!({
                    "Output": { "__type": "com.amazon.coral.service#UnknownOperationException" },
                    "Version": "1.0",
                });
                let converse_response = json!({
                    "output": { "message": { "role": "assistant", "content": [{ "text": "Hello!" }] } },
                    "stopReason": "end_turn",
                    "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 },
                });
                let converse_stream_response = [
                    json!({ "messageStart": { "role": "assistant" } }),
                    json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello!" } } }),
                    json!({ "contentBlockStop": { "contentBlockIndex": 0 } }),
                    json!({ "messageStop": { "stopReason": "end_turn" } }),
                    json!({ "metadata": { "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 } } }),
                ];
                if is_encoded_route && url.ends_with("/converse-stream") {
                    Canned {
                        status: 200,
                        headers: vec![(
                            "content-type".into(),
                            "application/vnd.amazon.eventstream".into(),
                        )],
                        body: event_stream_body(&converse_stream_response),
                    }
                } else if is_encoded_route {
                    Canned::json(&converse_response)
                } else {
                    Canned::json(&unknown_operation_response)
                }
            });
            chat_model(INFERENCE_PROFILE_ARN, fetch.transport())
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › request URL › ARN model IDs containing a slash › should generate text through the encoded Converse route
        #[tokio::test]
        async fn should_generate_text_through_the_encoded_converse_route() {
            let result = create_inference_profile_model()
                .do_generate(&options_with(json!({})))
                .await
                .expect("generate succeeds");

            assert_eq!(
                serde_json::to_value(&result.content).unwrap(),
                json!([
                    {
                        "text": "Hello!",
                        "type": "text"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › request URL › ARN model IDs containing a slash › should stream text through the encoded Converse route
        #[tokio::test]
        async fn should_stream_text_through_the_encoded_converse_route() {
            let result = create_inference_profile_model()
                .do_stream(&options_with(json!({ "includeRawChunks": false })))
                .await
                .expect("stream starts");

            assert_eq!(
                parts_json(&stream_parts(result).await),
                json!([
                    {
                        "type": "stream-start",
                        "warnings": []
                    },
                    {
                        "modelId": "arn:aws:bedrock:eu-west-1:474668406012:inference-profile/eu.amazon.nova-lite-v1:0",
                        "type": "response-metadata"
                    },
                    {
                        "id": "0",
                        "type": "text-start"
                    },
                    {
                        "delta": "Hello!",
                        "id": "0",
                        "type": "text-delta"
                    },
                    {
                        "id": "0",
                        "type": "text-end"
                    },
                    {
                        "finishReason": {
                            "raw": "end_turn",
                            "unified": "stop"
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "cacheWrite": 0,
                                "noCache": 1,
                                "total": 1
                            },
                            "outputTokens": {
                                "text": 1,
                                "total": 1
                            },
                            "raw": {
                                "inputTokens": 1,
                                "outputTokens": 1,
                                "totalTokens": 2
                            }
                        }
                    }
                ])
            );
        }
    }
}

mod do_stream {
    use super::*;

    mod text {
        use super::*;

        fn text_fixture_response(headers: &[(&str, &str)]) -> Canned {
            stream_response(
                event_stream_body(&fixture_chunks(include_str!(
                    "fixtures/amazon-bedrock-text.chunks.txt"
                ))),
                headers,
            )
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › text › should stream text deltas
        #[tokio::test]
        async fn should_stream_text_deltas() {
            let mock = MockFetch::new(vec![text_fixture_response(&[])]);

            let result = model(&mock)
                .do_stream(&options_with(json!({ "includeRawChunks": false })))
                .await
                .expect("stream starts");

            let expected: Value = serde_json::from_str(include_str!(
                "snapshots/do_stream_text_should_stream_text_deltas.json"
            ))
            .unwrap();
            assert_eq!(parts_json(&stream_parts(result).await), expected);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › text › should expose the raw response headers
        #[tokio::test]
        async fn should_expose_the_raw_response_headers() {
            let mock = MockFetch::new(vec![text_fixture_response(&[(
                "test-header",
                "test-value",
            )])]);

            let result = model(&mock)
                .do_stream(&options_with(json!({ "includeRawChunks": false })))
                .await
                .expect("stream starts");

            assert_eq!(
                serde_json::to_value(result.response.and_then(|response| response.headers))
                    .unwrap(),
                json!({
                    "cache-control": "no-cache",
                    "connection": "keep-alive",
                    "content-type": "text/event-stream",
                    "test-header": "test-value"
                })
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › text › should return the request body
        #[tokio::test]
        async fn should_return_the_request_body() {
            let mock = MockFetch::new(vec![text_fixture_response(&[])]);

            let result = model(&mock)
                .do_stream(&options_with(json!({ "includeRawChunks": false })))
                .await
                .expect("stream starts");

            assert_eq!(
                result.request.and_then(|request| request.body),
                Some(json!({
                    "additionalModelResponseFieldPaths": [
                        "/delta/stop_sequence"
                    ],
                    "messages": [
                        {
                            "content": [
                                {
                                    "text": "Hello"
                                }
                            ],
                            "role": "user"
                        }
                    ],
                    "system": [
                        {
                            "text": "System Prompt"
                        }
                    ]
                }))
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should accept citation deltas
    #[tokio::test]
    async fn should_accept_citation_deltas() {
        let mock = MockFetch::new(vec![stream_chunks(&[json!({
            "contentBlockDelta": {
                "contentBlockIndex": 1,
                "delta": {
                    "citation": {
                        "location": {
                            "documentPage": { "documentIndex": 0, "start": 1, "end": 2 },
                        },
                        "sourceContent": [{ "text": "Source content" }],
                        "title": "document",
                    },
                },
            },
        })])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        let parts = stream_parts(result).await;
        let errors: Vec<Value> = parts
            .iter()
            .map(part_json)
            .filter(|part| part["type"] == json!("error"))
            .collect();
        assert_eq!(errors, Vec::<Value>::new());
    }

    mod reasoning {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › reasoning › should stream reasoning and text parts
        #[tokio::test]
        async fn should_stream_reasoning_and_text_parts() {
            let mock = MockFetch::new(vec![stream_chunks(&fixture_chunks(include_str!(
                "fixtures/amazon-bedrock-reasoning.chunks.txt"
            )))]);

            let result = model(&mock)
                .do_stream(&options_with(json!({ "includeRawChunks": false })))
                .await
                .expect("stream starts");

            let expected: Value = serde_json::from_str(include_str!(
                "snapshots/do_stream_reasoning_should_stream_reasoning_and_text_parts.json"
            ))
            .unwrap();
            assert_eq!(parts_json(&stream_parts(result).await), expected);
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should stream text deltas with metadata and usage
    #[tokio::test]
    async fn should_stream_text_deltas_with_metadata_and_usage() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 1, "delta": { "text": ", " } } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 2, "delta": { "text": "World!" } } }),
            json!({
                "metadata": {
                    "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
                    "metrics": { "latencyMs": 10 },
                },
            }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        assert_eq!(
            parts_json(&stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Hello",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "1",
                    "type": "text-start"
                },
                {
                    "delta": ", ",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "id": "2",
                    "type": "text-start"
                },
                {
                    "delta": "World!",
                    "id": "2",
                    "type": "text-delta"
                },
                {
                    "finishReason": {
                        "raw": "stop_sequence",
                        "unified": "stop"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "cacheWrite": 0,
                            "noCache": 4,
                            "total": 4
                        },
                        "outputTokens": {
                            "text": 34,
                            "total": 34
                        },
                        "raw": {
                            "inputTokens": 4,
                            "outputTokens": 34,
                            "totalTokens": 38
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should stream tool deltas
    #[tokio::test]
    async fn should_stream_tool_deltas() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({
                "contentBlockStart": {
                    "contentBlockIndex": 0,
                    "start": { "toolUse": { "toolUseId": "tool-use-id", "name": "test-tool" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "toolUse": { "input": "{\"value\":" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "toolUse": { "input": "\"Sparkle Day\"}" } },
                },
            }),
            json!({ "contentBlockStop": { "contentBlockIndex": 0 } }),
            json!({ "messageStop": { "stopReason": "tool_use" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({
                "tools": [
                    {
                        "type": "function",
                        "name": "test-tool",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "value": { "type": "string" } },
                            "required": ["value"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    },
                ],
                "toolChoice": { "type": "tool", "toolName": "test-tool" },
                "includeRawChunks": false,
            })))
            .await
            .expect("stream starts");

        assert_eq!(
            parts_json(&stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "tool-use-id",
                    "toolName": "test-tool",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"value\":",
                    "id": "tool-use-id",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\"Sparkle Day\"}",
                    "id": "tool-use-id",
                    "type": "tool-input-delta"
                },
                {
                    "id": "tool-use-id",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"value\":\"Sparkle Day\"}",
                    "toolCallId": "tool-use-id",
                    "toolName": "test-tool",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "tool-calls"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should stream parallel tool calls
    #[tokio::test]
    async fn should_stream_parallel_tool_calls() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({
                "contentBlockStart": {
                    "contentBlockIndex": 0,
                    "start": { "toolUse": { "toolUseId": "tool-use-id-1", "name": "test-tool-1" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "toolUse": { "input": "{\"value1\":" } },
                },
            }),
            json!({
                "contentBlockStart": {
                    "contentBlockIndex": 1,
                    "start": { "toolUse": { "toolUseId": "tool-use-id-2", "name": "test-tool-2" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 1,
                    "delta": { "toolUse": { "input": "{\"value2\":" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 1,
                    "delta": { "toolUse": { "input": "\"Sparkle Day\"}" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "toolUse": { "input": "\"Sparkle Day\"}" } },
                },
            }),
            json!({ "contentBlockStop": { "contentBlockIndex": 0 } }),
            json!({ "contentBlockStop": { "contentBlockIndex": 1 } }),
            json!({ "messageStop": { "stopReason": "tool_use" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({
                "tools": [
                    {
                        "type": "function",
                        "name": "test-tool-1",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "value1": { "type": "string" } },
                            "required": ["value"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    },
                    {
                        "type": "function",
                        "name": "test-tool-2",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "value2": { "type": "string" } },
                            "required": ["value"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    },
                ],
                "toolChoice": { "type": "tool", "toolName": "test-tool" },
                "includeRawChunks": false,
            })))
            .await
            .expect("stream starts");

        assert_eq!(
            parts_json(&stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "tool-use-id-1",
                    "toolName": "test-tool-1",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"value1\":",
                    "id": "tool-use-id-1",
                    "type": "tool-input-delta"
                },
                {
                    "id": "tool-use-id-2",
                    "toolName": "test-tool-2",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"value2\":",
                    "id": "tool-use-id-2",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\"Sparkle Day\"}",
                    "id": "tool-use-id-2",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\"Sparkle Day\"}",
                    "id": "tool-use-id-1",
                    "type": "tool-input-delta"
                },
                {
                    "id": "tool-use-id-1",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"value1\":\"Sparkle Day\"}",
                    "toolCallId": "tool-use-id-1",
                    "toolName": "test-tool-1",
                    "type": "tool-call"
                },
                {
                    "id": "tool-use-id-2",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"value2\":\"Sparkle Day\"}",
                    "toolCallId": "tool-use-id-2",
                    "toolName": "test-tool-2",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "tool-calls"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// The parts of a stream that carries one `{ <type>: <exception> }` chunk.
    async fn exception_stream(kind: &str, message: &str, name: &str) -> Value {
        let mock = MockFetch::new(vec![stream_chunks(&[json!({
            kind: { "message": message, "name": name, "$fault": "server", "$metadata": {} },
        })])]);
        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");
        parts_json(&stream_parts(result).await)
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle error stream parts
    #[tokio::test]
    async fn should_handle_error_stream_parts() {
        assert_eq!(
            exception_stream(
                "internalServerException",
                "Internal Server Error",
                "InternalServerException"
            )
            .await,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "error": {
                        "data": {
                            "internalServerException": {
                                "$fault": "server",
                                "$metadata": {},
                                "message": "Internal Server Error",
                                "name": "InternalServerException"
                            }
                        },
                        "isRetryable": true,
                        "message": "Internal Server Error",
                        "statusCode": 500,
                        "type": "internalServerException"
                    },
                    "type": "error"
                },
                {
                    "finishReason": {
                        "unified": "error"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle modelStreamErrorException error
    #[tokio::test]
    async fn should_handle_model_stream_error_exception_error() {
        assert_eq!(
            exception_stream(
                "modelStreamErrorException",
                "Model Stream Error",
                "ModelStreamErrorException"
            )
            .await,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "error": {
                        "data": {
                            "modelStreamErrorException": {
                                "$fault": "server",
                                "$metadata": {},
                                "message": "Model Stream Error",
                                "name": "ModelStreamErrorException"
                            }
                        },
                        "isRetryable": true,
                        "message": "Model Stream Error",
                        "statusCode": 424,
                        "type": "modelStreamErrorException"
                    },
                    "type": "error"
                },
                {
                    "finishReason": {
                        "unified": "error"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle throttlingException error
    #[tokio::test]
    async fn should_handle_throttling_exception_error() {
        assert_eq!(
            exception_stream(
                "throttlingException",
                "Throttling Error",
                "ThrottlingException"
            )
            .await,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "error": {
                        "data": {
                            "throttlingException": {
                                "$fault": "server",
                                "$metadata": {},
                                "message": "Throttling Error",
                                "name": "ThrottlingException"
                            }
                        },
                        "isRetryable": true,
                        "message": "Throttling Error",
                        "statusCode": 429,
                        "type": "throttlingException"
                    },
                    "type": "error"
                },
                {
                    "finishReason": {
                        "unified": "error"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle validationException error
    #[tokio::test]
    async fn should_handle_validation_exception_error() {
        assert_eq!(
            exception_stream(
                "validationException",
                "Validation Error",
                "ValidationException"
            )
            .await,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "error": {
                        "data": {
                            "validationException": {
                                "$fault": "server",
                                "$metadata": {},
                                "message": "Validation Error",
                                "name": "ValidationException"
                            }
                        },
                        "isRetryable": false,
                        "message": "Validation Error",
                        "statusCode": 400,
                        "type": "validationException"
                    },
                    "type": "error"
                },
                {
                    "finishReason": {
                        "unified": "error"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle failed chunk parsing
    ///
    /// Upstream mocks the response handler to emit a failed parse result
    /// carrying an injected error value (`{ message: 'Chunk Parsing Failed' }`).
    /// Rust has no seam for that value, so the chunk that fails to parse is a
    /// frame whose payload is not JSON; the parts are compared with the error
    /// value left out of both sides.
    #[tokio::test]
    async fn should_handle_failed_chunk_parsing() {
        let body = encode_frame(
            &[
                (":message-type", "event"),
                (":event-type", "contentBlockDelta"),
                (":content-type", "application/json"),
            ],
            b"Chunk Parsing Failed",
        );
        let mock = MockFetch::new(vec![stream_response(body, &[])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        let mut actual = parts_json(&stream_parts(result).await);
        let mut expected = json!([
            {
                "type": "stream-start",
                "warnings": []
            },
            {
                "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                "type": "response-metadata"
            },
            {
                "error": {
                    "message": "Chunk Parsing Failed"
                },
                "type": "error"
            },
            {
                "finishReason": {
                    "unified": "error"
                },
                "type": "finish",
                "usage": {
                    "inputTokens": {},
                    "outputTokens": {}
                }
            }
        ]);
        for parts in [&mut actual, &mut expected] {
            for part in parts.as_array_mut().expect("parts array") {
                if part["type"] == json!("error") {
                    part.as_object_mut().unwrap().remove("error");
                }
            }
        }
        assert_eq!(actual, expected);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should pass the messages and the model
    #[tokio::test]
    async fn should_pass_the_messages_and_the_model() {
        let mock = MockFetch::new(vec![stream_chunks(&[])]);

        model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        assert_eq!(
            request_body(&mock.seen(), 0),
            json!({
                "additionalModelResponseFieldPaths": [
                    "/delta/stop_sequence"
                ],
                "messages": [
                    {
                        "content": [
                            {
                                "text": "Hello"
                            }
                        ],
                        "role": "user"
                    }
                ],
                "system": [
                    {
                        "text": "System Prompt"
                    }
                ]
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should support guardrails
    #[tokio::test]
    async fn should_support_guardrails() {
        let mock = MockFetch::new(vec![stream_chunks(&[])]);

        model(&mock)
            .do_stream(&options_with(json!({
                "includeRawChunks": false,
                "providerOptions": {
                    "bedrock": {
                        "guardrailConfig": {
                            "guardrailIdentifier": "-1",
                            "guardrailVersion": "1",
                            "trace": "enabled",
                            "streamProcessingMode": "async",
                        },
                    },
                },
            })))
            .await
            .expect("stream starts");

        assert_match_object(
            &request_body(&mock.seen(), 0),
            &json!({
                "messages": [{ "role": "user", "content": [{ "text": "Hello" }] }],
                "system": [{ "text": "System Prompt" }],
                "guardrailConfig": {
                    "guardrailIdentifier": "-1",
                    "guardrailVersion": "1",
                    "trace": "enabled",
                    "streamProcessingMode": "async",
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should accept the new `amazonBedrock` providerOptions key
    #[tokio::test]
    async fn should_accept_the_new_amazon_bedrock_provider_options_key() {
        let mock = MockFetch::new(vec![stream_chunks(&[])]);

        model(&mock)
            .do_stream(&options_with(json!({
                "includeRawChunks": false,
                "providerOptions": {
                    "amazonBedrock": {
                        "guardrailConfig": {
                            "guardrailIdentifier": "-1",
                            "guardrailVersion": "1",
                            "trace": "enabled",
                            "streamProcessingMode": "async",
                        },
                    },
                },
            })))
            .await
            .expect("stream starts");

        assert_match_object(
            &request_body(&mock.seen(), 0),
            &json!({
                "guardrailConfig": {
                    "guardrailIdentifier": "-1",
                    "guardrailVersion": "1",
                    "trace": "enabled",
                    "streamProcessingMode": "async",
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should prefer `amazonBedrock` providerOptions over the legacy `bedrock` key
    #[tokio::test]
    async fn should_prefer_amazon_bedrock_provider_options_over_the_legacy_bedrock_key() {
        let mock = MockFetch::new(vec![stream_chunks(&[])]);

        model(&mock)
            .do_stream(&options_with(json!({
                "includeRawChunks": false,
                "providerOptions": {
                    "bedrock": {
                        "guardrailConfig": { "guardrailIdentifier": "legacy", "guardrailVersion": "1" },
                    },
                    "amazonBedrock": {
                        "guardrailConfig": { "guardrailIdentifier": "preferred", "guardrailVersion": "2" },
                    },
                },
            })))
            .await
            .expect("stream starts");

        assert_match_object(
            &request_body(&mock.seen(), 0),
            &json!({
                "guardrailConfig": { "guardrailIdentifier": "preferred", "guardrailVersion": "2" },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should send text parts as guardContent blocks in streaming requests
    #[tokio::test]
    async fn should_send_text_parts_as_guard_content_blocks_in_streaming_requests() {
        let mock = MockFetch::new(vec![stream_chunks(&[])]);

        model(&mock)
            .do_stream(&call_options(json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "What is the capital of Japan?",
                                "providerOptions": {
                                    "bedrock": {
                                        "guardContent": true,
                                        "guardContentQualifiers": ["query"],
                                    },
                                },
                            },
                        ],
                    },
                ],
                "includeRawChunks": false,
                "providerOptions": {
                    "bedrock": {
                        "guardrailConfig": {
                            "guardrailIdentifier": "-1",
                            "guardrailVersion": "1",
                            "trace": "enabled",
                        },
                    },
                },
            })))
            .await
            .expect("stream starts");

        assert_match_object(
            &request_body(&mock.seen(), 0),
            &json!({
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "guardContent": {
                                    "text": {
                                        "text": "What is the capital of Japan?",
                                        "qualifiers": ["query"],
                                    },
                                },
                            },
                        ],
                    },
                ],
                "guardrailConfig": {
                    "guardrailIdentifier": "-1",
                    "guardrailVersion": "1",
                    "trace": "enabled",
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should map guardrail_intervened messageStop to a content-filter finish reason in streaming
    #[tokio::test]
    async fn should_map_guardrail_intervened_message_stop_to_a_content_filter_finish_reason() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "text": "Sorry, the model cannot answer this question." },
                },
            }),
            json!({
                "metadata": {
                    "usage": { "inputTokens": 18, "outputTokens": 12, "totalTokens": 30 },
                    "metrics": { "latencyMs": 412 },
                    "trace": {
                        "guardrail": {
                            "inputAssessment": {
                                "1abcd2ef34gh": {
                                    "topicPolicy": {
                                        "topics": [
                                            { "action": "BLOCKED", "name": "Restricted topic", "type": "DENY" },
                                        ],
                                    },
                                },
                            },
                        },
                    },
                },
            }),
            json!({ "messageStop": { "stopReason": "guardrail_intervened" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&call_options(json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Tell me something restricted.",
                                "providerOptions": {
                                    "bedrock": {
                                        "guardContent": true,
                                        "guardContentQualifiers": ["query"],
                                    },
                                },
                            },
                        ],
                    },
                ],
                "includeRawChunks": false,
            })))
            .await
            .expect("stream starts");

        let finish = find_part(&stream_parts(result).await, "finish").expect("finish part");
        assert_eq!(
            finish["finishReason"],
            json!({
                "raw": "guardrail_intervened",
                "unified": "content-filter"
            })
        );
        assert_eq!(
            finish["providerMetadata"]["bedrock"]["trace"],
            json!({
                "guardrail": {
                    "inputAssessment": {
                        "1abcd2ef34gh": {
                            "topicPolicy": {
                                "topics": [
                                    {
                                        "action": "BLOCKED",
                                        "name": "Restricted topic",
                                        "type": "DENY"
                                    }
                                ]
                            }
                        }
                    }
                }
            })
        );
    }

    /// `mockTrace` of the upstream test file.
    fn mock_trace() -> Value {
        json!({
            "guardrail": {
                "inputAssessment": {
                    "1abcd2ef34gh": {
                        "contentPolicy": {
                            "filters": [{ "action": "BLOCKED", "confidence": "LOW", "type": "INSULTS" }],
                        },
                        "wordPolicy": {
                            "managedWordLists": [
                                { "action": "BLOCKED", "match": "<rude word>", "type": "PROFANITY" },
                            ],
                        },
                    },
                },
            },
        })
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include trace information in providerMetadata
    #[tokio::test]
    async fn should_include_trace_information_in_provider_metadata() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
            json!({
                "metadata": {
                    "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
                    "metrics": { "latencyMs": 10 },
                    "trace": mock_trace(),
                },
            }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        assert_eq!(
            parts_json(&stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Hello",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "finishReason": {
                        "raw": "stop_sequence",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "trace": {
                                "guardrail": {
                                    "inputAssessment": {
                                        "1abcd2ef34gh": {
                                            "contentPolicy": {
                                                "filters": [
                                                    {
                                                        "action": "BLOCKED",
                                                        "confidence": "LOW",
                                                        "type": "INSULTS"
                                                    }
                                                ]
                                            },
                                            "wordPolicy": {
                                                "managedWordLists": [
                                                    {
                                                        "action": "BLOCKED",
                                                        "match": "<rude word>",
                                                        "type": "PROFANITY"
                                                    }
                                                ]
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        "bedrock": {
                            "trace": {
                                "guardrail": {
                                    "inputAssessment": {
                                        "1abcd2ef34gh": {
                                            "contentPolicy": {
                                                "filters": [
                                                    {
                                                        "action": "BLOCKED",
                                                        "confidence": "LOW",
                                                        "type": "INSULTS"
                                                    }
                                                ]
                                            },
                                            "wordPolicy": {
                                                "managedWordLists": [
                                                    {
                                                        "action": "BLOCKED",
                                                        "match": "<rude word>",
                                                        "type": "PROFANITY"
                                                    }
                                                ]
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "cacheWrite": 0,
                            "noCache": 4,
                            "total": 4
                        },
                        "outputTokens": {
                            "text": 34,
                            "total": 34
                        },
                        "raw": {
                            "inputTokens": 4,
                            "outputTokens": 34,
                            "totalTokens": 38
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include stop_sequence in provider metadata
    #[tokio::test]
    async fn should_include_stop_sequence_in_provider_metadata() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
            json!({ "metadata": { "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 } } }),
            json!({
                "messageStop": {
                    "stopReason": "stop_sequence",
                    "additionalModelResponseFields": { "delta": { "stop_sequence": "STOP" } },
                },
            }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "stopSequences": ["STOP"] })))
            .await
            .expect("stream starts");

        let finishes: Vec<Value> = stream_parts(result)
            .await
            .iter()
            .map(part_json)
            .filter(|part| part["type"] == json!("finish"))
            .collect();
        assert_eq!(
            Value::Array(finishes),
            json!([
                {
                    "finishReason": {
                        "raw": "stop_sequence",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "stopSequence": "STOP"
                        },
                        "bedrock": {
                            "stopSequence": "STOP"
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "cacheWrite": 0,
                            "noCache": 4,
                            "total": 4
                        },
                        "outputTokens": {
                            "text": 34,
                            "total": 34
                        },
                        "raw": {
                            "inputTokens": 4,
                            "outputTokens": 34,
                            "totalTokens": 38
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should correctly parse delta.stop_sequence structure in streaming with additional fields
    #[tokio::test]
    async fn should_correctly_parse_delta_stop_sequence_structure_in_streaming() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Response" } } }),
            json!({ "metadata": { "usage": { "inputTokens": 5, "outputTokens": 10, "totalTokens": 15 } } }),
            json!({
                "messageStop": {
                    "stopReason": "stop_sequence",
                    "additionalModelResponseFields": {
                        "delta": { "stop_sequence": "CUSTOM_END" },
                        "otherField": "value",
                    },
                },
            }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "stopSequences": ["CUSTOM_END"] })))
            .await
            .expect("stream starts");

        let finish = find_part(&stream_parts(result).await, "finish").expect("finish part");
        assert_eq!(
            finish["providerMetadata"]["bedrock"]["stopSequence"],
            json!("CUSTOM_END")
        );
        assert_eq!(
            finish["finishReason"],
            json!({ "unified": "stop", "raw": "stop_sequence" })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include response headers in rawResponse
    #[tokio::test]
    async fn should_include_response_headers_in_raw_response() {
        let mock = MockFetch::new(vec![stream_response(
            event_stream_body(&[
                json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
                json!({ "messageStop": { "stopReason": "stop_sequence" } }),
            ]),
            &[
                ("x-amzn-requestid", "test-request-id"),
                ("x-amzn-trace-id", "test-trace-id"),
            ],
        )]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        assert_eq!(
            serde_json::to_value(result.response.and_then(|response| response.headers)).unwrap(),
            json!({
                "cache-control": "no-cache",
                "connection": "keep-alive",
                "content-type": "text/event-stream",
                "x-amzn-requestid": "test-request-id",
                "x-amzn-trace-id": "test-trace-id",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should extract response metadata
    #[tokio::test]
    async fn should_extract_response_metadata() {
        let mock = MockFetch::new(vec![stream_response(
            event_stream_body(&[
                json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
                json!({ "messageStop": { "stopReason": "end_turn" } }),
            ]),
            &[
                ("x-amzn-requestid", "test-request-id"),
                ("date", "Wed, 01 Jan 2025 00:00:00 GMT"),
            ],
        )]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        let metadata = find_part(&stream_parts(result).await, "response-metadata");
        assert_eq!(
            metadata,
            Some(json!({
                "id": "test-request-id",
                "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                "timestamp": "2025-01-01T00:00:00.000Z",
                "type": "response-metadata"
            }))
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should properly combine headers from all sources
    #[tokio::test]
    async fn should_properly_combine_headers_from_all_sources() {
        let mock = MockFetch::new(vec![stream_response(
            event_stream_body(&[
                json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
                json!({ "messageStop": { "stopReason": "stop_sequence" } }),
            ]),
            &[
                ("x-amzn-requestid", "test-request-id"),
                ("x-amzn-trace-id", "test-trace-id"),
            ],
        )]);
        let model = provider(
            inject_fetch_headers(
                vec![
                    ("options-header", "options-value"),
                    ("model-header", "model-value"),
                    ("shared-header", "options-shared"),
                    ("signed-header", "signed-value"),
                    ("authorization", "AWS4-HMAC-SHA256..."),
                ],
                mock.transport(),
            ),
            Some(
                [
                    ("model-header".to_string(), Some("model-value".to_string())),
                    (
                        "shared-header".to_string(),
                        Some("model-shared".to_string()),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
        )
        .chat(MODEL_ID);

        let mut options = options_with(json!({ "includeRawChunks": false }));
        options.headers = Some(
            [
                ("options-header".to_string(), "options-value".to_string()),
                ("shared-header".to_string(), "options-shared".to_string()),
            ]
            .into_iter()
            .collect(),
        );
        model.do_stream(&options).await.expect("stream starts");

        let request_headers = &mock.seen()[0].headers;
        assert_eq!(
            request_headers.get("options-header").map(String::as_str),
            Some("options-value")
        );
        assert_eq!(
            request_headers.get("model-header").map(String::as_str),
            Some("model-value")
        );
        assert_eq!(
            request_headers.get("signed-header").map(String::as_str),
            Some("signed-value")
        );
        assert_eq!(
            request_headers.get("authorization").map(String::as_str),
            Some("AWS4-HMAC-SHA256...")
        );
        assert_eq!(
            request_headers.get("shared-header").map(String::as_str),
            Some("options-shared")
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should work with partial headers
    #[tokio::test]
    async fn should_work_with_partial_headers() {
        let mock = MockFetch::new(vec![stream_chunks(&[])]);
        let model = provider(
            inject_fetch_headers(
                vec![
                    ("model-header", "model-value"),
                    ("signed-header", "signed-value"),
                    ("authorization", "AWS4-HMAC-SHA256..."),
                ],
                mock.transport(),
            ),
            Some(
                [("model-header".to_string(), Some("model-value".to_string()))]
                    .into_iter()
                    .collect(),
            ),
        )
        .chat(MODEL_ID);

        model
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        let request_headers = &mock.seen()[0].headers;
        assert_eq!(
            request_headers.get("model-header").map(String::as_str),
            Some("model-value")
        );
        assert_eq!(
            request_headers.get("signed-header").map(String::as_str),
            Some("signed-value")
        );
        assert_eq!(
            request_headers.get("authorization").map(String::as_str),
            Some("AWS4-HMAC-SHA256...")
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include providerOptions in the request for streaming calls
    #[tokio::test]
    async fn should_include_provider_options_in_the_request_for_streaming_calls() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Dummy" } } }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        model(&mock)
            .do_stream(&options_with(json!({
                "includeRawChunks": false,
                "providerOptions": { "bedrock": { "foo": "bar" } },
            })))
            .await
            .expect("stream starts");

        assert_match_object(&request_body(&mock.seen(), 0), &json!({ "foo": "bar" }));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include cache token usage in providerMetadata
    #[tokio::test]
    async fn should_include_cache_token_usage_in_provider_metadata() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
            json!({
                "metadata": {
                    "usage": {
                        "inputTokens": 4,
                        "outputTokens": 34,
                        "totalTokens": 38,
                        "cacheReadInputTokens": 2,
                        "cacheWriteInputTokens": 3,
                    },
                },
            }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        assert_eq!(
            parts_json(&stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Hello",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "finishReason": {
                        "raw": "stop_sequence",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "usage": {
                                "cacheWriteInputTokens": 3
                            }
                        },
                        "bedrock": {
                            "usage": {
                                "cacheWriteInputTokens": 3
                            }
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 2,
                            "cacheWrite": 3,
                            "noCache": 4,
                            "total": 9
                        },
                        "outputTokens": {
                            "text": 34,
                            "total": 34
                        },
                        "raw": {
                            "cacheReadInputTokens": 2,
                            "cacheWriteInputTokens": 3,
                            "inputTokens": 4,
                            "outputTokens": 34,
                            "totalTokens": 38
                        }
                    }
                }
            ])
        );
    }

    /// The finish part's provider metadata of a `Hello` stream whose
    /// `metadata` chunk is `metadata`.
    async fn finish_provider_metadata(metadata: Value) -> Value {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
            json!({ "metadata": metadata }),
            json!({ "messageStop": { "stopReason": "end_turn" } }),
        ])]);
        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");
        let finish = find_part(&stream_parts(result).await, "finish").expect("finish part");
        finish["providerMetadata"].clone()
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include performanceConfig in providerMetadata
    #[tokio::test]
    async fn should_include_performance_config_in_provider_metadata() {
        let provider_metadata = finish_provider_metadata(json!({
            "usage": { "inputTokens": 4, "outputTokens": 34 },
            "performanceConfig": { "latency": "optimized" },
        }))
        .await;

        assert_eq!(
            provider_metadata,
            json!({
                "amazonBedrock": {
                    "performanceConfig": {
                        "latency": "optimized"
                    }
                },
                "bedrock": {
                    "performanceConfig": {
                        "latency": "optimized"
                    }
                }
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include serviceTier in providerMetadata
    #[tokio::test]
    async fn should_include_service_tier_in_provider_metadata() {
        let provider_metadata = finish_provider_metadata(json!({
            "usage": { "inputTokens": 4, "outputTokens": 34 },
            "serviceTier": { "type": "on-demand" },
        }))
        .await;

        assert_eq!(
            provider_metadata,
            json!({
                "amazonBedrock": {
                    "serviceTier": {
                        "type": "on-demand"
                    }
                },
                "bedrock": {
                    "serviceTier": {
                        "type": "on-demand"
                    }
                }
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include cacheDetails in providerMetadata
    #[tokio::test]
    async fn should_include_cache_details_in_provider_metadata() {
        let provider_metadata = finish_provider_metadata(json!({
            "usage": {
                "inputTokens": 4,
                "outputTokens": 34,
                "cacheDetails": [{ "inputTokens": 100, "ttl": "T5M" }],
            },
        }))
        .await;

        assert_eq!(
            provider_metadata,
            json!({
                "amazonBedrock": {
                    "usage": {
                        "cacheDetails": [
                            {
                                "inputTokens": 100,
                                "ttl": "T5M"
                            }
                        ]
                    }
                },
                "bedrock": {
                    "usage": {
                        "cacheDetails": [
                            {
                                "inputTokens": 100,
                                "ttl": "T5M"
                            }
                        ]
                    }
                }
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle system messages with cache points
    #[tokio::test]
    async fn should_handle_system_messages_with_cache_points() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        model(&mock)
            .do_stream(&call_options(json!({
                "prompt": [
                    {
                        "role": "system",
                        "content": "System Prompt",
                        "providerOptions": { "bedrock": { "cachePoint": { "type": "default" } } },
                    },
                    { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                ],
                "includeRawChunks": false,
            })))
            .await
            .expect("stream starts");

        assert_match_object(
            &request_body(&mock.seen(), 0),
            &json!({
                "system": [{ "text": "System Prompt" }, { "cachePoint": { "type": "default" } }],
                "messages": [{ "role": "user", "content": [{ "text": "Hello" }] }],
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should stream reasoning text deltas
    #[tokio::test]
    async fn should_stream_reasoning_text_deltas() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "reasoningContent": { "text": "I am thinking" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "reasoningContent": { "text": " about this problem..." } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "reasoningContent": { "signature": "abc123signature" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 1,
                    "delta": { "text": "Based on my reasoning, the answer is 42." },
                },
            }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        assert_eq!(
            parts_json(&stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "reasoning-start"
                },
                {
                    "delta": "I am thinking",
                    "id": "0",
                    "type": "reasoning-delta"
                },
                {
                    "delta": " about this problem...",
                    "id": "0",
                    "type": "reasoning-delta"
                },
                {
                    "delta": "",
                    "id": "0",
                    "providerMetadata": {
                        "amazonBedrock": {
                            "signature": "abc123signature"
                        },
                        "bedrock": {
                            "signature": "abc123signature"
                        }
                    },
                    "type": "reasoning-delta"
                },
                {
                    "id": "1",
                    "type": "text-start"
                },
                {
                    "delta": "Based on my reasoning, the answer is 42.",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "finishReason": {
                        "raw": "stop_sequence",
                        "unified": "stop"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should stream redacted reasoning
    #[tokio::test]
    async fn should_stream_redacted_reasoning() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 0,
                    "delta": { "reasoningContent": { "data": "redacted-reasoning-data" } },
                },
            }),
            json!({
                "contentBlockDelta": {
                    "contentBlockIndex": 1,
                    "delta": { "text": "Here is my answer." },
                },
            }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": false })))
            .await
            .expect("stream starts");

        assert_eq!(
            parts_json(&stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "reasoning-start"
                },
                {
                    "delta": "",
                    "id": "0",
                    "providerMetadata": {
                        "amazonBedrock": {
                            "redactedData": "redacted-reasoning-data"
                        },
                        "bedrock": {
                            "redactedData": "redacted-reasoning-data"
                        }
                    },
                    "type": "reasoning-delta"
                },
                {
                    "id": "1",
                    "type": "text-start"
                },
                {
                    "delta": "Here is my answer.",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "finishReason": {
                        "raw": "stop_sequence",
                        "unified": "stop"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include raw chunks when includeRawChunks is true
    #[tokio::test]
    async fn should_include_raw_chunks_when_include_raw_chunks_is_true() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({ "includeRawChunks": true })))
            .await
            .expect("stream starts");

        assert_eq!(
            parts_json(&stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "rawValue": {
                        "contentBlockDelta": {
                            "contentBlockIndex": 0,
                            "delta": {
                                "text": "Hello"
                            }
                        }
                    },
                    "type": "raw"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Hello",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "rawValue": {
                        "messageStop": {
                            "stopReason": "stop_sequence"
                        }
                    },
                    "type": "raw"
                },
                {
                    "finishReason": {
                        "raw": "stop_sequence",
                        "unified": "stop"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should transform reasoningConfig to thinking in stream requests
    #[tokio::test]
    async fn should_transform_reasoning_config_to_thinking_in_stream_requests() {
        let mock = MockFetch::new(vec![stream_chunks(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "text": "Hello" } } }),
            json!({ "messageStop": { "stopReason": "stop_sequence" } }),
        ])]);

        model(&mock)
            .do_stream(&options_with(json!({
                "maxOutputTokens": 100,
                "includeRawChunks": false,
                "providerOptions": {
                    "bedrock": { "reasoningConfig": { "type": "enabled", "budgetTokens": 2000 } },
                },
            })))
            .await
            .expect("stream starts");

        let request_body = request_body(&mock.seen(), 0);
        assert_match_object(
            &request_body,
            &json!({
                "additionalModelRequestFields": {
                    "thinking": { "type": "enabled", "budget_tokens": 2000 },
                },
                "inferenceConfig": { "maxTokens": 2100 },
            }),
        );
        assert_eq!(request_body.get("reasoningConfig"), None);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › merges user additionalModelRequestFields with derived thinking (stream)
    #[tokio::test]
    async fn merges_user_additional_model_request_fields_with_derived_thinking_stream() {
        let mock = MockFetch::new(vec![stream_chunks(&[json!({
            "messageStop": { "stopReason": "stop_sequence" },
        })])]);

        model(&mock)
            .do_stream(&options_with(json!({
                "includeRawChunks": false,
                "providerOptions": {
                    "bedrock": {
                        "reasoningConfig": { "type": "enabled", "budgetTokens": 500 },
                        "additionalModelRequestFields": { "foo": "bar", "custom": 42 },
                    },
                },
            })))
            .await
            .expect("stream starts");

        let body = request_body(&mock.seen(), 0);
        assert_eq!(body.get("reasoningConfig"), None);
        assert_match_object(
            &body["additionalModelRequestFields"],
            &json!({
                "foo": "bar",
                "custom": 42,
                "thinking": { "type": "enabled", "budget_tokens": 500 },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should pass serviceTier provider option in stream requests
    #[tokio::test]
    async fn should_pass_service_tier_provider_option_in_stream_requests() {
        let mock = MockFetch::new(vec![stream_chunks(&[json!({
            "messageStop": { "stopReason": "stop_sequence" },
        })])]);

        model(&mock)
            .do_stream(&options_with(json!({
                "includeRawChunks": false,
                "providerOptions": { "bedrock": { "serviceTier": "priority" } },
            })))
            .await
            .expect("stream starts");

        let request_body = request_body(&mock.seen(), 0);
        assert_match_object(
            &request_body,
            &json!({ "serviceTier": { "type": "priority" } }),
        );
        assert_eq!(
            request_body
                .get("additionalModelRequestFields")
                .and_then(|fields| fields.get("serviceTier")),
            None
        );
    }

    /// The parts of a fixture stream answered for `options`.
    async fn fixture_stream(chunks: &str, options: Value) -> Value {
        let mock = MockFetch::new(vec![stream_chunks(&fixture_chunks(chunks))]);
        let result = model(&mock)
            .do_stream(&call_options(options))
            .await
            .expect("stream starts");
        parts_json(&stream_parts(result).await)
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle JSON response format in streaming
    #[tokio::test]
    async fn should_handle_json_response_format_in_streaming() {
        let parts = fixture_stream(
            include_str!("fixtures/amazon-bedrock-json-tool.1.chunks.txt"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate JSON" }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                    },
                },
                "includeRawChunks": false,
            }),
        )
        .await;

        assert_eq!(
            parts,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "{\"value\":\"test\"}",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "0",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        },
                        "bedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include text content before JSON tool call in streaming
    #[tokio::test]
    async fn should_include_text_content_before_json_tool_call_in_streaming() {
        let parts = fixture_stream(
            include_str!("fixtures/amazon-bedrock-json-tool.2.chunks.txt"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate JSON" }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                    },
                },
                "includeRawChunks": false,
            }),
        )
        .await;

        assert_eq!(
            parts,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Let me generate",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "delta": " that JSON for you.",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "1",
                    "type": "text-start"
                },
                {
                    "delta": "{\"value\":\"test\"}",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "id": "1",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        },
                        "bedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should stream text introduction before JSON-only response
    #[tokio::test]
    async fn should_stream_text_introduction_before_json_only_response() {
        let parts = fixture_stream(
            include_str!("fixtures/amazon-bedrock-json-only-text-first.1.chunks.txt"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Return name data" }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "name": { "type": "string" } },
                        "required": ["name"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                    },
                },
                "includeRawChunks": false,
            }),
        )
        .await;

        assert_eq!(
            parts,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Here is your data:",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "0",
                    "type": "text-end"
                },
                {
                    "id": "1",
                    "type": "text-start"
                },
                {
                    "delta": "{\"name\":\"John\"}",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "id": "1",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        },
                        "bedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "cacheWrite": 0,
                            "noCache": 100,
                            "total": 100
                        },
                        "outputTokens": {
                            "text": 20,
                            "total": 20
                        },
                        "raw": {
                            "inputTokens": 100,
                            "outputTokens": 20,
                            "totalTokens": 120
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should include multiple text blocks before JSON tool call in streaming
    #[tokio::test]
    async fn should_include_multiple_text_blocks_before_json_tool_call_in_streaming() {
        let parts = fixture_stream(
            include_str!("fixtures/amazon-bedrock-json-tool.3.chunks.txt"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate data" }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "result": { "type": "number" } },
                        "required": ["result"],
                    },
                },
                "includeRawChunks": false,
            }),
        )
        .await;

        assert_eq!(
            parts,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "First text block.",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "1",
                    "type": "text-start"
                },
                {
                    "delta": "Second text block.",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "id": "2",
                    "type": "text-start"
                },
                {
                    "delta": "Third block.",
                    "id": "2",
                    "type": "text-delta"
                },
                {
                    "id": "3",
                    "type": "text-start"
                },
                {
                    "delta": "{\"result\":42}",
                    "id": "3",
                    "type": "text-delta"
                },
                {
                    "id": "3",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        },
                        "bedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle regular tool call before JSON tool call in streaming
    #[tokio::test]
    async fn should_handle_regular_tool_call_before_json_tool_call_in_streaming() {
        let parts = fixture_stream(
            include_str!("fixtures/amazon-bedrock-json-with-tool.1.chunks.txt"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Test" }] }],
                "tools": [
                    {
                        "type": "function",
                        "name": "get-weather",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "location": { "type": "string" } },
                            "required": ["location"],
                        },
                    },
                ],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "result": { "type": "string" } },
                        "required": ["result"],
                    },
                },
                "includeRawChunks": false,
            }),
        )
        .await;

        assert_eq!(
            parts,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Calling a tool first.",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "tool-1",
                    "toolName": "get-weather",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"location\":\"SF\"}",
                    "id": "tool-1",
                    "type": "tool-input-delta"
                },
                {
                    "id": "tool-1",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"location\":\"SF\"}",
                    "toolCallId": "tool-1",
                    "toolName": "get-weather",
                    "type": "tool-call"
                },
                {
                    "id": "2",
                    "type": "text-start"
                },
                {
                    "delta": "Now JSON.",
                    "id": "2",
                    "type": "text-delta"
                },
                {
                    "id": "3",
                    "type": "text-start"
                },
                {
                    "delta": "{\"result\":\"data\"}",
                    "id": "3",
                    "type": "text-delta"
                },
                {
                    "id": "3",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        },
                        "bedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should handle multiple regular tool calls before JSON tool call in streaming
    #[tokio::test]
    async fn should_handle_multiple_regular_tool_calls_before_json_tool_call_in_streaming() {
        let parts = fixture_stream(
            include_str!("fixtures/amazon-bedrock-json-with-tools.1.chunks.txt"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Test" }] }],
                "tools": [
                    {
                        "type": "function",
                        "name": "tool-a",
                        "inputSchema": { "type": "object", "properties": { "a": { "type": "string" } } },
                    },
                    {
                        "type": "function",
                        "name": "tool-b",
                        "inputSchema": { "type": "object", "properties": { "b": { "type": "string" } } },
                    },
                ],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "final": { "type": "string" } },
                        "required": ["final"],
                    },
                },
                "includeRawChunks": false,
            }),
        )
        .await;

        assert_eq!(
            parts,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Multiple tools.",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "tool-1",
                    "toolName": "tool-a",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"a\":\"1\"}",
                    "id": "tool-1",
                    "type": "tool-input-delta"
                },
                {
                    "id": "tool-1",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"a\":\"1\"}",
                    "toolCallId": "tool-1",
                    "toolName": "tool-a",
                    "type": "tool-call"
                },
                {
                    "id": "tool-2",
                    "toolName": "tool-b",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"b\":\"2\"}",
                    "id": "tool-2",
                    "type": "tool-input-delta"
                },
                {
                    "id": "tool-2",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"b\":\"2\"}",
                    "toolCallId": "tool-2",
                    "toolName": "tool-b",
                    "type": "tool-call"
                },
                {
                    "id": "3",
                    "type": "text-start"
                },
                {
                    "delta": "Final JSON.",
                    "id": "3",
                    "type": "text-delta"
                },
                {
                    "id": "4",
                    "type": "text-start"
                },
                {
                    "delta": "{\"final\":\"result\"}",
                    "id": "4",
                    "type": "text-delta"
                },
                {
                    "id": "4",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "amazonBedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        },
                        "bedrock": {
                            "isJsonResponseFromTool": true,
                            "stopSequence": null
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should stream text, then regular tool calls, with JSON response format available
    #[tokio::test]
    async fn should_stream_text_then_regular_tool_calls_with_json_response_format_available() {
        let parts = fixture_stream(
            include_str!("fixtures/amazon-bedrock-json-tool-text-then-weather-then-json.1.chunks.txt"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "What's 2+2? Also check the weather in San Francisco and London.",
                            },
                        ],
                    },
                ],
                "tools": [
                    {
                        "type": "function",
                        "name": "weather",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "location": { "type": "string" } },
                            "required": ["location"],
                        },
                    },
                ],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "locations": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "location": { "type": "string" },
                                        "temperature": { "type": "number" },
                                    },
                                },
                            },
                        },
                    },
                },
                "includeRawChunks": false,
            }),
        )
        .await;

        assert_eq!(
            parts,
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "modelId": "anthropic.claude-3-haiku-20240307-v1:0",
                    "type": "response-metadata"
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "2 + 2 equals 4. Now let me check",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "delta": " the weather for you.",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "0",
                    "type": "text-end"
                },
                {
                    "id": "weather-tool-1",
                    "toolName": "weather",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"location\":",
                    "id": "weather-tool-1",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\"San Francisco\"}",
                    "id": "weather-tool-1",
                    "type": "tool-input-delta"
                },
                {
                    "id": "weather-tool-1",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"location\":\"San Francisco\"}",
                    "toolCallId": "weather-tool-1",
                    "toolName": "weather",
                    "type": "tool-call"
                },
                {
                    "id": "weather-tool-2",
                    "toolName": "weather",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"location\":\"London\"}",
                    "id": "weather-tool-2",
                    "type": "tool-input-delta"
                },
                {
                    "id": "weather-tool-2",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"location\":\"London\"}",
                    "toolCallId": "weather-tool-2",
                    "toolName": "weather",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_use",
                        "unified": "tool-calls"
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "cacheWrite": 0,
                            "noCache": 500,
                            "total": 500
                        },
                        "outputTokens": {
                            "text": 100,
                            "total": 100
                        },
                        "raw": {
                            "inputTokens": 500,
                            "outputTokens": 100,
                            "totalTokens": 600
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should warn when Anthropic model receives maxReasoningEffort in stream
    #[tokio::test]
    async fn should_warn_when_anthropic_model_receives_max_reasoning_effort_in_stream() {
        let mock = MockFetch::new(vec![stream_chunks(&[json!({
            "messageStop": { "stopReason": "stop_sequence" },
        })])]);

        let result = model(&mock)
            .do_stream(&options_with(json!({
                "includeRawChunks": false,
                "providerOptions": {
                    "bedrock": {
                        "reasoningConfig": { "type": "enabled", "maxReasoningEffort": "medium" },
                    },
                },
            })))
            .await
            .expect("stream starts");
        stream_items(result).await;

        let request_body = request_body(&mock.seen(), 0);
        assert_eq!(
            request_body
                .get("additionalModelRequestFields")
                .and_then(|fields| fields.get("reasoningConfig")),
            None
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doStream › should support tool calls with empty input (no arguments)
    #[tokio::test]
    async fn should_support_tool_calls_with_empty_input_no_arguments() {
        let mock = MockFetch::new(vec![stream_chunks(&fixture_chunks(include_str!(
            "fixtures/amazon-bedrock-tool-no-args.chunks.txt"
        )))]);

        let result = model(&mock)
            .do_stream(&options_with(json!({
                "tools": [
                    {
                        "type": "function",
                        "name": "updateIssueList",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "required": [],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    },
                ],
                "includeRawChunks": false,
            })))
            .await
            .expect("stream starts");

        let tool_call_part = find_part(&stream_parts(result).await, "tool-call");
        assert!(tool_call_part.is_some());
        assert_eq!(tool_call_part.unwrap()["input"], json!("{}"));
    }
}

mod do_generate {
    #[allow(unused_imports)]
    use super::*;

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use aimux_core::language_model::LanguageModel;
    use aimux_core::options::CallOptions;
    use aimux_core::result::GenerateResult;
    use aimux_provider_utils::{
        Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt, Resolvable,
    };
    use async_trait::async_trait;
    use reqwest::header::{HeaderName, HeaderValue};
    use serde_json::{Value, json};

    use crate::bedrock::{AmazonBedrockProviderSettings, create_amazon_bedrock};
    use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

    const BASE_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";
    const MODEL_ID: &str = "anthropic.claude-3-haiku-20240307-v1:0";
    const ANTHROPIC_MODEL_ID: &str = "anthropic.claude-3-5-sonnet-20240620-v1:0";
    const LEGACY_ANTHROPIC_37_MODEL_ID: &str = "us.anthropic.claude-3-7-sonnet-20250219-v1:0";
    const NOVA_MODEL_ID: &str = "us.amazon.nova-2-lite-v1:0";
    const NOVA_MICRO_MODEL_ID: &str = "us.amazon.nova-micro-v1:0";
    const OPENAI_MODEL_ID: &str = "openai.gpt-oss-120b-1:0";
    const US_OPENAI_MODEL_ID: &str = "us.openai.gpt-5.6-luna";
    const GLOBAL_OPENAI_MODEL_ID: &str = "global.openai.gpt-5.6-luna";
    const CUSTOM_OPENAI_SUBSTRING_MODEL_ID: &str = "custom-openai.gpt-5.6-luna";
    const NEWER_ANTHROPIC_MODEL_ID: &str = "anthropic.claude-sonnet-4-6-v1";
    const HAIKU45_ANTHROPIC_MODEL_ID: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";
    const NATIVE_STRUCTURED_OUTPUT_ANTHROPIC_MODEL_ID: &str =
        "anthropic.claude-sonnet-4-5-20250929-v1:0";
    const OPUS_ANTHROPIC_MODEL_ID: &str = "us.anthropic.claude-opus-4-8";
    const OPUS5_ANTHROPIC_MODEL_ID: &str = "us.anthropic.claude-opus-5";
    const OPUS55_ANTHROPIC_MODEL_ID: &str = "us.anthropic.claude-opus-5-5";
    const SONNET5_ANTHROPIC_MODEL_ID: &str = "us.anthropic.claude-sonnet-5";

    /// Upstream `TEST_PROMPT`.
    fn test_prompt() -> Value {
        json!([
            { "role": "system", "content": "System Prompt" },
            { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
        ])
    }

    /// Upstream `mockTrace`.
    fn mock_trace() -> Value {
        json!({
            "guardrail": {
                "inputAssessment": {
                    "1abcd2ef34gh": {
                        "contentPolicy": {
                            "filters": [
                                { "action": "BLOCKED", "confidence": "LOW", "type": "INSULTS" },
                            ],
                        },
                        "wordPolicy": {
                            "managedWordLists": [
                                { "action": "BLOCKED", "match": "<rude word>", "type": "PROFANITY" },
                            ],
                        },
                    },
                },
            },
        })
    }

    /// An upstream `__fixtures__/<name>.json` file.
    fn fixture(name: &str) -> Value {
        let text = match name {
            "amazon-bedrock-text" => include_str!("fixtures/amazon-bedrock-text.json"),
            "amazon-bedrock-reasoning" => include_str!("fixtures/amazon-bedrock-reasoning.json"),
            "amazon-bedrock-guard-content-intervened" => {
                include_str!("fixtures/amazon-bedrock-guard-content-intervened.json")
            }
            "amazon-bedrock-json-tool.1" => {
                include_str!("fixtures/amazon-bedrock-json-tool.1.json")
            }
            "amazon-bedrock-json-tool.2" => {
                include_str!("fixtures/amazon-bedrock-json-tool.2.json")
            }
            "amazon-bedrock-json-tool.3" => {
                include_str!("fixtures/amazon-bedrock-json-tool.3.json")
            }
            "amazon-bedrock-json-tool-with-answer.1" => {
                include_str!("fixtures/amazon-bedrock-json-tool-with-answer.1.json")
            }
            "amazon-bedrock-json-other-tool.1" => {
                include_str!("fixtures/amazon-bedrock-json-other-tool.1.json")
            }
            "amazon-bedrock-json-with-tool.1" => {
                include_str!("fixtures/amazon-bedrock-json-with-tool.1.json")
            }
            "amazon-bedrock-json-with-tools.1" => {
                include_str!("fixtures/amazon-bedrock-json-with-tools.1.json")
            }
            "amazon-bedrock-tool-no-args" => {
                include_str!("fixtures/amazon-bedrock-tool-no-args.json")
            }
            other => panic!("unknown fixture {other}"),
        };
        serde_json::from_str(text).expect("fixture is JSON")
    }

    /// The test server's `json-value` response: the compact JSON body with
    /// `content-type` and `content-length`, plus any extra headers.
    fn json_response_with_headers(body: &Value, headers: &[(&str, &str)]) -> Canned {
        let mut canned = Canned::json(body);
        canned
            .headers
            .push(("content-length".into(), canned.body.len().to_string()));
        for (name, value) in headers {
            canned.headers.push(((*name).into(), (*value).into()));
        }
        canned
    }

    fn json_response(body: &Value) -> Canned {
        json_response_with_headers(body, &[])
    }

    /// Upstream `prepareJsonFixtureResponse(name)`.
    fn fixture_response(name: &str) -> Canned {
        json_response(&fixture(name))
    }

    /// The `simpleResponse` of the reasoning describes.
    fn simple_response() -> Canned {
        hello_response("stop_sequence")
    }

    /// `{ output: { content: [{ text: 'Hello' }] }, stopReason }` with usage 1/1/2.
    fn hello_response(stop_reason: &str) -> Canned {
        json_response(&json!({
            "output": { "message": { "content": [{ "text": "Hello" }], "role": "assistant" } },
            "stopReason": stop_reason,
            "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 },
        }))
    }

    /// A text-only response with usage 4/34/38 plus extra top-level fields.
    fn hello_world_response(extra: Value) -> Canned {
        let mut body = json!({
            "output": { "message": { "role": "assistant", "content": [{ "type": "text", "text": "Hello, World!" }] } },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence",
        });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json_response(&body)
    }

    /// A response whose message content is `content` (usage 4/34/38,
    /// stopReason `stop_sequence`).
    fn content_response(content: Value) -> Canned {
        json_response(&json!({
            "output": { "message": { "role": "assistant", "content": content } },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence",
        }))
    }

    /// The JSON response of a JSON-tool call: `{ name: 'Test' }`.
    fn json_tool_name_response() -> Canned {
        json_response(&json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        { "toolUse": { "toolUseId": "json-tool-id", "name": "json", "input": { "name": "Test" } } },
                    ],
                },
            },
            "usage": { "inputTokens": 4, "outputTokens": 10, "totalTokens": 14 },
            "stopReason": "tool_use",
        }))
    }

    /// The text response of the Anthropic beta cases.
    fn test_response_stop() -> Canned {
        json_response(&json!({
            "output": { "message": { "role": "assistant", "content": [{ "type": "text", "text": "test response" }] } },
            "usage": { "inputTokens": 10, "outputTokens": 20, "totalTokens": 30 },
            "stopReason": "stop",
        }))
    }

    /// The `{ name: string }` response format schema many cases use.
    fn name_response_format() -> Value {
        json!({
            "type": "json",
            "schema": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
            },
        })
    }

    /// The tool-call conversation of the "no active tools" cases.
    fn conversation_with_tool_calls() -> Value {
        json!([
            { "role": "user", "content": [{ "type": "text", "text": "What is the weather in Toronto?" }] },
            {
                "role": "assistant",
                "content": [
                    { "type": "tool-call", "toolCallId": "tool-call-1", "toolName": "weather", "input": { "city": "Toronto" } },
                ],
            },
            {
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolCallId": "tool-call-1",
                        "toolName": "weather",
                        "output": { "type": "text", "value": "The weather in Toronto is 20°C." },
                    },
                ],
            },
            { "role": "user", "content": [{ "type": "text", "text": "Now give me a summary." }] },
        ])
    }

    /// The `bash_20241022` provider tool.
    fn bash_tool() -> Value {
        json!({ "type": "provider", "id": "anthropic.bash_20241022", "name": "bash", "args": {} })
    }

    /// Upstream `injectFetchHeaders`: sets the headers on every request, then
    /// forwards it.
    struct InjectHeaders {
        headers: Vec<(&'static str, &'static str)>,
        inner: FetchFunction,
    }

    #[async_trait]
    impl Fetch for InjectHeaders {
        async fn fetch(&self, mut request: FetchRequest) -> Result<FetchResponse, FetchError> {
            for (name, value) in &self.headers {
                request.headers.insert(
                    HeaderName::from_static(name),
                    HeaderValue::from_static(value),
                );
            }
            self.inner.fetch(request).await
        }
    }

    /// The settings of upstream's module-level models (`baseUrl`, a fetch
    /// that authenticates the request, no extra headers).
    fn settings(fetch: FetchFunction) -> AmazonBedrockProviderSettings {
        AmazonBedrockProviderSettings {
            api_key: Some(Resolvable::Value("test-api-key".into())),
            region: Some("us-east-1".into()),
            base_url: Some(BASE_URL.into()),
            fetch: Some(fetch),
            ..Default::default()
        }
    }

    /// Upstream `LanguageModelV4CallOptions` (camelCase JSON).
    fn call_options(options: Value) -> CallOptions {
        serde_json::from_value(options).expect("valid call options")
    }

    struct Generated {
        result: GenerateResult,
        seen: Vec<Seen>,
    }

    impl Generated {
        /// `server.calls[0].requestBodyJson`.
        fn request_body(&self) -> Value {
            self.seen[0].json_body()
        }

        /// `server.calls[0].requestHeaders`.
        fn request_headers(&self) -> &BTreeMap<String, String> {
            &self.seen[0].headers
        }

        fn content(&self) -> Value {
            to_json(&self.result.content)
        }

        fn warnings(&self) -> Value {
            to_json(&self.result.warnings)
        }

        fn finish_reason(&self) -> Value {
            to_json(&self.result.finish_reason)
        }

        /// `result.providerMetadata?.bedrock` at a JSON pointer below it.
        fn bedrock_metadata(&self, pointer: &str) -> Option<Value> {
            to_json(&self.result.provider_metadata)
                .pointer(&format!("/bedrock{pointer}"))
                .cloned()
        }
    }

    async fn generate_with(
        settings: AmazonBedrockProviderSettings,
        fetch: &Arc<MockFetch>,
        model_id: &str,
        options: Value,
    ) -> Generated {
        let model = create_amazon_bedrock(settings)
            .expect("provider")
            .chat(model_id);
        let result = model
            .do_generate(&call_options(options))
            .await
            .expect("doGenerate succeeds");
        Generated {
            result,
            seen: fetch.seen(),
        }
    }

    /// `doGenerate` on a model built like upstream's module-level models.
    async fn generate(model_id: &str, response: Canned, options: Value) -> Generated {
        let fetch = MockFetch::new(vec![response]);
        generate_with(settings(fetch.transport()), &fetch, model_id, options).await
    }

    fn to_json<T: serde::Serialize>(value: &T) -> Value {
        serde_json::to_value(value).expect("serializable")
    }

    /// Deep equality with JavaScript number semantics (`1` equals `1.0`).
    fn js_equal(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Number(a), Value::Number(e)) => a.as_f64() == e.as_f64(),
            (Value::Object(a), Value::Object(e)) => {
                a.len() == e.len()
                    && e.iter()
                        .all(|(key, ev)| a.get(key).is_some_and(|av| js_equal(av, ev)))
            }
            (Value::Array(a), Value::Array(e)) => {
                a.len() == e.len() && a.iter().zip(e).all(|(av, ev)| js_equal(av, ev))
            }
            _ => actual == expected,
        }
    }

    /// `toMatchObject`: every expected key matches, recursively.
    fn js_match(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(a), Value::Object(e)) => e
                .iter()
                .all(|(key, ev)| a.get(key).is_some_and(|av| js_match(av, ev))),
            (Value::Array(a), Value::Array(e)) => {
                a.len() == e.len() && a.iter().zip(e).all(|(av, ev)| js_match(av, ev))
            }
            _ => js_equal(actual, expected),
        }
    }

    /// `expect(actual).toEqual(expected)`.
    fn assert_json_eq(actual: &Value, expected: &Value) {
        assert!(
            js_equal(actual, expected),
            "toEqual failed\n  actual: {actual:#}\nexpected: {expected:#}"
        );
    }

    /// `expect(actual).toMatchObject(expected)`.
    fn assert_match_object(actual: &Value, expected: &Value) {
        assert!(
            js_match(actual, expected),
            "toMatchObject failed\n  actual: {actual:#}\nexpected: {expected:#}"
        );
    }

    /// `expect(list).toContainEqual(expected)`.
    fn assert_contains_equal(list: &Value, expected: &Value) {
        assert!(
            list.as_array()
                .is_some_and(|items| items.iter().any(|item| js_equal(item, expected))),
            "toContainEqual failed\n    list: {list:#}\nexpected: {expected:#}"
        );
    }

    /// Whether some warning has this `feature`
    /// (`expect.objectContaining({ feature })`).
    fn has_warning_feature(g: &Generated, feature: &str) -> bool {
        g.warnings()
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["feature"] == feature)
    }

    /// The value at a JSON pointer (the `?.` chains); `None` is `undefined`.
    fn at<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
        value.pointer(pointer)
    }

    /// The `toolSpec.name` of every tool in the request's `toolConfig`.
    fn tool_names(body: &Value) -> Vec<String> {
        body["toolConfig"]["tools"]
            .as_array()
            .expect("toolConfig.tools")
            .iter()
            .map(|tool| {
                tool["toolSpec"]["name"]
                    .as_str()
                    .expect("toolSpec.name")
                    .to_string()
            })
            .collect()
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should generate an ID when toolUseId is empty
    #[tokio::test]
    async fn should_generate_an_id_when_tool_use_id_is_empty() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": {
                    "message": {
                        "role": "assistant",
                        "content": [
                            { "toolUse": { "toolUseId": "", "name": "test-tool", "input": { "value": "test" } } },
                        ],
                    },
                },
                "stopReason": "tool_use",
                "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 },
            })),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        // Upstream injects `generateId: () => 'test-id'`; aimux has no id
        // generator seam, so the generated id must be non-empty and the rest
        // of the part is compared with the id replaced by `test-id`.
        let content = g.content();
        let call = content
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["type"] == "tool-call")
            .expect("a tool-call part");
        assert!(
            call["toolCallId"].as_str().is_some_and(|id| !id.is_empty()),
            "expected a generated toolCallId, got {call:#}"
        );
        let mut call = call.clone();
        call["toolCallId"] = json!("test-id");
        assert_contains_equal(
            &json!([call]),
            &json!({
                "type": "tool-call",
                "toolCallId": "test-id",
                "toolName": "test-tool",
                "input": "{\"value\":\"test\"}",
            }),
        );
    }

    mod text {
        use super::*;

        /// The describe's `beforeEach`.
        fn text_response() -> Canned {
            fixture_response("amazon-bedrock-text")
        }

        fn citations_response(citations_content: Value) -> Canned {
            json_response(&json!({
                "output": {
                    "message": {
                        "role": "assistant",
                        "content": [{ "citationsContent": citations_content }],
                    },
                },
                "stopReason": "end_turn",
                "usage": { "inputTokens": 1, "outputTokens": 2, "totalTokens": 3 },
            }))
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › text › should extract text response
        #[tokio::test]
        async fn should_extract_text_response() {
            let g = generate(
                MODEL_ID,
                text_response(),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            let snapshot: Value = serde_json::from_str(include_str!(
                "snapshots/do_generate_text_should_extract_text_response.json"
            ))
            .unwrap();
            assert_json_eq(&to_json(&g.result), &snapshot);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › text › should extract text from citation content
        #[tokio::test]
        async fn should_extract_text_from_citation_content() {
            let g = generate(
                MODEL_ID,
                citations_response(json!({
                    "content": [{ "text": "Citation " }, { "text": "response" }],
                    "citations": [],
                })),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            assert_json_eq(
                &g.content(),
                &json!([
                    { "type": "text", "text": "Citation " },
                    { "type": "text", "text": "response" },
                ]),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › text › should ignore citation content when generated content is %s
        #[tokio::test]
        async fn should_ignore_citation_content_when_generated_content_is_s() {
            // `undefined` leaves the key out; `null` sends it as null.
            for generated_content in [None, Some(Value::Null)] {
                let mut citations_content = json!({ "citations": [] });
                if let Some(value) = generated_content {
                    citations_content["content"] = value;
                }
                let g = generate(
                    MODEL_ID,
                    citations_response(citations_content),
                    json!({ "prompt": test_prompt() }),
                )
                .await;

                assert_json_eq(&g.content(), &json!([]));
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › text › should ignore citation generated content without text
        #[tokio::test]
        async fn should_ignore_citation_generated_content_without_text() {
            let g = generate(
                MODEL_ID,
                citations_response(json!({
                    "content": [{}, { "text": null }, { "text": "Response" }],
                    "citations": [],
                })),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            assert_json_eq(
                &g.content(),
                &json!([{ "type": "text", "text": "Response" }]),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › text › should prefer regular text when citation content is also present
        #[tokio::test]
        async fn should_prefer_regular_text_when_citation_content_is_also_present() {
            let g = generate(
                MODEL_ID,
                json_response(&json!({
                    "output": {
                        "message": {
                            "role": "assistant",
                            "content": [
                                {
                                    "text": "Canonical response",
                                    "citationsContent": {
                                        "content": [{ "text": "Duplicate response" }],
                                        "citations": [],
                                    },
                                },
                            ],
                        },
                    },
                    "stopReason": "end_turn",
                    "usage": { "inputTokens": 1, "outputTokens": 2, "totalTokens": 3 },
                })),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            assert_json_eq(
                &g.content(),
                &json!([{ "type": "text", "text": "Canonical response" }]),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › text › should extract usage
        #[tokio::test]
        async fn should_extract_usage() {
            let g = generate(
                MODEL_ID,
                text_response(),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            assert_json_eq(
                &to_json(&g.result.usage),
                &json!({
                    "inputTokens": { "cacheRead": 0, "cacheWrite": 0, "noCache": 22, "total": 22 },
                    "outputTokens": { "text": 57, "total": 57 },
                    "raw": {
                        "cacheReadInputTokenCount": 0,
                        "cacheReadInputTokens": 0,
                        "cacheWriteInputTokenCount": 0,
                        "cacheWriteInputTokens": 0,
                        "inputTokens": 22,
                        "outputTokens": 57,
                        "serverToolUsage": {},
                        "totalTokens": 79,
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › text › should send additional response information
        #[tokio::test]
        async fn should_send_additional_response_information() {
            let g = generate(
                MODEL_ID,
                text_response(),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            let response = g.result.response.as_ref().expect("response");
            assert_eq!(response.id, None);
            assert_eq!(response.timestamp, None);
            assert_eq!(response.model_id.as_deref(), Some(MODEL_ID));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › text › should expose the raw response headers
        #[tokio::test]
        async fn should_expose_the_raw_response_headers() {
            let g = generate(
                MODEL_ID,
                json_response_with_headers(
                    &fixture("amazon-bedrock-text"),
                    &[("test-header", "test-value")],
                ),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            assert_json_eq(
                &to_json(&g.result.response.as_ref().unwrap().headers),
                &json!({
                    "content-length": "435",
                    "content-type": "application/json",
                    "test-header": "test-value",
                }),
            );
        }
    }

    mod reasoning {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › reasoning › should extract reasoning and text response
        #[tokio::test]
        async fn should_extract_reasoning_and_text_response() {
            let g = generate(
                MODEL_ID,
                fixture_response("amazon-bedrock-reasoning"),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            let snapshot: Value = serde_json::from_str(include_str!(
                "snapshots/do_generate_reasoning_should_extract_reasoning_and_text_response.json"
            ))
            .unwrap();
            assert_json_eq(&to_json(&g.result), &snapshot);
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should extract finish reason
    #[tokio::test]
    async fn should_extract_finish_reason() {
        let g = generate(
            MODEL_ID,
            hello_world_response(json!({})),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.finish_reason(),
            &json!({ "raw": "stop_sequence", "unified": "stop" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should support unknown finish reason
    #[tokio::test]
    async fn should_support_unknown_finish_reason() {
        let g = generate(
            MODEL_ID,
            hello_world_response(json!({ "stopReason": "eos" })),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.finish_reason(),
            &json!({ "raw": "eos", "unified": "other" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should pass the model and the messages
    #[tokio::test]
    async fn should_pass_the_model_and_the_messages() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.request_body(),
            &json!({
                "additionalModelResponseFieldPaths": ["/delta/stop_sequence"],
                "messages": [{ "content": [{ "text": "Hello" }], "role": "user" }],
                "system": [{ "text": "System Prompt" }],
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should pass settings
    #[tokio::test]
    async fn should_pass_settings() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": test_prompt(),
                "maxOutputTokens": 100,
                "temperature": 0.5,
                "topP": 0.5,
                "topK": 1,
            }),
        )
        .await;

        assert_match_object(
            &g.request_body(),
            &json!({
                "inferenceConfig": { "maxTokens": 100, "temperature": 0.5, "topP": 0.5, "topK": 1 },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should omit unsupported sampling parameters for %s
    #[tokio::test]
    async fn should_omit_unsupported_sampling_parameters_for_s() {
        for sampling_model_id in [
            "global.anthropic.claude-opus-4-7",
            "eu.anthropic.claude-opus-4-8",
            "us.anthropic.claude-opus-5",
        ] {
            let g = generate(
                sampling_model_id,
                json_response(&json!({
                    "output": { "message": { "role": "assistant", "content": [{ "text": "OK" }] } },
                    "stopReason": "end_turn",
                    "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 },
                })),
                json!({ "prompt": test_prompt(), "temperature": 0.5, "topP": 0.7, "topK": 10 }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(at(&body, "/inferenceConfig/temperature"), None);
            assert_eq!(at(&body, "/inferenceConfig/topP"), None);
            assert_eq!(at(&body, "/inferenceConfig/topK"), None);
            assert_json_eq(
                &g.warnings(),
                &json!([
                    {
                        "type": "unsupported",
                        "feature": "temperature",
                        "details": format!("temperature is not supported by {sampling_model_id} and will be ignored"),
                    },
                    {
                        "type": "unsupported",
                        "feature": "topK",
                        "details": format!("topK is not supported by {sampling_model_id} and will be ignored"),
                    },
                    {
                        "type": "unsupported",
                        "feature": "topP",
                        "details": format!("topP is not supported by {sampling_model_id} and will be ignored"),
                    },
                ]),
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should support guardrails
    #[tokio::test]
    async fn should_support_guardrails() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    "bedrock": {
                        "guardrailConfig": { "guardrailIdentifier": "-1", "guardrailVersion": "1", "trace": "enabled" },
                    },
                },
            }),
        )
        .await;

        assert_match_object(
            &g.request_body(),
            &json!({
                "guardrailConfig": { "guardrailIdentifier": "-1", "guardrailVersion": "1", "trace": "enabled" },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should send text parts as guardContent blocks when providerOptions.bedrock.guardContent is set
    #[tokio::test]
    async fn should_send_text_parts_as_guard_content_blocks() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "London is the capital of UK. Tokyo is the capital of Japan.",
                                "providerOptions": {
                                    "bedrock": { "guardContent": true, "guardContentQualifiers": ["grounding_source"] },
                                },
                            },
                            { "type": "text", "text": "Some additional background information." },
                            {
                                "type": "text",
                                "text": "What is the capital of Japan?",
                                "providerOptions": {
                                    "bedrock": { "guardContent": true, "guardContentQualifiers": ["query"] },
                                },
                            },
                        ],
                    },
                ],
                "providerOptions": {
                    "bedrock": {
                        "guardrailConfig": { "guardrailIdentifier": "-1", "guardrailVersion": "1", "trace": "enabled" },
                    },
                },
            }),
        )
        .await;

        assert_match_object(
            &g.request_body(),
            &json!({
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "guardContent": {
                                    "text": {
                                        "text": "London is the capital of UK. Tokyo is the capital of Japan.",
                                        "qualifiers": ["grounding_source"],
                                    },
                                },
                            },
                            { "text": "Some additional background information." },
                            {
                                "guardContent": {
                                    "text": { "text": "What is the capital of Japan?", "qualifiers": ["query"] },
                                },
                            },
                        ],
                    },
                ],
                "guardrailConfig": { "guardrailIdentifier": "-1", "guardrailVersion": "1", "trace": "enabled" },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should send image parts as guardContent blocks when providerOptions.bedrock.guardContent is set
    #[tokio::test]
    async fn should_send_image_parts_as_guard_content_blocks() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "file",
                                "data": { "type": "data", "data": "AAECAw==" },
                                "mediaType": "image/png",
                                "providerOptions": { "bedrock": { "guardContent": true } },
                            },
                            { "type": "text", "text": "Describe this image." },
                        ],
                    },
                ],
            }),
        )
        .await;

        assert_match_object(
            &g.request_body(),
            &json!({
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            { "guardContent": { "image": { "format": "png", "source": { "bytes": "AAECAw==" } } } },
                            { "text": "Describe this image." },
                        ],
                    },
                ],
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should surface guardrail intervention as a content-filter finish reason with trace metadata
    #[tokio::test]
    async fn should_surface_guardrail_intervention_as_a_content_filter_finish_reason() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-guard-content-intervened"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Tell me something restricted.",
                                "providerOptions": {
                                    "bedrock": { "guardContent": true, "guardContentQualifiers": ["query"] },
                                },
                            },
                        ],
                    },
                ],
                "providerOptions": {
                    "bedrock": {
                        "guardrailConfig": {
                            "guardrailIdentifier": "1abcd2ef34gh",
                            "guardrailVersion": "1",
                            "trace": "enabled",
                        },
                    },
                },
            }),
        )
        .await;

        assert_json_eq(
            &g.finish_reason(),
            &json!({ "raw": "guardrail_intervened", "unified": "content-filter" }),
        );
        assert_json_eq(
            &g.content(),
            &json!([{ "text": "Sorry, the model cannot answer this question.", "type": "text" }]),
        );
        assert_json_eq(
            &g.bedrock_metadata("/trace").expect("trace"),
            &json!({
                "guardrail": {
                    "inputAssessment": {
                        "1abcd2ef34gh": {
                            "topicPolicy": {
                                "topics": [{ "action": "BLOCKED", "name": "Restricted topic", "type": "DENY" }],
                            },
                        },
                    },
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include trace information in providerMetadata
    #[tokio::test]
    async fn should_include_trace_information_in_provider_metadata() {
        let g = generate(
            MODEL_ID,
            hello_world_response(json!({ "trace": mock_trace() })),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_match_object(&g.bedrock_metadata("/trace").expect("trace"), &mock_trace());
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include stop_sequence in provider metadata
    #[tokio::test]
    async fn should_include_stop_sequence_in_provider_metadata() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "Hello, World!" }] } },
                "stopReason": "stop_sequence",
                "additionalModelResponseFields": { "delta": { "stop_sequence": "STOP" } },
                "usage": { "inputTokens": 4, "outputTokens": 30, "totalTokens": 34 },
            })),
            json!({ "prompt": test_prompt(), "stopSequences": ["STOP"] }),
        )
        .await;

        assert_json_eq(
            &to_json(&g.result.provider_metadata),
            &json!({
                "amazonBedrock": { "stopSequence": "STOP" },
                "bedrock": { "stopSequence": "STOP" },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should handle stop_sequence: null when stopReason is tool_use
    #[tokio::test]
    async fn should_handle_stop_sequence_null_when_stop_reason_is_tool_use() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": {
                    "message": {
                        "role": "assistant",
                        "content": [
                            { "text": "I'll query your Salesforce..." },
                            {
                                "toolUse": {
                                    "toolUseId": "tool-use-id",
                                    "name": "querySalesforce",
                                    "input": { "query": "SELECT Id FROM Account" },
                                },
                            },
                        ],
                    },
                },
                "stopReason": "tool_use",
                "additionalModelResponseFields": { "delta": { "stop_sequence": null } },
                "usage": { "inputTokens": 10, "outputTokens": 20, "totalTokens": 30 },
            })),
            json!({
                "prompt": test_prompt(),
                "tools": [
                    {
                        "type": "function",
                        "name": "querySalesforce",
                        "description": "Query Salesforce",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "query": { "type": "string" } },
                            "required": ["query"],
                            "additionalProperties": false,
                        },
                    },
                ],
            }),
        )
        .await;

        assert_json_eq(
            &g.finish_reason(),
            &json!({ "unified": "tool-calls", "raw": "tool_use" }),
        );
        assert_json_eq(
            &g.content(),
            &json!([
                { "text": "I'll query your Salesforce...", "type": "text" },
                {
                    "input": "{\"query\":\"SELECT Id FROM Account\"}",
                    "toolCallId": "tool-use-id",
                    "toolName": "querySalesforce",
                    "type": "tool-call",
                },
            ]),
        );
        assert_json_eq(
            &to_json(&g.result.provider_metadata),
            &json!({
                "amazonBedrock": { "stopSequence": null },
                "bedrock": { "stopSequence": null },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should correctly parse delta.stop_sequence structure from additionalModelResponseFields
    #[tokio::test]
    async fn should_correctly_parse_delta_stop_sequence_structure() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "Response text" }] } },
                "stopReason": "stop_sequence",
                "additionalModelResponseFields": {
                    "delta": { "stop_sequence": "CUSTOM_STOP" },
                    "otherField": "value",
                },
                "usage": { "inputTokens": 10, "outputTokens": 20, "totalTokens": 30 },
            })),
            json!({ "prompt": test_prompt(), "stopSequences": ["CUSTOM_STOP"] }),
        )
        .await;

        assert_eq!(
            g.bedrock_metadata("/stopSequence"),
            Some(json!("CUSTOM_STOP"))
        );
        assert_json_eq(
            &g.finish_reason(),
            &json!({ "unified": "stop", "raw": "stop_sequence" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include response headers in rawResponse
    #[tokio::test]
    async fn should_include_response_headers_in_raw_response() {
        let g = generate(
            MODEL_ID,
            json_response_with_headers(
                &json!({
                    "output": { "message": { "role": "assistant", "content": [{ "text": "Testing" }] } },
                    "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
                    "stopReason": "stop_sequence",
                }),
                &[
                    ("x-amzn-requestid", "test-request-id"),
                    ("x-amzn-trace-id", "test-trace-id"),
                ],
            ),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &to_json(&g.result.response.as_ref().unwrap().headers),
            &json!({
                "x-amzn-requestid": "test-request-id",
                "x-amzn-trace-id": "test-trace-id",
                "content-type": "application/json",
                "content-length": "164",
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should extract response metadata
    #[tokio::test]
    async fn should_extract_response_metadata() {
        let g = generate(
            MODEL_ID,
            json_response_with_headers(
                &json!({
                    "output": { "message": { "role": "assistant", "content": [{ "text": "Testing" }] } },
                    "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
                    "stopReason": "end_turn",
                }),
                &[
                    ("x-amzn-requestid", "test-request-id"),
                    ("date", "Wed, 01 Jan 2025 00:00:00 GMT"),
                ],
            ),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        let response = g.result.response.as_ref().unwrap();
        assert_eq!(response.id.as_deref(), Some("test-request-id"));
        assert_eq!(
            response.timestamp.map(|timestamp| timestamp.to_string()),
            Some("2025-01-01T00:00:00.000Z".to_string())
        );
        assert_eq!(response.model_id.as_deref(), Some(MODEL_ID));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should pass tools and tool choice correctly
    #[tokio::test]
    async fn should_pass_tools_and_tool_choice_correctly() {
        let schema = json!({
            "type": "object",
            "properties": { "param1": { "type": "string" }, "param2": { "type": "number" } },
            "required": ["param1"],
            "additionalProperties": false,
        });
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "tools": [
                    { "type": "function", "name": "test-tool-1", "description": "A test tool", "inputSchema": schema },
                ],
                "toolChoice": { "type": "auto" },
                "prompt": test_prompt(),
            }),
        )
        .await;

        assert_match_object(
            &g.request_body(),
            &json!({
                "toolConfig": {
                    "tools": [
                        {
                            "toolSpec": {
                                "name": "test-tool-1",
                                "description": "A test tool",
                                "inputSchema": { "json": schema },
                            },
                        },
                    ],
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should disable parallel tool use without sending conflicting tool choices
    #[tokio::test]
    async fn should_disable_parallel_tool_use_without_sending_conflicting_tool_choices() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "tools": [
                    {
                        "type": "function",
                        "name": "test-tool",
                        "description": "A test tool",
                        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
                    },
                ],
                "toolChoice": { "type": "auto" },
                "prompt": test_prompt(),
                "providerOptions": { "anthropic": { "disableParallelToolUse": true } },
            }),
        )
        .await;

        let body = g.request_body();
        assert_match_object(
            &body["additionalModelRequestFields"],
            &json!({ "tool_choice": { "type": "auto", "disable_parallel_tool_use": true } }),
        );
        assert_eq!(at(&body, "/toolConfig/toolChoice"), None);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should only send the forced tool when toolChoice specifies a specific tool
    #[tokio::test]
    async fn should_only_send_the_forced_tool_when_tool_choice_specifies_a_specific_tool() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "tools": [
                    {
                        "type": "function",
                        "name": "getWeatherByCity",
                        "description": "Get weather by city",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "city": { "type": "string" } },
                            "required": ["city"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "type": "function",
                        "name": "getCurrentWeather",
                        "description": "Get current weather",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "location": { "type": "string" } },
                            "required": ["location"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "type": "function",
                        "name": "getWeatherForecast",
                        "description": "Get forecast",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "location": { "type": "string" }, "days": { "type": "number" } },
                            "required": ["location", "days"],
                            "additionalProperties": false,
                        },
                    },
                ],
                "toolChoice": { "type": "tool", "toolName": "getWeatherByCity" },
                "prompt": test_prompt(),
            }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(tool_names(&body), vec!["getWeatherByCity"]);
        assert_json_eq(
            &body["toolConfig"]["toolChoice"],
            &json!({ "tool": { "name": "getWeatherByCity" } }),
        );
    }

    mod models_that_reject_forced_tool_use {
        use super::*;

        fn weather_tool() -> Value {
            json!({ "type": "function", "name": "getWeather", "description": "Get weather", "inputSchema": { "type": "object" } })
        }

        fn time_tool() -> Value {
            json!({ "type": "function", "name": "getTime", "description": "Get time", "inputSchema": { "type": "object" } })
        }

        /// The response of upstream `createModelThatCapturesRequest`'s fetch.
        fn done_response() -> Canned {
            json_response(&json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "Done" }] } },
                "stopReason": "end_turn",
                "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 },
            }))
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › models that reject forced tool use › should build an auto tool choice for required choice on %s
        #[tokio::test]
        async fn should_build_an_auto_tool_choice_for_required_choice_on_s() {
            for model_id in [
                "anthropic.claude-opus-5-5",
                "us.anthropic.claude-opus-5-5",
                "global.anthropic.claude-opus-5-5",
                "anthropic.claude-sonnet-5-5",
                "us.anthropic.claude-sonnet-5-5",
            ] {
                let g = generate(
                    model_id,
                    done_response(),
                    json!({
                        "tools": [weather_tool()],
                        "toolChoice": { "type": "required" },
                        "prompt": test_prompt(),
                    }),
                )
                .await;

                assert_json_eq(
                    &g.request_body()["toolConfig"]["toolChoice"],
                    &json!({ "auto": {} }),
                );
                assert_json_eq(
                    &g.warnings(),
                    &json!([
                        {
                            "type": "unsupported",
                            "feature": "toolChoice",
                            "details": "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.",
                        },
                    ]),
                );
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › models that reject forced tool use › should build an auto choice containing only the named tool
        #[tokio::test]
        async fn should_build_an_auto_choice_containing_only_the_named_tool() {
            let g = generate(
                OPUS55_ANTHROPIC_MODEL_ID,
                done_response(),
                json!({
                    "tools": [weather_tool(), time_tool()],
                    "toolChoice": { "type": "tool", "toolName": "getWeather" },
                    "prompt": test_prompt(),
                }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(&body["toolConfig"]["toolChoice"], &json!({ "auto": {} }));
            assert_eq!(tool_names(&body), vec!["getWeather"]);
            assert_json_eq(
                &g.warnings(),
                &json!([
                    {
                        "type": "unsupported",
                        "feature": "toolChoice",
                        "details": "toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the 'getWeather' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made.",
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › models that reject forced tool use › should build an Anthropic auto choice when parallel tool use is disabled
        #[tokio::test]
        async fn should_build_an_anthropic_auto_choice_when_parallel_tool_use_is_disabled() {
            let g = generate(
                OPUS55_ANTHROPIC_MODEL_ID,
                done_response(),
                json!({
                    "tools": [weather_tool()],
                    "toolChoice": { "type": "required" },
                    "prompt": test_prompt(),
                    "providerOptions": { "anthropic": { "disableParallelToolUse": true } },
                }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/tool_choice").expect("tool_choice"),
                &json!({ "type": "auto", "disable_parallel_tool_use": true }),
            );
            assert_eq!(at(&body, "/toolConfig/toolChoice"), None);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › models that reject forced tool use › should keep building forced tool choices for models that support them
        #[tokio::test]
        async fn should_keep_building_forced_tool_choices_for_models_that_support_them() {
            let g = generate(
                OPUS5_ANTHROPIC_MODEL_ID,
                done_response(),
                json!({
                    "tools": [weather_tool()],
                    "toolChoice": { "type": "required" },
                    "prompt": test_prompt(),
                }),
            )
            .await;

            assert_json_eq(
                &g.request_body()["toolConfig"]["toolChoice"],
                &json!({ "any": {} }),
            );
            assert_json_eq(&g.warnings(), &json!([]));
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should send all tools when toolChoice is auto
    #[tokio::test]
    async fn should_send_all_tools_when_tool_choice_is_auto() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "tools": [
                    {
                        "type": "function",
                        "name": "getWeatherByCity",
                        "description": "Get weather by city",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "city": { "type": "string" } },
                            "required": ["city"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "type": "function",
                        "name": "getCurrentWeather",
                        "description": "Get current weather",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "location": { "type": "string" } },
                            "required": ["location"],
                            "additionalProperties": false,
                        },
                    },
                ],
                "toolChoice": { "type": "auto" },
                "prompt": test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            tool_names(&g.request_body()),
            vec!["getWeatherByCity", "getCurrentWeather"]
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should omit empty tool descriptions to avoid Bedrock validation errors
    #[tokio::test]
    async fn should_omit_empty_tool_descriptions_to_avoid_bedrock_validation_errors() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "tools": [
                    {
                        "type": "function",
                        "name": "tool-with-empty-desc",
                        "description": "",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "param1": { "type": "string" } },
                            "required": ["param1"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "type": "function",
                        "name": "tool-with-whitespace-desc",
                        "description": "   ",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "param2": { "type": "number" } },
                            "required": ["param2"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "type": "function",
                        "name": "tool-with-valid-desc",
                        "description": "Valid description",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "param3": { "type": "boolean" } },
                            "required": ["param3"],
                            "additionalProperties": false,
                        },
                    },
                ],
                "toolChoice": { "type": "auto" },
                "prompt": test_prompt(),
            }),
        )
        .await;

        let body = g.request_body();
        let tools = &body["toolConfig"]["tools"];
        assert_eq!(tools[0]["toolSpec"].get("description"), None);
        assert_eq!(tools[0]["toolSpec"]["name"], "tool-with-empty-desc");
        assert_eq!(tools[1]["toolSpec"].get("description"), None);
        assert_eq!(tools[1]["toolSpec"]["name"], "tool-with-whitespace-desc");
        assert_eq!(tools[2]["toolSpec"]["description"], "Valid description");
        assert_eq!(tools[2]["toolSpec"]["name"], "tool-with-valid-desc");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should handle Anthropic provider-defined tools
    #[tokio::test]
    async fn should_handle_anthropic_provider_defined_tools() {
        // Upstream mocks Anthropic `prepareTools` to return what the real
        // function returns for `bash_20241022` (tool choice `auto`, beta
        // `computer-use-2024-10-22`); aimux runs the real one.
        let g = generate(
            ANTHROPIC_MODEL_ID,
            json_response(&json!({
                "output": {
                    "message": {
                        "role": "assistant",
                        "content": [
                            { "toolUse": { "toolUseId": "tool-use-id", "name": "bash", "input": { "command": "ls -l" } } },
                        ],
                    },
                },
                "usage": { "inputTokens": 10, "outputTokens": 20, "totalTokens": 30 },
                "stopReason": "tool_use",
            })),
            json!({
                "prompt": test_prompt(),
                "tools": [bash_tool()],
                "toolChoice": { "type": "auto" },
            }),
        )
        .await;

        let body = g.request_body();
        assert_json_eq(
            &body["additionalModelRequestFields"],
            &json!({
                "tool_choice": { "type": "auto" },
                "anthropic_beta": ["computer-use-2024-10-22"],
            }),
        );
        assert!(body.get("toolConfig").is_some());
        assert_eq!(tool_names(&body), vec!["bash"]);
        assert_json_eq(
            &body["toolConfig"]["tools"][0]["toolSpec"]["inputSchema"]["json"],
            &json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": { "command": { "type": "string" }, "restart": { "type": "boolean" } },
                "required": ["command"],
                "additionalProperties": false,
            }),
        );
        assert_json_eq(&g.warnings(), &json!([]));
        assert_json_eq(
            &g.content(),
            &json!([
                {
                    "input": "{\"command\":\"ls -l\"}",
                    "toolCallId": "tool-use-id",
                    "toolName": "bash",
                    "type": "tool-call",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include anthropic_beta in additionalModelRequestFields when using extended context
    #[tokio::test]
    async fn should_include_anthropic_beta_in_additional_model_request_fields() {
        let g = generate(
            ANTHROPIC_MODEL_ID,
            test_response_stop(),
            json!({
                "prompt": test_prompt(),
                "providerOptions": { "bedrock": { "anthropicBeta": ["context-1m-2025-08-07"] } },
            }),
        )
        .await;

        assert_json_eq(
            &g.request_body()["additionalModelRequestFields"],
            &json!({ "anthropic_beta": ["context-1m-2025-08-07"] }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should not include anthropic-beta in HTTP headers
    #[tokio::test]
    async fn should_not_include_anthropic_beta_in_http_headers() {
        let g = generate(
            ANTHROPIC_MODEL_ID,
            test_response_stop(),
            json!({ "prompt": test_prompt(), "tools": [bash_tool()] }),
        )
        .await;

        assert_eq!(g.request_headers().get("anthropic-beta"), None);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should combine user-provided and tool-generated betas in body
    #[tokio::test]
    async fn should_combine_user_provided_and_tool_generated_betas_in_body() {
        let g = generate(
            ANTHROPIC_MODEL_ID,
            json_response(&json!({
                "output": {
                    "message": {
                        "role": "assistant",
                        "content": [
                            { "type": "tool_use", "id": "tool-use-id", "name": "bash", "input": { "command": "ls -l" } },
                        ],
                    },
                },
                "usage": { "inputTokens": 10, "outputTokens": 20, "totalTokens": 30 },
                "stopReason": "tool_use",
            })),
            json!({
                "prompt": test_prompt(),
                "tools": [bash_tool()],
                "providerOptions": { "bedrock": { "anthropicBeta": ["context-1m-2025-08-07"] } },
            }),
        )
        .await;

        assert_json_eq(
            &g.request_body()["additionalModelRequestFields"]["anthropic_beta"],
            &json!(["context-1m-2025-08-07", "computer-use-2024-10-22"]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should properly combine headers from all sources
    #[tokio::test]
    async fn should_properly_combine_headers_from_all_sources() {
        let fetch = MockFetch::new(vec![fixture_response("amazon-bedrock-text")]);
        let inject: FetchFunction = Arc::new(InjectHeaders {
            headers: vec![
                ("options-header", "options-value"),
                ("model-header", "model-value"),
                ("shared-header", "options-shared"),
                ("signed-header", "signed-value"),
                ("authorization", "AWS4-HMAC-SHA256..."),
            ],
            inner: fetch.transport(),
        });
        let model_headers: HeaderMapOpt = [
            ("model-header".to_string(), Some("model-value".to_string())),
            (
                "shared-header".to_string(),
                Some("model-shared".to_string()),
            ),
        ]
        .into_iter()
        .collect();
        let g = generate_with(
            AmazonBedrockProviderSettings {
                headers: Some(model_headers),
                ..settings(inject)
            },
            &fetch,
            MODEL_ID,
            json!({
                "prompt": test_prompt(),
                "headers": { "options-header": "options-value", "shared-header": "options-shared" },
            }),
        )
        .await;

        let headers = g.request_headers();
        assert_eq!(headers["options-header"], "options-value");
        assert_eq!(headers["model-header"], "model-value");
        assert_eq!(headers["signed-header"], "signed-value");
        assert_eq!(headers["authorization"], "AWS4-HMAC-SHA256...");
        assert_eq!(headers["shared-header"], "options-shared");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should work with partial headers
    #[tokio::test]
    async fn should_work_with_partial_headers() {
        let fetch = MockFetch::new(vec![fixture_response("amazon-bedrock-text")]);
        let inject: FetchFunction = Arc::new(InjectHeaders {
            headers: vec![
                ("model-header", "model-value"),
                ("signed-header", "signed-value"),
                ("authorization", "AWS4-HMAC-SHA256..."),
            ],
            inner: fetch.transport(),
        });
        let model_headers: HeaderMapOpt =
            [("model-header".to_string(), Some("model-value".to_string()))]
                .into_iter()
                .collect();
        let g = generate_with(
            AmazonBedrockProviderSettings {
                headers: Some(model_headers),
                ..settings(inject)
            },
            &fetch,
            MODEL_ID,
            json!({ "prompt": test_prompt() }),
        )
        .await;

        let headers = g.request_headers();
        assert_eq!(headers["model-header"], "model-value");
        assert_eq!(headers["signed-header"], "signed-value");
        assert_eq!(headers["authorization"], "AWS4-HMAC-SHA256...");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include providerOptions in the request for generate calls
    #[tokio::test]
    async fn should_include_provider_options_in_the_request_for_generate_calls() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({ "prompt": test_prompt(), "providerOptions": { "bedrock": { "foo": "bar" } } }),
        )
        .await;

        assert_match_object(&g.request_body(), &json!({ "foo": "bar" }));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include cache token usage in providerMetadata
    #[tokio::test]
    async fn should_include_cache_token_usage_in_provider_metadata() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": { "message": { "role": "assistant", "content": [{ "type": "text", "text": "Testing" }] } },
                "usage": {
                    "inputTokens": 4,
                    "outputTokens": 34,
                    "totalTokens": 38,
                    "cacheReadInputTokens": 2,
                    "cacheWriteInputTokens": 3,
                },
                "stopReason": "stop_sequence",
            })),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &to_json(&g.result.provider_metadata),
            &json!({
                "amazonBedrock": { "stopSequence": null, "usage": { "cacheWriteInputTokens": 3 } },
                "bedrock": { "stopSequence": null, "usage": { "cacheWriteInputTokens": 3 } },
            }),
        );
        assert_json_eq(
            &to_json(&g.result.usage),
            &json!({
                "inputTokens": { "cacheRead": 2, "cacheWrite": 3, "noCache": 4, "total": 9 },
                "outputTokens": { "text": 34, "total": 34 },
                "raw": {
                    "cacheReadInputTokens": 2,
                    "cacheWriteInputTokens": 3,
                    "inputTokens": 4,
                    "outputTokens": 34,
                    "totalTokens": 38,
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include performanceConfig in providerMetadata
    #[tokio::test]
    async fn should_include_performance_config_in_provider_metadata() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": { "message": { "role": "assistant", "content": [{ "type": "text", "text": "Hello" }] } },
                "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
                "stopReason": "end_turn",
                "performanceConfig": { "latency": "optimized" },
            })),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &to_json(&g.result.provider_metadata),
            &json!({
                "amazonBedrock": { "performanceConfig": { "latency": "optimized" }, "stopSequence": null },
                "bedrock": { "performanceConfig": { "latency": "optimized" }, "stopSequence": null },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include serviceTier in providerMetadata
    #[tokio::test]
    async fn should_include_service_tier_in_provider_metadata() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": { "message": { "role": "assistant", "content": [{ "type": "text", "text": "Hello" }] } },
                "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
                "stopReason": "end_turn",
                "serviceTier": { "type": "on-demand" },
            })),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &to_json(&g.result.provider_metadata),
            &json!({
                "amazonBedrock": { "serviceTier": { "type": "on-demand" }, "stopSequence": null },
                "bedrock": { "serviceTier": { "type": "on-demand" }, "stopSequence": null },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include cacheDetails in providerMetadata
    #[tokio::test]
    async fn should_include_cache_details_in_provider_metadata() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": { "message": { "role": "assistant", "content": [{ "type": "text", "text": "Hello" }] } },
                "usage": {
                    "inputTokens": 4,
                    "outputTokens": 34,
                    "totalTokens": 38,
                    "cacheDetails": [{ "inputTokens": 100, "ttl": "T5M" }],
                },
                "stopReason": "end_turn",
            })),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &to_json(&g.result.provider_metadata),
            &json!({
                "amazonBedrock": {
                    "stopSequence": null,
                    "usage": { "cacheDetails": [{ "inputTokens": 100, "ttl": "T5M" }] },
                },
                "bedrock": {
                    "stopSequence": null,
                    "usage": { "cacheDetails": [{ "inputTokens": 100, "ttl": "T5M" }] },
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should handle system messages with cache points
    #[tokio::test]
    async fn should_handle_system_messages_with_cache_points() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": [
                    {
                        "role": "system",
                        "content": "System Prompt",
                        "providerOptions": { "bedrock": { "cachePoint": { "type": "default" } } },
                    },
                    { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                ],
            }),
        )
        .await;

        assert_match_object(
            &g.request_body(),
            &json!({
                "system": [{ "text": "System Prompt" }, { "cachePoint": { "type": "default" } }],
                "messages": [{ "role": "user", "content": [{ "text": "Hello" }] }],
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should transform reasoningConfig to thinking in additionalModelRequestFields
    #[tokio::test]
    async fn should_transform_reasoning_config_to_thinking_in_additional_model_request_fields() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": test_prompt(),
                "maxOutputTokens": 100,
                "providerOptions": {
                    "bedrock": { "reasoningConfig": { "type": "enabled", "budgetTokens": 2000 } },
                },
            }),
        )
        .await;

        let body = g.request_body();
        assert_match_object(
            &body,
            &json!({
                "additionalModelRequestFields": { "thinking": { "type": "enabled", "budget_tokens": 2000 } },
                "inferenceConfig": { "maxTokens": 2100 },
            }),
        );
        assert_eq!(body.get("reasoningConfig"), None);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › merges user additionalModelRequestFields with derived thinking (generate)
    #[tokio::test]
    async fn merges_user_additional_model_request_fields_with_derived_thinking_generate() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    "bedrock": {
                        "reasoningConfig": { "type": "enabled", "budgetTokens": 1234 },
                        "additionalModelRequestFields": { "foo": "bar", "custom": 42 },
                    },
                },
            }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(body.get("reasoningConfig"), None);
        assert_match_object(
            &body["additionalModelRequestFields"],
            &json!({
                "foo": "bar",
                "custom": 42,
                "thinking": { "type": "enabled", "budget_tokens": 1234 },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should forward display in adaptive reasoningConfig to thinking
    #[tokio::test]
    async fn should_forward_display_in_adaptive_reasoning_config_to_thinking() {
        let g = generate(
            NEWER_ANTHROPIC_MODEL_ID,
            hello_response("stop_sequence"),
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    "bedrock": { "reasoningConfig": { "type": "adaptive", "display": "summarized" } },
                },
            }),
        )
        .await;

        assert_json_eq(
            at(&g.request_body(), "/additionalModelRequestFields/thinking").expect("thinking"),
            &json!({ "type": "adaptive", "display": "summarized" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should pass serviceTier provider option in generate requests
    #[tokio::test]
    async fn should_pass_service_tier_provider_option_in_generate_requests() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({ "prompt": test_prompt(), "providerOptions": { "bedrock": { "serviceTier": "priority" } } }),
        )
        .await;

        let body = g.request_body();
        assert_match_object(&body, &json!({ "serviceTier": { "type": "priority" } }));
        assert_eq!(at(&body, "/additionalModelRequestFields/serviceTier"), None);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › maps maxReasoningEffort for Nova without thinking (generate)
    #[tokio::test]
    async fn maps_max_reasoning_effort_for_nova_without_thinking_generate() {
        let g = generate(
            NOVA_MODEL_ID,
            hello_response("stop_sequence"),
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    "bedrock": {
                        "reasoningConfig": { "type": "enabled", "maxReasoningEffort": "medium", "budgetTokens": 2048 },
                    },
                },
            }),
        )
        .await;

        let body = g.request_body();
        assert_match_object(
            &body,
            &json!({
                "additionalModelRequestFields": {
                    "reasoningConfig": { "type": "enabled", "maxReasoningEffort": "medium" },
                },
            }),
        );
        assert_eq!(at(&body, "/additionalModelRequestFields/thinking"), None);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › maps maxReasoningEffort to reasoning_effort for OpenAI gpt-oss models (generate)
    #[tokio::test]
    async fn maps_max_reasoning_effort_to_reasoning_effort_for_openai_gpt_oss_models() {
        let g = generate(
            OPENAI_MODEL_ID,
            hello_response("stop_sequence"),
            json!({
                "prompt": test_prompt(),
                "providerOptions": { "bedrock": { "reasoningConfig": { "maxReasoningEffort": "medium" } } },
            }),
        )
        .await;

        let body = g.request_body();
        assert_match_object(
            &body,
            &json!({ "additionalModelRequestFields": { "reasoning_effort": "medium" } }),
        );
        assert_eq!(
            at(&body, "/additionalModelRequestFields/reasoningConfig"),
            None
        );
        assert_eq!(at(&body, "/additionalModelRequestFields/thinking"), None);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › maps maxReasoningEffort to nested reasoning.effort for CRIS model %s (generate)
    #[tokio::test]
    async fn maps_max_reasoning_effort_to_nested_reasoning_effort_for_cris_model_s() {
        for cris_model_id in [US_OPENAI_MODEL_ID, GLOBAL_OPENAI_MODEL_ID] {
            let g = generate(
                cris_model_id,
                hello_response("stop_sequence"),
                json!({
                    "prompt": test_prompt(),
                    "providerOptions": { "bedrock": { "reasoningConfig": { "maxReasoningEffort": "medium" } } },
                }),
            )
            .await;

            let body = g.request_body();
            assert_match_object(
                &body,
                &json!({ "additionalModelRequestFields": { "reasoning": { "effort": "medium" } } }),
            );
            assert_eq!(
                at(&body, "/additionalModelRequestFields/reasoning_effort"),
                None
            );
            assert_eq!(
                at(&body, "/additionalModelRequestFields/reasoningConfig"),
                None
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › does not classify custom model IDs containing openai. as OpenAI models
    #[tokio::test]
    async fn does_not_classify_custom_model_ids_containing_openai_as_openai_models() {
        let g = generate(
            CUSTOM_OPENAI_SUBSTRING_MODEL_ID,
            hello_response("stop_sequence"),
            json!({
                "prompt": test_prompt(),
                "providerOptions": { "bedrock": { "reasoningConfig": { "maxReasoningEffort": "medium" } } },
            }),
        )
        .await;

        let body = g.request_body();
        assert_match_object(
            &body,
            &json!({
                "additionalModelRequestFields": { "reasoningConfig": { "maxReasoningEffort": "medium" } },
            }),
        );
        assert_eq!(at(&body, "/additionalModelRequestFields/reasoning"), None);
        assert_eq!(
            at(&body, "/additionalModelRequestFields/reasoning_effort"),
            None
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › strips unsupported sampling settings for OpenAI model %s
    #[tokio::test]
    async fn strips_unsupported_sampling_settings_for_openai_model_s() {
        for cris_model_id in [US_OPENAI_MODEL_ID, GLOBAL_OPENAI_MODEL_ID] {
            let g = generate(
                cris_model_id,
                hello_response("end_turn"),
                json!({
                    "prompt": test_prompt(),
                    "temperature": 0,
                    "topP": 0.5,
                    "topK": 5,
                    "stopSequences": [],
                    "maxOutputTokens": 100,
                }),
            )
            .await;

            assert_json_eq(
                &g.request_body()["inferenceConfig"],
                &json!({ "maxTokens": 100, "topK": 5 }),
            );
            assert_json_eq(
                &g.warnings(),
                &json!([
                    {
                        "type": "unsupported",
                        "feature": "temperature",
                        "details": "temperature is not supported by this OpenAI model on the Converse API",
                    },
                    {
                        "type": "unsupported",
                        "feature": "topP",
                        "details": "topP is not supported by this OpenAI model on the Converse API",
                    },
                    {
                        "type": "unsupported",
                        "feature": "stopSequences",
                        "details": "stopSequences is not supported by this OpenAI model on the Converse API",
                    },
                ]),
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › does not warn about clamping an unsupported OpenAI temperature
    #[tokio::test]
    async fn does_not_warn_about_clamping_an_unsupported_openai_temperature() {
        let g = generate(
            US_OPENAI_MODEL_ID,
            hello_response("end_turn"),
            json!({ "prompt": test_prompt(), "temperature": 2 }),
        )
        .await;

        assert_eq!(g.request_body().get("inferenceConfig"), None);
        assert_json_eq(
            &g.warnings(),
            &json!([
                {
                    "type": "unsupported",
                    "feature": "temperature",
                    "details": "temperature is not supported by this OpenAI model on the Converse API",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › keeps supported sampling settings for OpenAI gpt-oss models
    #[tokio::test]
    async fn keeps_supported_sampling_settings_for_openai_gpt_oss_models() {
        let g = generate(
            OPENAI_MODEL_ID,
            hello_response("end_turn"),
            json!({ "prompt": test_prompt(), "temperature": 0.2, "topP": 0.5, "stopSequences": ["END"] }),
        )
        .await;

        assert_json_eq(
            &g.request_body()["inferenceConfig"],
            &json!({ "temperature": 0.2, "topP": 0.5 }),
        );
        assert_json_eq(
            &g.warnings(),
            &json!([
                {
                    "type": "unsupported",
                    "feature": "stopSequences",
                    "details": "stopSequences is not supported by this OpenAI model on the Converse API",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should pass maxReasoningEffort as output_config.effort for Anthropic models (generate)
    #[tokio::test]
    async fn should_pass_max_reasoning_effort_as_output_config_effort_for_anthropic_models() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    "bedrock": { "reasoningConfig": { "type": "enabled", "maxReasoningEffort": "medium" } },
                },
            }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(
            at(&body, "/additionalModelRequestFields/reasoningConfig"),
            None
        );
        assert_json_eq(
            at(&body, "/additionalModelRequestFields/output_config").expect("output_config"),
            &json!({ "effort": "medium" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should use native output_config.format instead of json tool when thinking is enabled with structured output
    #[tokio::test]
    async fn should_use_native_output_config_format_when_thinking_is_enabled() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate a recipe" }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "recipe": {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string" },
                                    "ingredients": { "type": "array", "items": { "type": "string" } },
                                },
                                "required": ["name", "ingredients"],
                            },
                        },
                        "required": ["recipe"],
                    },
                },
                "providerOptions": {
                    "bedrock": { "reasoningConfig": { "type": "enabled", "budgetTokens": 2000 } },
                },
            }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(body.get("toolConfig"), None);
        assert_json_eq(
            at(&body, "/additionalModelRequestFields/output_config").expect("output_config"),
            &json!({
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "recipe": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "name": { "type": "string" },
                                    "ingredients": { "type": "array", "items": { "type": "string" } },
                                },
                                "required": ["name", "ingredients"],
                            },
                        },
                        "required": ["recipe"],
                    },
                },
            }),
        );
        assert!(at(&body, "/additionalModelRequestFields/thinking").is_some());
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should merge output_config.effort and output_config.format when thinking with maxReasoningEffort and structured output
    #[tokio::test]
    async fn should_merge_output_config_effort_and_output_config_format() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate a recipe" }] }],
                "responseFormat": name_response_format(),
                "providerOptions": {
                    "bedrock": {
                        "reasoningConfig": { "type": "enabled", "budgetTokens": 2000, "maxReasoningEffort": "medium" },
                    },
                },
            }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(body.get("toolConfig"), None);
        assert_json_eq(
            at(&body, "/additionalModelRequestFields/output_config").expect("output_config"),
            &json!({
                "effort": "medium",
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": { "name": { "type": "string" } },
                        "required": ["name"],
                    },
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should still use json tool fallback for structured output without thinking enabled
    #[tokio::test]
    async fn should_still_use_json_tool_fallback_for_structured_output_without_thinking() {
        let g = generate(
            MODEL_ID,
            json_tool_name_response(),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate a name" }] }],
                "responseFormat": name_response_format(),
            }),
        )
        .await;

        let body = g.request_body();
        assert!(body.get("toolConfig").is_some());
        assert_eq!(tool_names(&body), vec!["json"]);
        assert_json_eq(&body["toolConfig"]["toolChoice"], &json!({ "any": {} }));
        assert_eq!(
            at(&body, "/additionalModelRequestFields/output_config/format"),
            None
        );
        assert_json_eq(
            &g.content(),
            &json!([{ "text": "{\"name\":\"Test\"}", "type": "text" }]),
        );
        assert_eq!(
            g.bedrock_metadata("/isJsonResponseFromTool"),
            Some(json!(true))
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should use the json tool fallback for claude-opus-5 (Bedrock rejects output_config.format)
    #[tokio::test]
    async fn should_use_the_json_tool_fallback_for_claude_opus_5() {
        let g = generate(
            OPUS5_ANTHROPIC_MODEL_ID,
            json_tool_name_response(),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate a name" }] }],
                "responseFormat": name_response_format(),
            }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(body["toolConfig"]["tools"][0]["toolSpec"]["name"], "json");
        assert_eq!(
            at(&body, "/additionalModelRequestFields/output_config"),
            None
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should use JSON instructions instead of forced tool use for claude-opus-5-5 with structuredOutputMode %s
    #[tokio::test]
    async fn should_use_json_instructions_instead_of_forced_tool_use_for_claude_opus_5_5() {
        for structured_output_mode in [None, Some("jsonTool")] {
            let mut options = json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Return an answer of OK." }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "answer": { "type": "string" } },
                        "required": ["answer"],
                    },
                },
            });
            if let Some(mode) = structured_output_mode {
                options["providerOptions"] =
                    json!({ "amazonBedrock": { "structuredOutputMode": mode } });
            }
            let g = generate(
                OPUS55_ANTHROPIC_MODEL_ID,
                json_response(&json!({
                    "output": { "message": { "role": "assistant", "content": [{ "text": "{\"answer\":\"OK\"}" }] } },
                    "usage": { "inputTokens": 4, "outputTokens": 5, "totalTokens": 9 },
                    "stopReason": "end_turn",
                })),
                options,
            )
            .await;

            let body = g.request_body();
            assert_eq!(body.get("toolConfig"), None);
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config"),
                None
            );
            assert_json_eq(
                &body["system"],
                &json!([
                    {
                        "text": "JSON schema:\n{\"type\":\"object\",\"properties\":{\"answer\":{\"type\":\"string\"}},\"required\":[\"answer\"]}\nYou MUST answer with only a JSON object that matches the JSON schema above. Do not wrap it in markdown fences or include any other text.",
                    },
                ]),
            );
            assert_json_eq(
                &g.content(),
                &json!([{ "type": "text", "text": "{\"answer\":\"OK\"}" }]),
            );
            assert_eq!(g.bedrock_metadata("/isJsonResponseFromTool"), None);
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should default to the json tool for $modelId
    #[tokio::test]
    async fn should_default_to_the_json_tool_for_model_id() {
        for model_id in [NEWER_ANTHROPIC_MODEL_ID, HAIKU45_ANTHROPIC_MODEL_ID] {
            let g = generate(
                model_id,
                json_tool_name_response(),
                json!({ "prompt": test_prompt(), "responseFormat": name_response_format() }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(tool_names(&body), vec!["json"]);
            assert_json_eq(&body["toolConfig"]["toolChoice"], &json!({ "any": {} }));
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config/format"),
                None
            );
            assert_json_eq(
                &g.content(),
                &json!([{ "type": "text", "text": "{\"name\":\"Test\"}" }]),
            );
            assert_eq!(g.finish_reason()["unified"], "stop");
            assert_eq!(
                g.bedrock_metadata("/isJsonResponseFromTool"),
                Some(json!(true))
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should force the json tool wire format with $providerOptionsName provider options
    #[tokio::test]
    async fn should_force_the_json_tool_wire_format_with_provider_options_name_provider_options() {
        for provider_options in [
            json!({ "amazonBedrock": { "structuredOutputMode": "jsonTool" } }),
            json!({ "bedrock": { "structuredOutputMode": "jsonTool" } }),
            json!({ "anthropic": { "structuredOutputMode": "jsonTool" } }),
        ] {
            let g = generate(
                NEWER_ANTHROPIC_MODEL_ID,
                json_tool_name_response(),
                json!({
                    "prompt": test_prompt(),
                    "providerOptions": provider_options,
                    "responseFormat": name_response_format(),
                }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(tool_names(&body), vec!["json"]);
            assert_json_eq(&body["toolConfig"]["toolChoice"], &json!({ "any": {} }));
            assert_eq!(body.get("structuredOutputMode"), None);
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config/format"),
                None
            );
            assert_json_eq(
                &g.content(),
                &json!([{ "type": "text", "text": "{\"name\":\"Test\"}" }]),
            );
            assert_eq!(g.finish_reason()["unified"], "stop");
            assert_eq!(
                g.bedrock_metadata("/isJsonResponseFromTool"),
                Some(json!(true))
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should remove a manually supplied output_config.format in jsonTool mode while preserving sibling fields
    #[tokio::test]
    async fn should_remove_a_manually_supplied_output_config_format_in_json_tool_mode() {
        let g = generate(
            NEWER_ANTHROPIC_MODEL_ID,
            json_tool_name_response(),
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    "amazonBedrock": {
                        "structuredOutputMode": "jsonTool",
                        "additionalModelRequestFields": {
                            "output_config": {
                                "effort": "medium",
                                "format": { "type": "manually-supplied-format" },
                            },
                        },
                    },
                },
                "responseFormat": name_response_format(),
            }),
        )
        .await;

        assert_json_eq(
            at(
                &g.request_body(),
                "/additionalModelRequestFields/output_config",
            )
            .expect("output_config"),
            &json!({ "effort": "medium" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should force output_config.format for models that auto mode routes to the json tool
    #[tokio::test]
    async fn should_force_output_config_format_for_models_that_auto_mode_routes_to_json_tool() {
        let g = generate(
            OPUS5_ANTHROPIC_MODEL_ID,
            json_response(&json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "{\"name\":\"Test\"}" }] } },
                "usage": { "inputTokens": 4, "outputTokens": 10, "totalTokens": 14 },
                "stopReason": "end_turn",
            })),
            json!({
                "prompt": test_prompt(),
                "providerOptions": { "amazonBedrock": { "structuredOutputMode": "outputFormat" } },
                "responseFormat": name_response_format(),
            }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(body.get("toolConfig"), None);
        assert_json_eq(
            at(&body, "/additionalModelRequestFields/output_config/format").expect("format"),
            &json!({
                "type": "json_schema",
                "schema": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"],
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should prefer amazonBedrock structuredOutputMode over anthropic structuredOutputMode
    #[tokio::test]
    async fn should_prefer_amazon_bedrock_structured_output_mode_over_anthropic() {
        let g = generate(
            NEWER_ANTHROPIC_MODEL_ID,
            json_tool_name_response(),
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    "amazonBedrock": { "structuredOutputMode": "jsonTool" },
                    "anthropic": { "structuredOutputMode": "outputFormat" },
                },
                "responseFormat": name_response_format(),
            }),
        )
        .await;

        let body = g.request_body();
        assert_json_eq(&body["toolConfig"]["toolChoice"], &json!({ "any": {} }));
        assert_eq!(
            at(&body, "/additionalModelRequestFields/output_config/format"),
            None
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should use native output_config.format for models with structured output support even without thinking enabled
    #[tokio::test]
    async fn should_use_native_output_config_format_for_models_with_structured_output_support() {
        let g = generate(
            NATIVE_STRUCTURED_OUTPUT_ANTHROPIC_MODEL_ID,
            json_response(&json!({
                "output": { "message": { "content": [{ "text": "{\"name\":\"Test\"}" }], "role": "assistant" } },
                "usage": { "inputTokens": 4, "outputTokens": 10, "totalTokens": 14 },
                "stopReason": "end_turn",
            })),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate a name" }] }],
                "responseFormat": name_response_format(),
            }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(body.get("toolConfig"), None);
        assert_json_eq(
            at(&body, "/additionalModelRequestFields/output_config").expect("output_config"),
            &json!({
                "format": {
                    "schema": {
                        "additionalProperties": false,
                        "properties": { "name": { "type": "string" } },
                        "required": ["name"],
                        "type": "object",
                    },
                    "type": "json_schema",
                },
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should sanitize unsupported JSON schema keywords for native structured output
    #[tokio::test]
    async fn should_sanitize_unsupported_json_schema_keywords_for_native_structured_output() {
        let g = generate(
            NATIVE_STRUCTURED_OUTPUT_ANTHROPIC_MODEL_ID,
            json_response(&json!({
                "output": {
                    "message": { "content": [{ "text": "{\"labels\":[\"Spring\",\"Summer\",\"Autumn\"]}" }], "role": "assistant" },
                },
                "usage": { "inputTokens": 4, "outputTokens": 10, "totalTokens": 14 },
                "stopReason": "end_turn",
            })),
            json!({
                "prompt": test_prompt(),
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "labels": { "type": "array", "maxItems": 3, "items": { "type": "string" } },
                        },
                        "required": ["labels"],
                        "additionalProperties": false,
                    },
                },
            }),
        )
        .await;

        assert_json_eq(
            &g.request_body()["additionalModelRequestFields"]["output_config"]["format"]["schema"],
            &json!({
                "additionalProperties": false,
                "properties": {
                    "labels": { "description": "max items: 3.", "items": { "type": "string" }, "type": "array" },
                },
                "required": ["labels"],
                "type": "object",
            }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should use JSON instructions instead of a response tool when structured output is combined with tools on %s
    #[tokio::test]
    async fn should_use_json_instructions_when_structured_output_is_combined_with_tools_on_s() {
        for model_id in [OPUS_ANTHROPIC_MODEL_ID, SONNET5_ANTHROPIC_MODEL_ID] {
            let g = generate(
                model_id,
                json_response(&json!({
                    "output": {
                        "message": { "content": [{ "text": "```json\n{\"name\":\"Test\"}\n```." }], "role": "assistant" },
                    },
                    "usage": { "inputTokens": 4, "outputTokens": 10, "totalTokens": 14 },
                    "stopReason": "end_turn",
                })),
                json!({
                    "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Look up and generate a name" }] }],
                    "responseFormat": name_response_format(),
                    "tools": [
                        { "type": "function", "name": "lookupName", "inputSchema": { "type": "object", "properties": {} } },
                    ],
                }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(
                &body["toolConfig"],
                &json!({
                    "tools": [
                        {
                            "toolSpec": {
                                "inputSchema": { "json": { "properties": {}, "type": "object" } },
                                "name": "lookupName",
                            },
                        },
                    ],
                }),
            );
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config"),
                None
            );
            assert_json_eq(
                &body["system"],
                &json!([
                    {
                        "text": "JSON schema:\n{\"type\":\"object\",\"properties\":{\"name\":{\"type\":\"string\"}},\"required\":[\"name\"]}\nYou MUST answer with only a JSON object that matches the JSON schema above. Do not wrap it in markdown fences or include any other text.",
                    },
                ]),
            );
            assert_json_eq(
                &g.content(),
                &json!([{ "text": "{\"name\":\"Test\"}", "type": "text" }]),
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should extract reasoning text with signature
    #[tokio::test]
    async fn should_extract_reasoning_text_with_signature() {
        let g = generate(
            MODEL_ID,
            content_response(json!([
                {
                    "reasoningContent": {
                        "reasoningText": {
                            "text": "I need to think about this problem carefully...",
                            "signature": "abc123signature",
                        },
                    },
                },
                { "type": "text", "text": "The answer is 42." },
            ])),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                {
                    "providerMetadata": {
                        "amazonBedrock": { "signature": "abc123signature" },
                        "bedrock": { "signature": "abc123signature" },
                    },
                    "text": "I need to think about this problem carefully...",
                    "type": "reasoning",
                },
                { "text": "The answer is 42.", "type": "text" },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should preserve empty text blocks between reasoning blocks
    #[tokio::test]
    async fn should_preserve_empty_text_blocks_between_reasoning_blocks() {
        let g = generate(
            MODEL_ID,
            content_response(json!([
                { "reasoningContent": { "reasoningText": { "text": "thinking...", "signature": "sig-1" } } },
                { "text": "" },
                { "reasoningContent": { "reasoningText": { "text": "more thinking...", "signature": "sig-2" } } },
                { "text": "The answer is 42." },
            ])),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                {
                    "providerMetadata": {
                        "amazonBedrock": { "signature": "sig-1" },
                        "bedrock": { "signature": "sig-1" },
                    },
                    "text": "thinking...",
                    "type": "reasoning",
                },
                { "text": "", "type": "text" },
                {
                    "providerMetadata": {
                        "amazonBedrock": { "signature": "sig-2" },
                        "bedrock": { "signature": "sig-2" },
                    },
                    "text": "more thinking...",
                    "type": "reasoning",
                },
                { "text": "The answer is 42.", "type": "text" },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should extract reasoning text without signature
    #[tokio::test]
    async fn should_extract_reasoning_text_without_signature() {
        let g = generate(
            MODEL_ID,
            content_response(json!([
                { "reasoningContent": { "reasoningText": { "text": "I need to think about this problem carefully..." } } },
                { "type": "text", "text": "The answer is 42." },
            ])),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                { "text": "I need to think about this problem carefully...", "type": "reasoning" },
                { "text": "The answer is 42.", "type": "text" },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should extract redacted reasoning
    #[tokio::test]
    async fn should_extract_redacted_reasoning() {
        let g = generate(
            MODEL_ID,
            content_response(json!([
                { "reasoningContent": { "redactedReasoning": { "data": "redacted-reasoning-data" } } },
                { "type": "text", "text": "The answer is 42." },
            ])),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                {
                    "providerMetadata": {
                        "amazonBedrock": { "redactedData": "redacted-reasoning-data" },
                        "bedrock": { "redactedData": "redacted-reasoning-data" },
                    },
                    "text": "",
                    "type": "reasoning",
                },
                { "text": "The answer is 42.", "type": "text" },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should expose reasoning redacted as `redactedContent` for replay
    #[tokio::test]
    async fn should_expose_reasoning_redacted_as_redacted_content_for_replay() {
        let g = generate(
            MODEL_ID,
            content_response(json!([
                { "reasoningContent": { "redactedContent": "encrypted-reasoning-payload" } },
                { "type": "text", "text": "The answer is 42." },
            ])),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                {
                    "providerMetadata": {
                        "amazonBedrock": { "redactedContent": "encrypted-reasoning-payload" },
                        "bedrock": { "redactedContent": "encrypted-reasoning-payload" },
                    },
                    "text": "",
                    "type": "reasoning",
                },
                { "text": "The answer is 42.", "type": "text" },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should handle multiple reasoning blocks
    #[tokio::test]
    async fn should_handle_multiple_reasoning_blocks() {
        let g = generate(
            MODEL_ID,
            content_response(json!([
                { "reasoningContent": { "reasoningText": { "text": "First reasoning block", "signature": "sig1" } } },
                { "reasoningContent": { "redactedReasoning": { "data": "redacted-data" } } },
                { "type": "text", "text": "The answer is 42." },
            ])),
            json!({ "prompt": test_prompt() }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                {
                    "providerMetadata": {
                        "amazonBedrock": { "signature": "sig1" },
                        "bedrock": { "signature": "sig1" },
                    },
                    "text": "First reasoning block",
                    "type": "reasoning",
                },
                {
                    "providerMetadata": {
                        "amazonBedrock": { "redactedData": "redacted-data" },
                        "bedrock": { "redactedData": "redacted-data" },
                    },
                    "text": "",
                    "type": "reasoning",
                },
                { "text": "The answer is 42.", "type": "text" },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should omit toolConfig and filter tool content when conversation has tool calls but no active tools
    #[tokio::test]
    async fn should_omit_tool_config_and_filter_tool_content_without_active_tools() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({ "prompt": conversation_with_tool_calls(), "tools": [] }),
        )
        .await;

        let body = g.request_body();
        assert_eq!(body.get("toolConfig"), None);
        assert_json_eq(
            &body["messages"],
            &json!([
                {
                    "content": [
                        { "text": "What is the weather in Toronto?" },
                        { "text": "Now give me a summary." },
                    ],
                    "role": "user",
                },
            ]),
        );
        assert_json_eq(
            &g.warnings(),
            &json!([
                {
                    "details": "Tool calls and results removed from conversation because Bedrock does not support tool content without active tools.",
                    "feature": "toolContent",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should handle JSON response format with schema
    #[tokio::test]
    async fn should_handle_json_response_format_with_schema() {
        let g = generate(
            MODEL_ID,
            json_response(&json!({
                "output": {
                    "message": {
                        "role": "assistant",
                        "content": [
                            {
                                "toolUse": {
                                    "toolUseId": "json-tool-id",
                                    "name": "json",
                                    "input": { "recipe": { "name": "Lasagna", "ingredients": ["pasta", "cheese"] } },
                                },
                            },
                        ],
                    },
                },
                "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
                "stopReason": "tool_use",
            })),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate a recipe" }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "recipe": {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string" },
                                    "ingredients": { "type": "array", "items": { "type": "string" } },
                                },
                                "required": ["name", "ingredients"],
                            },
                        },
                        "required": ["recipe"],
                    },
                },
            }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                {
                    "text": "{\"recipe\":{\"name\":\"Lasagna\",\"ingredients\":[\"pasta\",\"cheese\"]}}",
                    "type": "text",
                },
            ]),
        );
        assert_eq!(
            g.bedrock_metadata("/isJsonResponseFromTool"),
            Some(json!(true))
        );
        assert_json_eq(
            &g.finish_reason(),
            &json!({ "raw": "tool_use", "unified": "stop" }),
        );
        let body = g.request_body();
        assert_eq!(tool_names(&body), vec!["json"]);
        assert_eq!(
            body["toolConfig"]["tools"][0]["toolSpec"]["description"],
            "Respond with a JSON object."
        );
        assert_json_eq(&body["toolConfig"]["toolChoice"], &json!({ "any": {} }));
    }

    mod json_schema_response_format_with_json_tool_response {
        use super::*;

        /// The describe's `beforeEach`.
        async fn generate_json_tool_response() -> Generated {
            generate(
                MODEL_ID,
                fixture_response("amazon-bedrock-json-tool.1"),
                json!({
                    "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Test" }] }],
                    "responseFormat": {
                        "type": "json",
                        "schema": {
                            "type": "object",
                            "properties": {
                                "elements": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "location": { "type": "string" },
                                            "temperature": { "type": "number" },
                                            "condition": { "type": "string" },
                                        },
                                        "required": ["location", "temperature", "condition"],
                                    },
                                },
                            },
                            "required": ["elements"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    },
                }),
            )
            .await
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › json schema response format with json tool response › should pass json schema response format as a tool
        #[tokio::test]
        async fn should_pass_json_schema_response_format_as_a_tool() {
            let g = generate_json_tool_response().await;

            let body = g.request_body();
            assert!(body.get("toolConfig").is_some());
            assert_eq!(tool_names(&body), vec!["json"]);
            assert_eq!(
                body["toolConfig"]["tools"][0]["toolSpec"]["description"],
                "Respond with a JSON object."
            );
            assert_json_eq(&body["toolConfig"]["toolChoice"], &json!({ "any": {} }));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › json schema response format with json tool response › should return the json response as text
        #[tokio::test]
        async fn should_return_the_json_response_as_text() {
            let g = generate_json_tool_response().await;

            assert_json_eq(
                &g.content(),
                &json!([
                    {
                        "text": "{\"elements\":[{\"location\":\"San Francisco\",\"temperature\":-5,\"condition\":\"snowy\"},{\"location\":\"London\",\"temperature\":0,\"condition\":\"snowy\"}]}",
                        "type": "text",
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › json schema response format with json tool response › should send stop finish reason when json tool is used
        #[tokio::test]
        async fn should_send_stop_finish_reason_when_json_tool_is_used() {
            let g = generate_json_tool_response().await;

            assert_json_eq(
                &g.finish_reason(),
                &json!({ "raw": "tool_use", "unified": "stop" }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › json schema response format with json tool response › should set isJsonResponseFromTool in provider metadata
        #[tokio::test]
        async fn should_set_is_json_response_from_tool_in_provider_metadata() {
            let g = generate_json_tool_response().await;

            assert_eq!(
                g.bedrock_metadata("/isJsonResponseFromTool"),
                Some(json!(true))
            );
        }
    }

    mod json_schema_response_format_with_other_tool_response {
        use super::*;

        /// The describe's `beforeEach`.
        async fn generate_other_tool_response() -> Generated {
            generate(
                MODEL_ID,
                fixture_response("amazon-bedrock-json-other-tool.1"),
                json!({
                    "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Test" }] }],
                    "tools": [
                        {
                            "type": "function",
                            "name": "get-weather",
                            "description": "Get the weather in a location",
                            "inputSchema": {
                                "type": "object",
                                "properties": { "location": { "type": "string" } },
                                "required": ["location"],
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        },
                    ],
                    "responseFormat": {
                        "type": "json",
                        "schema": {
                            "type": "object",
                            "properties": {
                                "weather": { "type": "string" },
                                "temperature": { "type": "number" },
                            },
                            "required": ["weather", "temperature"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    },
                }),
            )
            .await
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › json schema response format with other tool response › should pass the tool and the json schema response format as tools
        #[tokio::test]
        async fn should_pass_the_tool_and_the_json_schema_response_format_as_tools() {
            let g = generate_other_tool_response().await;

            let body = g.request_body();
            assert!(body.get("toolConfig").is_some());
            assert_eq!(tool_names(&body), vec!["get-weather", "json"]);
            assert_json_eq(&body["toolConfig"]["toolChoice"], &json!({ "any": {} }));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › json schema response format with other tool response › should return the regular tool call
        #[tokio::test]
        async fn should_return_the_regular_tool_call() {
            let g = generate_other_tool_response().await;

            assert_json_eq(
                &g.content(),
                &json!([
                    {
                        "input": "{\"location\":\"San Francisco\"}",
                        "toolCallId": "toolu_01PQjhxo3eirCdKNvCJrKc8f",
                        "toolName": "get-weather",
                        "type": "tool-call",
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › json schema response format with other tool response › should send tool-calls finish reason
        #[tokio::test]
        async fn should_send_tool_calls_finish_reason() {
            let g = generate_other_tool_response().await;

            assert_json_eq(
                &g.finish_reason(),
                &json!({ "raw": "tool_use", "unified": "tool-calls" }),
            );
        }
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should omit toolConfig when conversation has tool calls but toolChoice is none
    #[tokio::test]
    async fn should_omit_tool_config_when_conversation_has_tool_calls_but_tool_choice_is_none() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({
                "prompt": conversation_with_tool_calls(),
                "tools": [
                    {
                        "type": "function",
                        "name": "weather",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "city": { "type": "string" } },
                            "required": ["city"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    },
                ],
                "toolChoice": { "type": "none" },
            }),
        )
        .await;

        assert_eq!(g.request_body().get("toolConfig"), None);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should clamp temperature above 1 to 1 and add warning
    #[tokio::test]
    async fn should_clamp_temperature_above_1_to_1_and_add_warning() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({ "prompt": test_prompt(), "temperature": 1.5 }),
        )
        .await;

        assert_json_eq(
            &g.request_body()["inferenceConfig"]["temperature"],
            &json!(1),
        );
        assert_json_eq(
            &g.warnings(),
            &json!([
                {
                    "details": "1.5 exceeds bedrock maximum of 1.0. clamped to 1.0",
                    "feature": "temperature",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should clamp temperature below 0 to 0 and add warning
    #[tokio::test]
    async fn should_clamp_temperature_below_0_to_0_and_add_warning() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({ "prompt": test_prompt(), "temperature": -0.5 }),
        )
        .await;

        assert_json_eq(
            &g.request_body()["inferenceConfig"]["temperature"],
            &json!(0),
        );
        assert_json_eq(
            &g.warnings(),
            &json!([
                {
                    "details": "-0.5 is below bedrock minimum of 0. clamped to 0",
                    "feature": "temperature",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should not clamp valid temperature between 0 and 1
    #[tokio::test]
    async fn should_not_clamp_valid_temperature_between_0_and_1() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-text"),
            json!({ "prompt": test_prompt(), "temperature": 0.7 }),
        )
        .await;

        assert_json_eq(
            &g.request_body()["inferenceConfig"]["temperature"],
            &json!(0.7),
        );
        assert_json_eq(&g.warnings(), &json!([]));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include text content before JSON tool call in doGenerate
    #[tokio::test]
    async fn should_include_text_content_before_json_tool_call_in_do_generate() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-json-tool.2"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate JSON" }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                    },
                },
            }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                { "text": "Let me generate that JSON for you.", "type": "text" },
                { "text": "{\"value\":\"test data\"}", "type": "text" },
            ]),
        );
        assert_eq!(
            g.bedrock_metadata("/isJsonResponseFromTool"),
            Some(json!(true))
        );
        assert_json_eq(
            &g.finish_reason(),
            &json!({ "raw": "tool_use", "unified": "stop" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should preserve text response before JSON output (answering question then returning structured data)
    #[tokio::test]
    async fn should_preserve_text_response_before_json_output() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-json-tool-with-answer.1"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [{ "type": "text", "text": "What's 2+2? Also the name to use is John Doe" }],
                    },
                ],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "firstName": { "type": "string" }, "lastName": { "type": "string" } },
                        "required": ["firstName", "lastName"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                    },
                },
            }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                { "text": "2 + 2 equals 4.", "type": "text" },
                { "text": "{\"firstName\":\"John\",\"lastName\":\"Doe\"}", "type": "text" },
            ]),
        );
        assert_json_eq(
            &g.finish_reason(),
            &json!({ "raw": "tool_use", "unified": "stop" }),
        );
        assert_eq!(
            g.bedrock_metadata("/isJsonResponseFromTool"),
            Some(json!(true))
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should include multiple text blocks before JSON tool call in doGenerate
    #[tokio::test]
    async fn should_include_multiple_text_blocks_before_json_tool_call_in_do_generate() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-json-tool.3"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Generate data" }] }],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "result": { "type": "number" } },
                        "required": ["result"],
                    },
                },
            }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                { "text": "First text block.", "type": "text" },
                { "text": "Second text block.", "type": "text" },
                { "text": "Third text block before JSON.", "type": "text" },
                { "text": "{\"result\":42}", "type": "text" },
            ]),
        );
        assert_eq!(
            g.bedrock_metadata("/isJsonResponseFromTool"),
            Some(json!(true))
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should handle regular tool call before JSON tool call in doGenerate
    #[tokio::test]
    async fn should_handle_regular_tool_call_before_json_tool_call_in_do_generate() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-json-with-tool.1"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Test" }] }],
                "tools": [
                    {
                        "type": "function",
                        "name": "get-weather",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "location": { "type": "string" } },
                            "required": ["location"],
                        },
                    },
                ],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "result": { "type": "string" } },
                        "required": ["result"],
                    },
                },
            }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                { "text": "Let me call a tool first.", "type": "text" },
                {
                    "input": "{\"location\":\"SF\"}",
                    "toolCallId": "tool-1",
                    "toolName": "get-weather",
                    "type": "tool-call",
                },
                { "text": "Now the JSON.", "type": "text" },
                { "text": "{\"result\":\"data\"}", "type": "text" },
            ]),
        );
        assert_eq!(
            g.bedrock_metadata("/isJsonResponseFromTool"),
            Some(json!(true))
        );
        assert_json_eq(
            &g.finish_reason(),
            &json!({ "raw": "tool_use", "unified": "stop" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should handle multiple regular tool calls before JSON tool call in doGenerate
    #[tokio::test]
    async fn should_handle_multiple_regular_tool_calls_before_json_tool_call_in_do_generate() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-json-with-tools.1"),
            json!({
                "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Test" }] }],
                "tools": [
                    {
                        "type": "function",
                        "name": "tool-a",
                        "inputSchema": { "type": "object", "properties": { "param": { "type": "string" } } },
                    },
                    {
                        "type": "function",
                        "name": "tool-b",
                        "inputSchema": { "type": "object", "properties": { "param": { "type": "string" } } },
                    },
                ],
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "final": { "type": "string" } },
                        "required": ["final"],
                    },
                },
            }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                { "text": "Calling multiple tools.", "type": "text" },
                { "input": "{\"param\":\"a\"}", "toolCallId": "tool-1", "toolName": "tool-a", "type": "tool-call" },
                { "input": "{\"param\":\"b\"}", "toolCallId": "tool-2", "toolName": "tool-b", "type": "tool-call" },
                { "text": "Finally the JSON response.", "type": "text" },
                { "text": "{\"final\":\"result\"}", "type": "text" },
            ]),
        );
        assert_eq!(
            g.bedrock_metadata("/isJsonResponseFromTool"),
            Some(json!(true))
        );
        assert_json_eq(
            &g.finish_reason(),
            &json!({ "raw": "tool_use", "unified": "stop" }),
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › should support tool calls with empty input (no arguments)
    #[tokio::test]
    async fn should_support_tool_calls_with_empty_input_no_arguments() {
        let g = generate(
            MODEL_ID,
            fixture_response("amazon-bedrock-tool-no-args"),
            json!({
                "tools": [
                    {
                        "type": "function",
                        "name": "updateIssueList",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "required": [],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    },
                ],
                "prompt": test_prompt(),
            }),
        )
        .await;

        assert_json_eq(
            &g.content(),
            &json!([
                { "text": "I'll update the issue list for you.", "type": "text" },
                { "input": "{}", "toolCallId": "tool-use-id", "toolName": "updateIssueList", "type": "tool-call" },
            ]),
        );
    }

    mod legacy_anthropic_model_capabilities {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › legacy Anthropic model capabilities › should use budget-based reasoning for a platform-prefixed Claude 3.5 model
        #[tokio::test]
        async fn should_use_budget_based_reasoning_for_a_platform_prefixed_claude_3_5_model() {
            let g = generate(
                ANTHROPIC_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "high" }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/thinking").expect("thinking"),
                &json!({ "type": "enabled", "budget_tokens": (4096.0_f64 * 0.6).round() }),
            );
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config"),
                None
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › legacy Anthropic model capabilities › should use the JSON tool fallback for a platform-prefixed Claude 3.7 model
        #[tokio::test]
        async fn should_use_the_json_tool_fallback_for_a_platform_prefixed_claude_3_7_model() {
            let g = generate(
                LEGACY_ANTHROPIC_37_MODEL_ID,
                simple_response(),
                json!({
                    "prompt": test_prompt(),
                    "responseFormat": {
                        "type": "json",
                        "schema": {
                            "type": "object",
                            "properties": { "name": { "type": "string" } },
                            "required": ["name"],
                            "additionalProperties": false,
                        },
                    },
                }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(tool_names(&body), vec!["json"]);
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config"),
                None
            );
        }
    }

    mod top_level_reasoning_parameter {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should not set reasoning config when reasoning is "provider-default" for newer Anthropic models
        #[tokio::test]
        async fn should_not_set_reasoning_config_when_reasoning_is_provider_default() {
            let g = generate(
                NEWER_ANTHROPIC_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "provider-default" }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(at(&body, "/additionalModelRequestFields/thinking"), None);
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config"),
                None
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should map reasoning to adaptive thinking with effort for newer Anthropic models
        #[tokio::test]
        async fn should_map_reasoning_to_adaptive_thinking_with_effort_for_newer_anthropic_models()
        {
            let g = generate(
                NEWER_ANTHROPIC_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "high" }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/thinking").expect("thinking"),
                &json!({ "type": "adaptive" }),
            );
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config/effort"),
                Some(&json!("high"))
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should map reasoning "xhigh" to effort "max" for newer Anthropic models
        #[tokio::test]
        async fn should_map_reasoning_xhigh_to_effort_max_for_newer_anthropic_models() {
            let g = generate(
                NEWER_ANTHROPIC_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "xhigh" }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/thinking").expect("thinking"),
                &json!({ "type": "adaptive" }),
            );
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config/effort"),
                Some(&json!("max"))
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should warn when reasoning "minimal" is mapped for newer Anthropic models
        #[tokio::test]
        async fn should_warn_when_reasoning_minimal_is_mapped_for_newer_anthropic_models() {
            let g = generate(
                NEWER_ANTHROPIC_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "minimal" }),
            )
            .await;

            assert_contains_equal(
                &g.warnings(),
                &json!({
                    "type": "compatibility",
                    "feature": "reasoning",
                    "details": "reasoning \"minimal\" is not directly supported by this model. mapped to effort \"low\".",
                }),
            );
            assert_eq!(
                at(
                    &g.request_body(),
                    "/additionalModelRequestFields/output_config/effort"
                ),
                Some(&json!("low"))
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should map reasoning to budget-based thinking for older Anthropic models
        #[tokio::test]
        async fn should_map_reasoning_to_budget_based_thinking_for_older_anthropic_models() {
            let g = generate(
                MODEL_ID,
                fixture_response("amazon-bedrock-text"),
                json!({ "prompt": test_prompt(), "reasoning": "high" }),
            )
            .await;

            assert_json_eq(
                at(&g.request_body(), "/additionalModelRequestFields/thinking").expect("thinking"),
                &json!({ "type": "enabled", "budget_tokens": (4096.0_f64 * 0.6).round().max(1024.0) }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should map reasoning to budget with minimum of 1024 tokens for older Anthropic models
        #[tokio::test]
        async fn should_map_reasoning_to_budget_with_minimum_of_1024_tokens_for_older_anthropic() {
            let g = generate(
                MODEL_ID,
                fixture_response("amazon-bedrock-text"),
                json!({ "prompt": test_prompt(), "reasoning": "minimal" }),
            )
            .await;

            assert_json_eq(
                at(
                    &g.request_body(),
                    "/additionalModelRequestFields/thinking/budget_tokens",
                )
                .expect("budget_tokens"),
                &json!(1024),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should map reasoning directly to reasoning_effort for OpenAI gpt-oss models
        #[tokio::test]
        async fn should_map_reasoning_directly_to_reasoning_effort_for_openai_gpt_oss_models() {
            let g = generate(
                OPENAI_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "medium" }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(
                at(&body, "/additionalModelRequestFields/reasoning_effort"),
                Some(&json!("medium"))
            );
            assert_eq!(at(&body, "/additionalModelRequestFields/thinking"), None);
            assert_eq!(
                at(&body, "/additionalModelRequestFields/reasoningConfig"),
                None
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should map portable reasoning for Nova 2
        #[tokio::test]
        async fn should_map_portable_reasoning_for_nova_2() {
            let g = generate(
                NOVA_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "high" }),
            )
            .await;

            assert_json_eq(
                at(
                    &g.request_body(),
                    "/additionalModelRequestFields/reasoningConfig",
                )
                .expect("reasoningConfig"),
                &json!({ "type": "enabled", "maxReasoningEffort": "high" }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should omit maxOutputTokens for Nova 2 high reasoning
        #[tokio::test]
        async fn should_omit_max_output_tokens_for_nova_2_high_reasoning() {
            let g = generate(
                NOVA_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "high", "maxOutputTokens": 1024 }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(at(&body, "/inferenceConfig/maxTokens"), None);
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/reasoningConfig")
                    .expect("reasoningConfig"),
                &json!({ "type": "enabled", "maxReasoningEffort": "high" }),
            );
            assert_contains_equal(
                &g.warnings(),
                &json!({
                    "type": "unsupported",
                    "feature": "maxOutputTokens",
                    "details": format!("maxOutputTokens is not supported by {NOVA_MODEL_ID} when high reasoning is enabled and will be ignored"),
                }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should preserve maxOutputTokens for Nova 2 medium reasoning
        #[tokio::test]
        async fn should_preserve_max_output_tokens_for_nova_2_medium_reasoning() {
            let g = generate(
                NOVA_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "medium", "maxOutputTokens": 1024 }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(
                at(&body, "/inferenceConfig/maxTokens").expect("maxTokens"),
                &json!(1024),
            );
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/reasoningConfig")
                    .expect("reasoningConfig"),
                &json!({ "type": "enabled", "maxReasoningEffort": "medium" }),
            );
            assert!(
                !has_warning_feature(&g, "maxOutputTokens"),
                "unexpected maxOutputTokens warning: {:#}",
                g.warnings()
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should omit maxOutputTokens for Nova 2 xhigh portable reasoning (max effort)
        #[tokio::test]
        async fn should_omit_max_output_tokens_for_nova_2_xhigh_portable_reasoning_max_effort() {
            let g = generate(
                NOVA_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "xhigh", "maxOutputTokens": 1024 }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(at(&body, "/inferenceConfig/maxTokens"), None);
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/reasoningConfig")
                    .expect("reasoningConfig"),
                &json!({ "type": "enabled", "maxReasoningEffort": "max" }),
            );
            assert!(
                has_warning_feature(&g, "maxOutputTokens"),
                "expected a maxOutputTokens warning: {:#}",
                g.warnings()
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should omit maxOutputTokens for Nova 2 explicit max reasoning effort
        #[tokio::test]
        async fn should_omit_max_output_tokens_for_nova_2_explicit_max_reasoning_effort() {
            let g = generate(
                NOVA_MODEL_ID,
                simple_response(),
                json!({
                    "prompt": test_prompt(),
                    "maxOutputTokens": 1024,
                    "providerOptions": {
                        "amazonBedrock": { "reasoningConfig": { "type": "enabled", "maxReasoningEffort": "max" } },
                    },
                }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(at(&body, "/inferenceConfig/maxTokens"), None);
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/reasoningConfig")
                    .expect("reasoningConfig"),
                &json!({ "type": "enabled", "maxReasoningEffort": "max" }),
            );
            assert!(
                has_warning_feature(&g, "maxOutputTokens"),
                "expected a maxOutputTokens warning: {:#}",
                g.warnings()
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should map reasoning to reasoningConfig when explicitly enabled for Nova 2
        #[tokio::test]
        async fn should_map_reasoning_to_reasoning_config_when_explicitly_enabled_for_nova_2() {
            let g = generate(
                NOVA_MODEL_ID,
                simple_response(),
                json!({
                    "prompt": test_prompt(),
                    "reasoning": "high",
                    "providerOptions": { "amazonBedrock": { "reasoningConfig": { "type": "enabled" } } },
                }),
            )
            .await;

            assert_json_eq(
                at(
                    &g.request_body(),
                    "/additionalModelRequestFields/reasoningConfig",
                )
                .expect("reasoningConfig"),
                &json!({ "type": "enabled", "maxReasoningEffort": "high" }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should ignore portable reasoning for models without known reasoning support
        #[tokio::test]
        async fn should_ignore_portable_reasoning_for_models_without_known_reasoning_support() {
            let g = generate(
                NOVA_MICRO_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "high" }),
            )
            .await;

            assert_eq!(
                at(
                    &g.request_body(),
                    "/additionalModelRequestFields/reasoningConfig"
                ),
                None
            );
            assert_contains_equal(
                &g.warnings(),
                &json!({
                    "type": "unsupported",
                    "feature": "reasoning",
                    "details": "Portable reasoning is not supported for this model and will be ignored. If the model supports a provider-specific reasoning configuration, use providerOptions.amazonBedrock.reasoningConfig.",
                }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should forward explicit reasoningConfig for models without known reasoning support
        #[tokio::test]
        async fn should_forward_explicit_reasoning_config_for_models_without_reasoning_support() {
            let g = generate(
                NOVA_MICRO_MODEL_ID,
                simple_response(),
                json!({
                    "prompt": test_prompt(),
                    "providerOptions": {
                        "amazonBedrock": { "reasoningConfig": { "type": "enabled", "maxReasoningEffort": "high" } },
                    },
                }),
            )
            .await;

            assert_json_eq(
                at(
                    &g.request_body(),
                    "/additionalModelRequestFields/reasoningConfig",
                )
                .expect("reasoningConfig"),
                &json!({ "type": "enabled", "maxReasoningEffort": "high" }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should let explicit reasoningConfig fields win over derived reasoning values
        #[tokio::test]
        async fn should_let_explicit_reasoning_config_fields_win_over_derived_reasoning_values() {
            let g = generate(
                MODEL_ID,
                fixture_response("amazon-bedrock-text"),
                json!({
                    "prompt": test_prompt(),
                    "reasoning": "high",
                    "providerOptions": {
                        "bedrock": { "reasoningConfig": { "type": "enabled", "budgetTokens": 5000 } },
                    },
                }),
            )
            .await;

            assert_json_eq(
                at(&g.request_body(), "/additionalModelRequestFields/thinking").expect("thinking"),
                &json!({ "type": "enabled", "budget_tokens": 5000 }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should merge top-level reasoning with partial reasoningConfig for newer Anthropic models
        #[tokio::test]
        async fn should_merge_top_level_reasoning_with_partial_reasoning_config_for_newer_models() {
            let g = generate(
                NEWER_ANTHROPIC_MODEL_ID,
                simple_response(),
                json!({
                    "prompt": test_prompt(),
                    "reasoning": "high",
                    "providerOptions": { "bedrock": { "reasoningConfig": { "display": "summarized" } } },
                }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/thinking").expect("thinking"),
                &json!({ "type": "adaptive", "display": "summarized" }),
            );
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config/effort"),
                Some(&json!("high"))
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should honor reasoning "none" even when partial reasoningConfig is provided
        #[tokio::test]
        async fn should_honor_reasoning_none_even_when_partial_reasoning_config_is_provided() {
            let g = generate(
                NEWER_ANTHROPIC_MODEL_ID,
                simple_response(),
                json!({
                    "prompt": test_prompt(),
                    "reasoning": "none",
                    "providerOptions": { "bedrock": { "reasoningConfig": { "display": "summarized" } } },
                }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(at(&body, "/additionalModelRequestFields/thinking"), None);
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config"),
                None
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should let user-specified type win while still deriving maxReasoningEffort from reasoning
        #[tokio::test]
        async fn should_let_user_specified_type_win_while_still_deriving_max_reasoning_effort() {
            let g = generate(
                NEWER_ANTHROPIC_MODEL_ID,
                simple_response(),
                json!({
                    "prompt": test_prompt(),
                    "reasoning": "high",
                    "providerOptions": {
                        "bedrock": { "reasoningConfig": { "type": "enabled", "budgetTokens": 3000 } },
                    },
                }),
            )
            .await;

            let body = g.request_body();
            assert_json_eq(
                at(&body, "/additionalModelRequestFields/thinking").expect("thinking"),
                &json!({ "type": "enabled", "budget_tokens": 3000 }),
            );
            assert_eq!(
                at(&body, "/additionalModelRequestFields/output_config/effort"),
                Some(&json!("high"))
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should let user-specified maxReasoningEffort win over derived for non-Anthropic models
        #[tokio::test]
        async fn should_let_user_specified_max_reasoning_effort_win_for_non_anthropic_models() {
            let g = generate(
                NOVA_MODEL_ID,
                simple_response(),
                json!({
                    "prompt": test_prompt(),
                    "reasoning": "high",
                    "providerOptions": { "bedrock": { "reasoningConfig": { "maxReasoningEffort": "low" } } },
                }),
            )
            .await;

            assert_eq!(
                at(
                    &g.request_body(),
                    "/additionalModelRequestFields/reasoningConfig/maxReasoningEffort"
                ),
                Some(&json!("low"))
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should strip temperature, topP, topK for Anthropic models when reasoning enables thinking
        #[tokio::test]
        async fn should_strip_temperature_top_p_top_k_for_anthropic_models_when_thinking() {
            let g = generate(
                MODEL_ID,
                fixture_response("amazon-bedrock-text"),
                json!({
                    "prompt": test_prompt(),
                    "reasoning": "high",
                    "temperature": 0.7,
                    "topP": 0.9,
                    "topK": 5,
                }),
            )
            .await;

            let body = g.request_body();
            assert_eq!(at(&body, "/inferenceConfig/temperature"), None);
            assert_eq!(at(&body, "/inferenceConfig/topP"), None);
            assert_eq!(at(&body, "/inferenceConfig/topK"), None);
            let warnings = g.warnings();
            assert_contains_equal(
                &warnings,
                &json!({
                    "type": "unsupported",
                    "feature": "temperature",
                    "details": "temperature is not supported when thinking is enabled",
                }),
            );
            assert_contains_equal(
                &warnings,
                &json!({
                    "type": "unsupported",
                    "feature": "topP",
                    "details": "topP is not supported when thinking is enabled",
                }),
            );
            assert_contains_equal(
                &warnings,
                &json!({
                    "type": "unsupported",
                    "feature": "topK",
                    "details": "topK is not supported when thinking is enabled",
                }),
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should handle reasoning "none" for Anthropic models
        #[tokio::test]
        async fn should_handle_reasoning_none_for_anthropic_models() {
            let g = generate(
                MODEL_ID,
                fixture_response("amazon-bedrock-text"),
                json!({ "prompt": test_prompt(), "reasoning": "none" }),
            )
            .await;

            assert_eq!(
                at(&g.request_body(), "/additionalModelRequestFields/thinking"),
                None
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model.test.ts › doGenerate › top-level reasoning parameter › should handle reasoning "none" for OpenAI models
        #[tokio::test]
        async fn should_handle_reasoning_none_for_openai_models() {
            let g = generate(
                OPENAI_MODEL_ID,
                simple_response(),
                json!({ "prompt": test_prompt(), "reasoning": "none" }),
            )
            .await;

            assert_eq!(
                at(
                    &g.request_body(),
                    "/additionalModelRequestFields/reasoning_effort"
                ),
                None
            );
        }
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
    use aimux_core::tool::RawToolCall;
    use serde_json::json;

    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
    use aimux_core::options::CallOptions;
    use aimux_core::shared::provider_namespace;
    use aimux_provider_utils::Resolvable;

    use crate::bedrock::chat_language_model::build_request_body_checked;
    use crate::test_support::mock_fetch::{Canned, MockFetch};
    use aimux_core::stream_part::StreamPart;
    use aimux_core::types::FinishReasonUnified;

    use crate::bedrock::{AmazonBedrockProviderSettings, create_amazon_bedrock};

    // ════════════════════════════════════════════════════════════════════════════
    // amazon-bedrock-chat-language-model.test.ts — request URL / body / response
    // ════════════════════════════════════════════════════════════════════════════

    // ── supportedUrls ────────────────────────────────────────────────────────────

    /// partial: `should support S3 URLs for image and video parts`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn model_supported_urls_s3() {
        let model = create_amazon_bedrock(AmazonBedrockProviderSettings::default())
            .unwrap()
            .chat("amazon.nova-pro-v1:0");
        let urls = model.supported_urls().await;
        assert_eq!(urls.0.len(), 2);
        for kind in ["image/*", "video/*"] {
            assert_eq!(urls.0[kind].len(), 1);
            assert_eq!(urls.0[kind][0].as_str(), "^s3://");
        }
    }

    // ── ARN model IDs containing a slash ─────────────────────────────────────────

    /// partial: `should generate text through the encoded Converse route`; full port in src/bedrock/tests/chat_language_model.rs.
    ///
    /// ARN inference-profile IDs contain a `/` which must be percent-encoded in the
    /// URL path. The upstream uses `encodeURIComponent(modelId)`.
    #[tokio::test]
    async fn arn_model_id_encoded_generate_route() {
        let arn =
            "arn:aws:bedrock:eu-west-1:474668406012:inference-profile/eu.amazon.nova-lite-v1:0";
        let encoded = "arn%3Aaws%3Abedrock%3Aeu-west-1%3A474668406012%3Ainference-profile%2Feu.amazon.nova-lite-v1%3A0";

        let server = mock_converse_json(
            200,
            json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "Hello!" }] } },
                "stopReason": "end_turn",
                "usage": { "inputTokens": 1, "outputTokens": 1 }
            }),
        );

        let model = create_amazon_bedrock(AmazonBedrockProviderSettings {
            fetch: Some(server.transport()),
            base_url: Some("https://bedrock.test".into()),
            api_key: Some(Resolvable::Value("tok".into())),
            ..Default::default()
        })
        .unwrap()
        .chat(arn);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed via the encoded route");
        assert_eq!(as_text(&result.content[0]), "Hello!");
        assert_eq!(
            server.seen()[0].url,
            format!("https://bedrock.test/model/{encoded}/converse")
        );
    }

    /// partial: `should stream text through the encoded Converse route`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn arn_model_id_encoded_stream_route() {
        let arn =
            "arn:aws:bedrock:eu-west-1:474668406012:inference-profile/eu.amazon.nova-lite-v1:0";
        let encoded = "arn%3Aaws%3Abedrock%3Aeu-west-1%3A474668406012%3Ainference-profile%2Feu.amazon.nova-lite-v1%3A0";

        let events: Vec<(&str, &str, &str)> = vec![
            ("event", "messageStart", r#"{"role":"assistant"}"#),
            (
                "event",
                "contentBlockDelta",
                r#"{"contentBlockIndex":0,"delta":{"text":"Hello!"}}"#,
            ),
            ("event", "contentBlockStop", r#"{"contentBlockIndex":0}"#),
            ("event", "messageStop", r#"{"stopReason":"end_turn"}"#),
            (
                "event",
                "metadata",
                r#"{"usage":{"inputTokens":1,"outputTokens":1}}"#,
            ),
        ];
        let body_bytes = crate::bedrock::event_stream_decoder::encode_messages(&events);

        let server = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![(
                "content-type".into(),
                "application/vnd.amazon.eventstream".into(),
            )],
            body: body_bytes,
        }]);

        let model = create_amazon_bedrock(AmazonBedrockProviderSettings {
            fetch: Some(server.transport()),
            base_url: Some("https://bedrock.test".into()),
            api_key: Some(Resolvable::Value("tok".into())),
            ..Default::default()
        })
        .unwrap()
        .chat(arn);

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("should succeed via the encoded stream route");
        let parts = collect_stream(result).await;

        let text: String = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello!");
        assert_eq!(
            server.seen()[0].url,
            format!("https://bedrock.test/model/{encoded}/converse-stream")
        );
    }

    // ── temperature clamping ─────────────────────────────────────────────────────

    /// partial: `should clamp temperature above 1 to 1 and add warning`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn temperature_clamped_above_1() {
        let server = mock_converse_json(200, ok_converse_body());

        let model = make_model(&server);
        let mut opts = default_options(test_prompt());
        opts.temperature = Some(1.5);

        let result = model.do_generate(&opts).await.expect("should succeed");

        let body = result.request.unwrap().body.expect("request body");
        assert_eq!(
            body["inferenceConfig"]["temperature"].as_f64().unwrap(),
            1.0
        );
        assert!(
            result.warnings.iter().any(
                |w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. }
                    if feature == "temperature")
            ),
            "expected an unsupported-temperature warning, got {:?}",
            result.warnings
        );
    }

    /// partial: `should clamp temperature below 0 to 0 and add warning`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn temperature_clamped_below_0() {
        let server = mock_converse_json(200, ok_converse_body());

        let model = make_model(&server);
        let mut opts = default_options(test_prompt());
        opts.temperature = Some(-0.5);

        let result = model.do_generate(&opts).await.expect("should succeed");

        let body = result.request.unwrap().body.expect("request body");
        assert_eq!(
            body["inferenceConfig"]["temperature"].as_f64().unwrap(),
            0.0
        );
        assert!(
            result.warnings.iter().any(
                |w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. }
                    if feature == "temperature")
            ),
            "expected an unsupported-temperature warning"
        );
    }

    /// partial: `should not clamp valid temperature between 0 and 1`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn temperature_not_clamped_in_range() {
        let server = mock_converse_json(200, ok_converse_body());

        let model = make_model(&server);
        let mut opts = default_options(test_prompt());
        opts.temperature = Some(0.7);

        let result = model.do_generate(&opts).await.expect("should succeed");

        let body = result.request.unwrap().body.expect("request body");
        let temp = body["inferenceConfig"]["temperature"].as_f64().unwrap();
        assert!(
            (temp - 0.7).abs() < 1e-6,
            "temperature should be 0.7, got {temp}"
        );
        assert!(result.warnings.is_empty(), "no warnings expected");
    }

    /// TS: "should omit unsupported sampling parameters for …" — a model that
    /// rejects sampling parameters warns temperature, topK, topP in that order
    /// (amazon-bedrock-chat-language-model.ts:204-230) and sends none of them.
    #[test]
    fn rejected_sampling_parameters_warn_in_upstream_order() {
        let model_id = "global.anthropic.claude-opus-4-7";
        let mut opts = default_options(test_prompt());
        opts.temperature = Some(0.5);
        opts.top_p = Some(0.7);
        opts.top_k = Some(10.0);

        let (body, warnings, _, _) = build_request_body_checked(model_id, &opts, None).unwrap();

        assert_eq!(body["inferenceConfig"].get("temperature"), None);
        assert_eq!(body["inferenceConfig"].get("topP"), None);
        assert_eq!(body["inferenceConfig"].get("topK"), None);
        let expected: Vec<_> = ["temperature", "topK", "topP"]
            .into_iter()
            .map(|feature| aimux_core::types::Warning::Unsupported {
                feature: feature.into(),
                details: Some(format!(
                    "{feature} is not supported by {model_id} and will be ignored"
                )),
            })
            .collect();
        assert_eq!(warnings, expected);
    }

    /// TS: "should omit maxOutputTokens for Nova 2 high reasoning" — the warning
    /// names the model (amazon-bedrock-chat-language-model.ts:511).
    #[test]
    fn nova_high_reasoning_max_output_tokens_warning_names_the_model() {
        let model_id = "us.amazon.nova-2-lite-v1:0";
        let mut opts = default_options(test_prompt());
        opts.reasoning = Some(aimux_core::types::ReasoningEffort::High);
        opts.max_output_tokens = Some(1024);

        let (body, warnings, _, _) = build_request_body_checked(model_id, &opts, None).unwrap();

        assert_eq!(body["inferenceConfig"].get("maxTokens"), None);
        assert!(
            warnings.contains(&aimux_core::types::Warning::Unsupported {
                feature: "maxOutputTokens".into(),
                details: Some(format!(
                    "maxOutputTokens is not supported by {model_id} when high reasoning is enabled and will be ignored"
                )),
            }),
            "{warnings:?}"
        );
    }

    // ── guardrails ───────────────────────────────────────────────────────────────

    /// partial: `should support guardrails`; full port in src/bedrock/tests/chat_language_model.rs.
    /// `providerOptions.bedrock.guardrailConfig` is forwarded as a top-level `guardrailConfig` in the request body.
    #[tokio::test]
    async fn guardrail_config_in_request_body() {
        let server = mock_converse_json(200, ok_converse_body());

        let model = make_model(&server);
        let mut opts = default_options(test_prompt());
        let po = provider_namespace(
            "bedrock",
            json!({
                "guardrailConfig": {
                    "guardrailIdentifier": "-1",
                    "guardrailVersion": "1",
                    "trace": "enabled"
                }
            }),
        )
        .unwrap();
        opts.provider_options = Some(po);

        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request.unwrap().body.expect("request body");
        assert_eq!(
            body["guardrailConfig"],
            json!({
                "guardrailIdentifier": "-1",
                "guardrailVersion": "1",
                "trace": "enabled"
            })
        );
    }

    // ── trace in providerMetadata ────────────────────────────────────────────────

    /// partial: `should include trace information in providerMetadata`; full port in src/bedrock/tests/chat_language_model.rs.
    /// The response `trace` field is surfaced as `providerMetadata.bedrock.trace`.
    #[tokio::test]
    async fn trace_in_provider_metadata() {
        let trace = json!({
            "guardrail": {
                "inputAssessment": {
                    "1abcd2ef34gh": {
                        "contentPolicy": {
                            "filters": [{
                                "action": "BLOCKED",
                                "confidence": "LOW",
                                "type": "INSULTS"
                            }]
                        }
                    }
                }
            }
        });
        let server = mock_converse_json(
            200,
            json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "Hello, World!" }] } },
                "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
                "stopReason": "stop_sequence",
                "trace": trace
            }),
        );

        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider_metadata should be Some");
        assert_eq!(pm["bedrock"]["trace"], trace);
    }

    // ── stop_sequence in providerMetadata ────────────────────────────────────────

    /// partial: `should include stop_sequence in provider metadata`; full port in src/bedrock/tests/chat_language_model.rs.
    /// When the response carries `additionalModelResponseFields.delta.stop_sequence`, it is surfaced as `providerMetadata.bedrock.stopSequence` (and `amazonBedrock.stopSequence`).
    #[tokio::test]
    async fn stop_sequence_in_provider_metadata() {
        let server = mock_converse_json(
            200,
            json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "Hello, World!" }] } },
                "stopReason": "stop_sequence",
                "additionalModelResponseFields": { "delta": { "stop_sequence": "STOP" } },
                "usage": { "inputTokens": 4, "outputTokens": 30, "totalTokens": 34 }
            }),
        );

        let model = make_model(&server);
        let mut opts = default_options(test_prompt());
        opts.stop_sequences = Some(vec!["STOP".to_string()]);

        let result = model.do_generate(&opts).await.expect("should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider_metadata should be Some");
        assert_eq!(pm["bedrock"]["stopSequence"], json!("STOP"));
        assert_eq!(pm["amazonBedrock"]["stopSequence"], json!("STOP"));
    }

    // ── tool calls with empty input ──────────────────────────────────────────────

    /// partial: `should support tool calls with empty input (no arguments)`; full port in src/bedrock/tests/chat_language_model.rs.
    /// (stream).
    ///
    /// When a `toolUse` block carries no input deltas, the stream event loop
    /// accumulates an empty string and yields `input: "{}"`.
    #[tokio::test]
    async fn stream_tool_call_empty_input() {
        let events: Vec<(&str, &str, &str)> = vec![
            ("event", "messageStart", r#"{"role":"assistant"}"#),
            (
                "event",
                "contentBlockStart",
                r#"{"contentBlockIndex":0,"start":{"toolUse":{"name":"updateIssueList","toolUseId":"tool_1"}}}"#,
            ),
            // No input deltas — the tool call has no arguments.
            ("event", "contentBlockStop", r#"{"contentBlockIndex":0}"#),
            ("event", "messageStop", r#"{"stopReason":"tool_use"}"#),
        ];
        let body_bytes = crate::bedrock::event_stream_decoder::encode_messages(&events);

        let server = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![(
                "content-type".into(),
                "application/vnd.amazon.eventstream".into(),
            )],
            body: body_bytes,
        }]);

        let model = make_model(&server);
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let tool_calls: Vec<_> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolCall(RawToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    ..
                }) => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].0, "tool_1");
        assert_eq!(tool_calls[0].1, "updateIssueList");
        assert_eq!(tool_calls[0].2, "{}");
    }

    // ── omit toolConfig ──────────────────────────────────────────────────────────

    /// partial: `should omit toolConfig and filter tool content when conversation has tool calls but no active tools`; full port in src/bedrock/tests/chat_language_model.rs.
    ///
    #[tokio::test]
    async fn omit_tool_config_when_no_active_tools() {
        let server = mock_converse_json(200, ok_converse_body());

        let prompt: LanguageModelPrompt = vec![
            LanguageModelMessage::User {
                content: vec![text("What is the weather in Toronto?")],
                provider_options: None,
            },
            LanguageModelMessage::Assistant {
                content: vec![tool_call(
                    "tool-call-1".to_string(),
                    "weather".to_string(),
                    json!({ "city": "Toronto" }),
                )],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![tool_result(
                    "tool-call-1".to_string(),
                    json!({ "type": "text", "value": "The weather in Toronto is 20°C." }),
                )],
                provider_options: None,
            },
            LanguageModelMessage::User {
                content: vec![text("Now give me a summary.")],
                provider_options: None,
            },
        ];

        let model = make_model(&server);
        let opts = CallOptions {
            tools: Some(vec![]),
            ..default_options(prompt)
        };

        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request.unwrap().body.expect("request body");
        assert!(
            body.get("toolConfig").is_none(),
            "toolConfig should be absent when tools list is empty"
        );
    }

    /// partial: `should omit toolConfig and filter tool content when conversation has tool calls but no active tools`; full port in src/bedrock/tests/chat_language_model.rs.
    /// (same test) the assistant's `toolUse` block should be filtered out of the messages when no tools are active.
    #[tokio::test]
    async fn filter_tool_content_when_no_active_tools() {
        let server = mock_converse_json(200, ok_converse_body());

        let prompt: LanguageModelPrompt = vec![
            LanguageModelMessage::User {
                content: vec![text("What is the weather in Toronto?")],
                provider_options: None,
            },
            LanguageModelMessage::Assistant {
                content: vec![tool_call(
                    "tool-call-1".to_string(),
                    "weather".to_string(),
                    json!({ "city": "Toronto" }),
                )],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![tool_result(
                    "tool-call-1".to_string(),
                    json!({ "type": "text", "value": "The weather in Toronto is 20°C." }),
                )],
                provider_options: None,
            },
            LanguageModelMessage::User {
                content: vec![text("Now give me a summary.")],
                provider_options: None,
            },
        ];

        let model = make_model(&server);
        let opts = CallOptions {
            tools: Some(vec![]),
            ..default_options(prompt)
        };

        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request.unwrap().body.expect("request body");
        assert_eq!(
            body["messages"],
            json!([{ "role": "user", "content": [{ "text": "What is the weather in Toronto?" }, { "text": "Now give me a summary." }] }])
        );
    }

    // ── doGenerate: tool call with empty input (non-streaming) ───────────────────

    /// partial: `should support tool calls with empty input (no arguments)`; full port in src/bedrock/tests/chat_language_model.rs.
    /// (generate).
    #[tokio::test]
    async fn generate_tool_call_empty_input() {
        let server = mock_converse_json(
            200,
            json!({
                "output": {
                    "message": {
                        "role": "assistant",
                        "content": [
                            {
                                "toolUse": {
                                    "toolUseId": "tool_1",
                                    "name": "updateIssueList"
                                    // no "input" field
                                }
                            }
                        ]
                    }
                },
                "stopReason": "tool_use",
                "usage": { "inputTokens": 5, "outputTokens": 5 }
            }),
        );

        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        assert_eq!(result.content.len(), 1);
        let (id, name, input) = as_tool_call(&result.content[0]);
        assert_eq!(id, "tool_1");
        assert_eq!(name, "updateIssueList");
        assert_eq!(input, "{}");
    }

    /// TS: "should generate an id when toolUseId is empty" — an empty
    /// `toolUseId` is replaced by a generated id, as
    /// amazon-bedrock-chat-language-model.ts:796-797 does with `||`.
    #[tokio::test]
    async fn generate_tool_call_empty_tool_use_id() {
        let server = mock_converse_json(
            200,
            json!({
                "output": {
                    "message": {
                        "role": "assistant",
                        "content": [
                            {"toolUse": {"toolUseId": "", "name": "test-tool", "input": {"value": "test"}}}
                        ]
                    }
                },
                "stopReason": "tool_use",
                "usage": { "inputTokens": 1, "outputTokens": 1 }
            }),
        );

        let result = make_model(&server)
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        let (id, name, input) = as_tool_call(&result.content[0]);
        assert!(!id.is_empty(), "expected a generated tool call id");
        assert_eq!(name, "test-tool");
        assert_eq!(input, r#"{"value":"test"}"#);
    }

    // ── doGenerate: basic text + finish reason (sanity, already covered but
    //    exercised here against the remaining-test helper set) ────────────────────

    /// partial: `should pass the model and the messages`; full port in src/bedrock/tests/chat_language_model.rs.
    /// The request body carries the model id implicitly via the URL path and the messages array.
    #[tokio::test]
    async fn request_body_messages_shape() {
        let server = mock_converse_json(200, ok_converse_body());

        let prompt: LanguageModelPrompt = vec![
            LanguageModelMessage::System {
                content: "System Prompt".into(),
                provider_options: None,
            },
            LanguageModelMessage::User {
                content: vec![text("Hello")],
                provider_options: None,
            },
        ];

        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(prompt))
            .await
            .expect("ok");
        let body = result.request.unwrap().body.expect("request body");
        assert_eq!(body["system"], json!([{ "text": "System Prompt" }]));
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["text"], "Hello");
    }

    /// partial: `should extract finish reason`; full port in src/bedrock/tests/chat_language_model.rs.
    /// `guardrail_intervened` → ContentFilter.
    #[tokio::test]
    async fn finish_reason_guardrail_intervened() {
        let server = mock_converse_json(
            200,
            json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "" }] } },
                "stopReason": "guardrail_intervened",
                "usage": { "inputTokens": 4, "outputTokens": 1 }
            }),
        );

        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        assert_eq!(
            result.finish_reason.unified,
            FinishReasonUnified::ContentFilter
        );
        assert_eq!(
            result.finish_reason.raw.as_deref(),
            Some("guardrail_intervened")
        );
    }

    /// partial: `should handle throttlingException error`; full port in src/bedrock/tests/chat_language_model.rs.
    /// (stream).
    #[tokio::test]
    async fn stream_throttling_error() {
        let body_bytes = crate::bedrock::event_stream_decoder::encode_messages(&[(
            "exception",
            "throttlingException",
            r#"{"message":"Throttling Error"}"#,
        )]);
        let server = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![(
                "content-type".into(),
                "application/vnd.amazon.eventstream".into(),
            )],
            body: body_bytes,
        }]);
        let model = make_model(&server);
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;
        assert!(parts.iter().any(
            |part| matches!(part, StreamPart::Error { error } if error.status_code() == Some(429))
        ));
        assert!(parts.iter().any(|part| matches!(part, StreamPart::Finish { finish_reason, .. } if finish_reason.unified == FinishReasonUnified::Error)));
    }

    /// partial: `should handle validationException error`; full port in src/bedrock/tests/chat_language_model.rs.
    /// (stream).
    #[tokio::test]
    async fn stream_validation_error() {
        let body_bytes = crate::bedrock::event_stream_decoder::encode_messages(&[(
            "exception",
            "validationException",
            r#"{"message":"Validation Error"}"#,
        )]);
        let server = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![(
                "content-type".into(),
                "application/vnd.amazon.eventstream".into(),
            )],
            body: body_bytes,
        }]);
        let model = make_model(&server);
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;
        assert!(parts.iter().any(
            |part| matches!(part, StreamPart::Error { error } if error.status_code() == Some(400))
        ));
        assert!(parts.iter().any(|part| matches!(part, StreamPart::Finish { finish_reason, .. } if finish_reason.unified == FinishReasonUnified::Error)));
    }

    // ── doStream: error handling ─────────────────────────────────────────────────

    // ── doStream: request body parity ────────────────────────────────────────────

    /// partial: `should return the request body`; full port in src/bedrock/tests/chat_language_model.rs.
    /// (stream) — the stream result carries the request body for debugging.
    #[tokio::test]
    async fn stream_request_body_available() {
        let events: Vec<(&str, &str, &str)> = vec![
            ("event", "messageStart", r#"{"role":"assistant"}"#),
            (
                "event",
                "contentBlockDelta",
                r#"{"contentBlockIndex":0,"delta":{"text":"hi"}}"#,
            ),
            ("event", "contentBlockStop", r#"{"contentBlockIndex":0}"#),
            ("event", "messageStop", r#"{"stopReason":"end_turn"}"#),
        ];
        let body_bytes = crate::bedrock::event_stream_decoder::encode_messages(&events);

        let server = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![(
                "content-type".into(),
                "application/vnd.amazon.eventstream".into(),
            )],
            body: body_bytes,
        }]);

        let model = make_model(&server);
        let mut opts = default_options(test_prompt());
        opts.max_output_tokens = Some(256);

        let result = model.do_stream(&opts).await.expect("should succeed");
        let body = result.request.unwrap().body.expect("stream request body");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["inferenceConfig"]["maxTokens"], 256);
    }
}

// ── Older ports (previously `tests/bedrock_upstream_e2e_test.rs`) ──

mod bedrock_upstream_e2e {
    use crate::bedrock::tests::upstream_e2e_support::*;

    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::LanguageModelMessage;
    use aimux_core::options::CallOptions;
    use aimux_core::result::GenerateContent;

    use aimux_core::stream_part::StreamPart;
    use aimux_core::tool::FunctionTool;
    use aimux_core::types::FinishReasonUnified;

    use crate::test_support::mock_fetch::{Canned, MockFetch};
    use futures::StreamExt;
    use serde_json::json;

    /// partial: `should extract reasoning text with signature`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn generate_reasoning_signature_and_usage() {
        let response = json!({"output":{"message":{"role":"assistant","content":[
            {"reasoningContent":{"reasoningText":{"text":"I need to think about this problem carefully...","signature":"abc123signature"}}},
            {"text":"The answer is 42."}]}},"usage":{"inputTokens":4,"outputTokens":34,"totalTokens":38},"stopReason":"stop_sequence"});
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let result = provider(&fetch)
            .chat("anthropic.model")
            .do_generate(&options())
            .await
            .unwrap();
        let seen = fetch.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(
            seen[0].url,
            "https://bedrock.test/model/anthropic.model/converse"
        );
        assert_eq!(seen[0].headers["authorization"], "Bearer test-token");
        assert_eq!(seen[0].json_body(), chat_body());
        assert_eq!(result.request.unwrap().body.unwrap(), chat_body());
        assert_eq!(result.content.len(), 2);
        let GenerateContent::Reasoning(reasoning) = &result.content[0] else {
            panic!("expected reasoning")
        };
        assert_eq!(
            reasoning.text,
            "I need to think about this problem carefully..."
        );
        assert_eq!(
            serde_json::to_value(&reasoning.provider_metadata).unwrap(),
            json!({"amazonBedrock":{"signature":"abc123signature"},"bedrock":{"signature":"abc123signature"}})
        );
        assert!(
            matches!(&result.content[1], GenerateContent::Text { text, .. } if text == "The answer is 42.")
        );
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("stop_sequence"));
        assert_eq!(result.usage.input_tokens.total, Some(4));
        assert_eq!(result.usage.input_tokens.no_cache, Some(4));
        assert_eq!(result.usage.output_tokens.total, Some(34));
        assert_eq!(result.usage.output_tokens.text, Some(34));
        assert_eq!(result.usage.raw, response["usage"].as_object().cloned());
    }

    /// partial: `should stream text deltas with metadata and usage`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn stream_text_and_usage() {
        let fetch = MockFetch::new(vec![stream_response(json!([
            {"contentBlockDelta":{"contentBlockIndex":0,"delta":{"text":"Hello"}}},
            {"contentBlockDelta":{"contentBlockIndex":1,"delta":{"text":", "}}},
            {"contentBlockDelta":{"contentBlockIndex":2,"delta":{"text":"World!"}}},
            {"metadata":{"usage":{"inputTokens":4,"outputTokens":34,"totalTokens":38},"metrics":{"latencyMs":10}}},
            {"messageStop":{"stopReason":"stop_sequence"}}
        ]))]);
        let result = provider(&fetch)
            .chat("anthropic.model")
            .do_stream(&options())
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].url,
            "https://bedrock.test/model/anthropic.model/converse-stream"
        );
        assert_eq!(fetch.seen()[0].json_body(), chat_body());
        assert_eq!(result.request.unwrap().body.unwrap(), chat_body());
        let parts = result
            .stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(parts.len(), 9);
        assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
        assert!(
            matches!(&parts[1], StreamPart::ResponseMetadata(meta) if meta.model_id.as_deref() == Some("anthropic.model"))
        );
        for (offset, id, expected) in [(2, "0", "Hello"), (4, "1", ", "), (6, "2", "World!")] {
            assert!(
                matches!(&parts[offset], StreamPart::TextStart { id: actual, .. } if actual == id)
            );
            assert!(
                matches!(&parts[offset + 1], StreamPart::TextDelta { id: actual, delta, .. } if actual == id && delta == expected)
            );
        }
        let StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } = &parts[8]
        else {
            panic!("expected finish")
        };
        assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(finish_reason.raw.as_deref(), Some("stop_sequence"));
        assert_eq!(usage.input_tokens.total, Some(4));
        assert_eq!(usage.input_tokens.no_cache, Some(4));
        assert_eq!(usage.input_tokens.cache_read, Some(0));
        assert_eq!(usage.input_tokens.cache_write, Some(0));
        assert_eq!(usage.output_tokens.total, Some(34));
        assert_eq!(usage.output_tokens.text, Some(34));
        assert_eq!(
            usage.raw,
            json!({"inputTokens":4,"outputTokens":34,"totalTokens":38})
                .as_object()
                .cloned()
        );
    }

    /// partial: `should stream tool deltas`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn stream_partial_tool_arguments() {
        let fetch = MockFetch::new(vec![stream_response(json!([
            {"contentBlockStart":{"contentBlockIndex":0,"start":{"toolUse":{"toolUseId":"tool-use-id","name":"test-tool"}}}},
            {"contentBlockDelta":{"contentBlockIndex":0,"delta":{"toolUse":{"input":"{\"value\":"}}}},
            {"contentBlockDelta":{"contentBlockIndex":0,"delta":{"toolUse":{"input":"\"Sparkle Day\"}"}}}},
            {"contentBlockStop":{"contentBlockIndex":0}},
            {"messageStop":{"stopReason":"tool_use"}}
        ]))]);
        let schema =
            json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"]});
        let mut opts = options();
        opts.tools = Some(vec![FunctionTool::new("test-tool", schema.clone()).into()]);
        let result = provider(&fetch)
            .chat("anthropic.model")
            .do_stream(&opts)
            .await
            .unwrap();
        let mut expected = chat_body();
        expected["toolConfig"] =
            json!({"tools":[{"toolSpec":{"name":"test-tool","inputSchema":{"json":schema}}}]});
        assert_eq!(fetch.seen()[0].json_body(), expected);
        let parts = result
            .stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(parts.len(), 8);
        assert!(
            matches!(&parts[2], StreamPart::ToolInputStart { id, tool_name, .. } if id == "tool-use-id" && tool_name == "test-tool")
        );
        assert!(
            matches!(&parts[3], StreamPart::ToolInputDelta { id, delta, .. } if id == "tool-use-id" && delta == "{\"value\":")
        );
        assert!(
            matches!(&parts[4], StreamPart::ToolInputDelta { id, delta, .. } if id == "tool-use-id" && delta == "\"Sparkle Day\"}")
        );
        assert!(matches!(&parts[5], StreamPart::ToolInputEnd { id, .. } if id == "tool-use-id"));
        let StreamPart::ToolCall(call) = &parts[6] else {
            panic!("expected tool call")
        };
        assert_eq!(call.tool_call_id, "tool-use-id");
        assert_eq!(call.tool_name, "test-tool");
        assert_eq!(call.input, "{\"value\":\"Sparkle Day\"}");
        assert!(
            matches!(&parts[7], StreamPart::Finish { finish_reason, .. } if finish_reason.unified == FinishReasonUnified::ToolCalls && finish_reason.raw.as_deref() == Some("tool_use"))
        );
    }

    /// partial: `should handle JSON response format in streaming`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn stream_json_response_tool_as_text() {
        let fetch = MockFetch::new(vec![stream_response(json!([
            {"contentBlockStart":{"contentBlockIndex":0,"start":{"toolUse":{"toolUseId":"tool-use-id","name":"json"}}}},
            {"contentBlockDelta":{"contentBlockIndex":0,"delta":{"toolUse":{"input":"{\"value\":\"test\"}"}}}},
            {"contentBlockStop":{"contentBlockIndex":0}},
            {"messageStop":{"stopReason":"tool_use"}}
        ]))]);
        let mut call = CallOptions::new(vec![LanguageModelMessage::user_text("Generate JSON")]);
        call.response_format = Some(aimux_core::options::ResponseFormat::Json {
            schema: Some(
                json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}),
            ),
            name: None,
            description: None,
        });
        let result = provider(&fetch)
            .chat("anthropic.model")
            .do_stream(&call)
            .await
            .unwrap();
        let parts = result
            .stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
        assert!(
            matches!(&parts[1], StreamPart::ResponseMetadata(meta) if meta.model_id.as_deref() == Some("anthropic.model"))
        );
        assert_eq!(parts.len(), 6);
        assert!(matches!(&parts[2], StreamPart::TextStart { id, .. } if id == "0"));
        assert!(
            matches!(&parts[3], StreamPart::TextDelta { id, delta, .. } if id == "0" && delta == "{\"value\":\"test\"}")
        );
        assert!(matches!(&parts[4], StreamPart::TextEnd { id, .. } if id == "0"));
        let StreamPart::Finish {
            finish_reason,
            provider_metadata,
            ..
        } = &parts[5]
        else {
            panic!("expected finish")
        };
        assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(finish_reason.raw.as_deref(), Some("tool_use"));
        assert_eq!(
            serde_json::to_value(provider_metadata.as_ref().unwrap()).unwrap(),
            json!({
                "amazonBedrock":{"isJsonResponseFromTool":true,"stopSequence":null},
                "bedrock":{"isJsonResponseFromTool":true,"stopSequence":null}
            })
        );
        assert_eq!(
            fetch.seen()[0].json_body(),
            json!({
                "messages":[{"role":"user","content":[{"text":"Generate JSON"}]}],
                "additionalModelResponseFieldPaths":["/delta/stop_sequence"],
                "toolConfig":{"tools":[{"toolSpec":{"name":"json","description":"Respond with a JSON object.",
                    "inputSchema":{"json":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}}}}],"toolChoice":{"any":{}}}
            })
        );
    }

    /// partial: `should handle Anthropic provider-defined tools`; full port in src/bedrock/tests/chat_language_model.rs.
    #[tokio::test]
    async fn generate_provider_defined_tool_with_input_schema() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "output":{"message":{"role":"assistant","content":[{"toolUse":{"toolUseId":"tool-use-id","name":"bash","input":{"command":"ls -l"}}}]}},
            "usage":{"inputTokens":10,"outputTokens":20,"totalTokens":30},"stopReason":"tool_use"
        }))]);
        let mut call = options();
        call.tools = Some(vec![aimux_core::tool::Tool::Provider(
            aimux_core::tool::ProviderTool {
                id: "anthropic.bash_20241022".parse().unwrap(),
                name: "bash".into(),
                args: serde_json::Map::new(),
                supports_deferred_results: None,
            },
        )]);
        call.tool_choice = Some(aimux_core::options::LanguageModelV4ToolChoice::Auto);
        let result = provider(&fetch)
            .chat("anthropic.model")
            .do_generate(&call)
            .await
            .unwrap();
        let body = fetch.seen()[0].json_body();
        assert_eq!(
            body["additionalModelRequestFields"],
            json!({"tool_choice":{"type":"auto"},"anthropic_beta":["computer-use-2024-10-22"]})
        );
        assert_eq!(
            body["toolConfig"]["tools"],
            json!([{"toolSpec":{"name":"bash","inputSchema":{"json":{
                "$schema":"http://json-schema.org/draft-07/schema#","type":"object",
                "properties":{"command":{"type":"string"},"restart":{"type":"boolean"}},
                "required":["command"],"additionalProperties":false
            }}}}])
        );
        assert!(body["toolConfig"].get("toolChoice").is_none());
        assert_eq!(result.content.len(), 1);
        assert!(result.warnings.is_empty());
        assert!(
            matches!(&result.content[0], GenerateContent::ToolCall(tool) if tool.tool_call_id == "tool-use-id" && tool.tool_name == "bash" && tool.input == "{\"command\":\"ls -l\"}")
        );
    }

    /// The JSON instruction Bedrock adds for an Anthropic model that cannot take
    /// a forced tool, appended to the system prompt
    /// (`@ai-sdk/amazon-bedrock@5.0.100` src/amazon-bedrock-chat-language-model.ts:614-621,
    /// with `injectJsonInstructionIntoMessages` from `@ai-sdk/provider-utils`).
    #[tokio::test]
    async fn json_instruction_text_for_a_model_without_forced_tools() {
        let response = json!({"output":{"message":{"role":"assistant","content":[{"text":"{}"}]}},
            "usage":{"inputTokens":1,"outputTokens":1,"totalTokens":2},"stopReason":"end_turn"});
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let mut call = CallOptions::new(vec![
            LanguageModelMessage::System {
                content: "Be brief.".into(),
                provider_options: None,
            },
            LanguageModelMessage::user_text("Hello"),
        ]);
        call.response_format = Some(aimux_core::options::ResponseFormat::Json {
            schema: Some(schema.clone()),
            name: None,
            description: None,
        });
        call.provider_options = Some(
            aimux_core::shared::provider_namespace(
                "bedrock",
                json!({"structuredOutputMode": "jsonTool"}),
            )
            .unwrap(),
        );
        provider(&fetch)
            .chat("us.anthropic.claude-sonnet-5-5-v1:0")
            .do_generate(&call)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body()["system"],
            json!([{"text": format!(
                "Be brief.\n\nJSON schema:\n{schema}\nYou MUST answer with only a JSON object that matches the JSON schema above. Do not wrap it in markdown fences or include any other text."
            )}])
        );
    }
}
