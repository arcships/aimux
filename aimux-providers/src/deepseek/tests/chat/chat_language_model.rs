//! Ports of `src/chat/deepseek-chat-language-model.test.ts` from `@ai-sdk/deepseek@3.0.56`:
//! `model IDs`, `supportedUrls`, `doGenerate` (`text`, `reasoning`, `logprobs`,
//! `top-level reasoning`, `tool call`, `json response format`, `assistant prefix
//! completion`) and `doStream` (stream errors, `text`, `reasoning`, `logprobs`,
//! `tool call`, `assistant prefix completion`), followed by the aimux cases that
//! have no upstream counterpart.
//!
//! The snapshot cases answer from [`MockFetch`] with the `fixtures/` files; the
//! other cases answer from a local `wiremock` server (`support.rs`, formerly
//! `tests/deepseek_chat_test.rs`).
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `toMatchSnapshot()` values are the `.snap` entries in `snapshots/`, as JSON:
//!   `undefined` properties are absent (as `JSON.stringify` drops them) and the
//!   `Date` timestamps are ISO strings. The result, or each stream part, is
//!   serialized and compared as a `serde_json::Value`.
//! - The `prepare*FixtureResponse` helpers are the `fixtures/` files, served by
//!   [`MockFetch`]. The TypeScript test server adds `content-length` to JSON
//!   responses and `cache-control` / `connection` to event streams; the mock does
//!   the same.
//! - A `stream-chunks` response is one `data:` frame per line of the
//!   `.chunks.txt` fixture, then `data: [DONE]`. Blank lines carry no frame and
//!   are skipped.
//!
//! Not ported: `json response format with structured outputs` (three cases:
//! `should send json_schema response format and skip schema injection`,
//! `should honor strictJsonSchema provider option` and `should fall back to
//! json_object without a schema`). The upstream builds that model directly with
//! `supportsStructuredOutputs: true`. The public API has no such setting:
//! `create_deepseek(..).chat(..)` only turns on the beta-endpoint flags, and the
//! chat model never sends a `json_schema` response format.

use std::sync::Arc;

use crate::deepseek::{DeepSeekProvider, DeepSeekProviderSettings, create_deepseek};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::StreamResult;
use aimux_core::tool::{FunctionTool, Tool};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch};

use super::support::*;

const TEXT_FIXTURE: &str = include_str!("fixtures/deepseek-text.json");
const REASONING_FIXTURE: &str = include_str!("fixtures/deepseek-reasoning.json");
const JSON_FIXTURE: &str = include_str!("fixtures/deepseek-json.json");
const TOOL_CALL_FIXTURE: &str = include_str!("fixtures/deepseek-tool-call.json");
const TEXT_CHUNKS: &str = include_str!("fixtures/deepseek-text.chunks.txt");
const REASONING_CHUNKS: &str = include_str!("fixtures/deepseek-reasoning.chunks.txt");
const TOOL_CALL_CHUNKS: &str = include_str!("fixtures/deepseek-tool-call.chunks.txt");

const SNAPSHOT_DO_GENERATE_TEXT: &str = include_str!("snapshots/do_generate_text.json");
const SNAPSHOT_DO_GENERATE_REASONING: &str = include_str!("snapshots/do_generate_reasoning.json");
const SNAPSHOT_DO_GENERATE_JSON_RESPONSE_FORMAT: &str =
    include_str!("snapshots/do_generate_json_response_format.json");
const SNAPSHOT_DO_GENERATE_TOOL_CALL: &str = include_str!("snapshots/do_generate_tool_call.json");
const SNAPSHOT_DO_STREAM_TEXT: &str = include_str!("snapshots/do_stream_text.json");
const SNAPSHOT_DO_STREAM_REASONING: &str = include_str!("snapshots/do_stream_reasoning.json");
const SNAPSHOT_DO_STREAM_TOOL_CALL: &str = include_str!("snapshots/do_stream_tool_call.json");

// ── helpers ─────────────────────────────────────────────────────────────────

fn json_value(source: &str) -> Value {
    serde_json::from_str(source).expect("fixture or snapshot is JSON")
}

/// `createDeepSeek({ apiKey })` with a mock transport.
fn provider(fetch: &Arc<MockFetch>) -> DeepSeekProvider {
    create_deepseek(DeepSeekProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// `prompt: TEST_PROMPT`.
fn fixture_options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn fixture_weather_tool() -> Tool {
    Tool::Function(FunctionTool::new(
        "weather",
        json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
    ))
}

/// The `tools: [weather]` and `providerOptions.deepseek.thinking: enabled` of
/// the upstream tool-call cases.
fn fixture_tool_options() -> CallOptions {
    let mut call = fixture_options();
    call.tools = Some(vec![fixture_weather_tool()]);
    call.provider_options = Some(
        serde_json::from_value(json!({ "deepseek": { "thinking": { "type": "enabled" } } }))
            .unwrap(),
    );
    call
}

/// `responseFormat: { type: 'json' }` (no schema) on top of [`tool_options`].
fn fixture_json_options() -> CallOptions {
    let mut call = fixture_tool_options();
    call.response_format = Some(ResponseFormat::Json {
        schema: None,
        name: None,
        description: None,
    });
    call
}

/// A non-streaming response with the headers the TypeScript test server sends.
fn json_response(body: &Value) -> Canned {
    let bytes = serde_json::to_vec(body).expect("serializable");
    Canned {
        status: 200,
        headers: vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("content-length".to_string(), bytes.len().to_string()),
        ],
        body: bytes,
    }
}

/// A single-response mock for a JSON body.
fn json_fetch(body: &Value) -> Arc<MockFetch> {
    MockFetch::new(vec![json_response(body)])
}

/// A single event-stream mock: one `data:` frame per payload, then `[DONE]`.
fn stream_fetch(payloads: &[String]) -> Arc<MockFetch> {
    let mut body: String = payloads
        .iter()
        .map(|payload| format!("data: {payload}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![
            ("content-type".to_string(), "text/event-stream".to_string()),
            ("cache-control".to_string(), "no-cache".to_string()),
            ("connection".to_string(), "keep-alive".to_string()),
        ],
        body: body.into_bytes(),
    }])
}

/// A `stream-chunks` mock from a `.chunks.txt` fixture: one frame per non-blank line.
fn chunks_fetch(source: &str) -> Arc<MockFetch> {
    let payloads: Vec<String> = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    stream_fetch(&payloads)
}

/// All parts of a stream, each serialized as the AI SDK writes it.
async fn fixture_stream_parts(result: StreamResult) -> Value {
    Value::Array(
        result
            .stream
            .map(|part| serde_json::to_value(part.expect("stream part")).unwrap())
            .collect()
            .await,
    )
}

// ── doGenerate and doStream ────────────────────────────────────────────────

mod deep_seek_chat_language_model {
    use super::*;

    mod do_generate {
        use super::*;

        mod text {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should extract text content
            #[tokio::test]
            async fn should_extract_text_content() {
                let fetch = json_fetch(&json_value(TEXT_FIXTURE));
                let result = provider(&fetch)
                    .chat("deepseek-chat")
                    .do_generate(&fixture_options())
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&result).unwrap(),
                    json_value(SNAPSHOT_DO_GENERATE_TEXT)
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should send correct request body
            #[tokio::test]
            async fn text_should_send_correct_request_body() {
                let mut options = options_for(vec![
                    LanguageModelMessage::System {
                        content: ("You are a helpful assistant.").into(),
                        provider_options: None,
                    },
                    LanguageModelMessage::user_text("Hello"),
                ]);
                options.temperature = Some(0.5);
                options.top_p = Some(0.3);
                assert_eq!(
                    generate_body("deepseek-chat", &options).await,
                    json!({
                        "messages": [
                            { "content": "You are a helpful assistant.", "role": "system" },
                            { "content": "Hello", "role": "user" }
                        ],
                        "model": "deepseek-chat",
                        "temperature": 0.5,
                        "top_p": 0.3
                    })
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should omit deprecated and ineffective sampling options in default V4 thinking mode
            #[tokio::test]
            async fn text_should_omit_deprecated_and_ineffective_sampling_options_in_default_v4_thinking_mode()
             {
                let mut options = options();
                options.temperature = Some(0.2);
                options.top_p = Some(0.4);
                options.frequency_penalty = Some(0.5);
                options.presence_penalty = Some(0.6);
                let result = generate_result("deepseek-v4-flash", &options).await;
                assert_eq!(
                    result.request.and_then(|request| request.body).unwrap(),
                    json!({ "model": "deepseek-v4-flash", "messages": [{ "role": "user", "content": "Hello" }] })
                );
                assert_eq!(
                    warning_values(&result.warnings),
                    warning_values(&[
                        deprecated("frequencyPenalty"),
                        deprecated("presencePenalty"),
                        thinking_unsupported("temperature"),
                        thinking_unsupported("topP"),
                    ])
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should preserve supported sampling options when V4 thinking is disabled
            #[tokio::test]
            async fn text_should_preserve_supported_sampling_options_when_v4_thinking_is_disabled()
            {
                let mut options = with_provider_options(
                    options(),
                    json!({ "deepseek": { "thinking": { "type": "disabled" } } }),
                );
                options.temperature = Some(0.2);
                options.top_p = Some(0.4);
                options.frequency_penalty = Some(0.5);
                options.presence_penalty = Some(0.6);
                let result = generate_result("deepseek-v4-flash", &options).await;
                assert_eq!(
                    result.request.and_then(|request| request.body).unwrap(),
                    json!({
                        "model": "deepseek-v4-flash",
                        "messages": [{ "role": "user", "content": "Hello" }],
                        "temperature": 0.2,
                        "top_p": 0.4,
                        "thinking": { "type": "disabled" }
                    })
                );
                assert_eq!(
                    warning_values(&result.warnings),
                    warning_values(&[
                        deprecated("frequencyPenalty"),
                        deprecated("presencePenalty")
                    ])
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should send message names
            #[tokio::test]
            async fn text_should_send_message_names() {
                let name = |value: &str| json!({ "deepseek": { "name": value } });
                let messages = wire_messages(
                    "deepseek-chat",
                    vec![
                        with_options(
                            LanguageModelMessage::System {
                                content: ("You are a helpful assistant.").into(),
                                provider_options: None,
                            },
                            name("guide"),
                        ),
                        with_options(LanguageModelMessage::user_text("Hello"), name("alice")),
                        with_options(
                            LanguageModelMessage::Assistant {
                                content: vec![AssistantPart::Text(TextPart {
                                    text: ("Hello, Alice.").into(),
                                    provider_options: None,
                                })],
                                provider_options: None,
                            },
                            name("assistant"),
                        ),
                        with_options(
                            LanguageModelMessage::user_text("How are you?"),
                            name("alice"),
                        ),
                    ],
                )
                .await;
                assert_eq!(
                    messages,
                    [
                        json!({ "content": "You are a helpful assistant.", "name": "guide", "role": "system" }),
                        json!({ "content": "Hello", "name": "alice", "role": "user" }),
                        json!({ "content": "Hello, Alice.", "name": "assistant", "role": "assistant" }),
                        json!({ "content": "How are you?", "name": "alice", "role": "user" }),
                    ]
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should pass providerOptions userId as user_id
            #[tokio::test]
            async fn text_should_pass_provider_options_user_id_as_user_id() {
                let options = with_provider_options(
                    options(),
                    json!({ "deepseek": { "userId": "tenant_123-user" } }),
                );
                assert_eq!(
                    generate_body("deepseek-chat", &options).await["user_id"],
                    "tenant_123-user"
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should omit user_id when userId is not provided
            #[tokio::test]
            async fn text_should_omit_user_id_when_user_id_is_not_provided() {
                assert!(
                    generate_body("deepseek-chat", &options())
                        .await
                        .get("user_id")
                        .is_none()
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should reject strict tools on the standard endpoint before fetching
            #[tokio::test]
            async fn text_should_reject_strict_tools_on_the_standard_endpoint_before_fetching() {
                let mut options = options();
                options.tools = Some(vec![strict_tool("getWeather", Some(true))]);
                let message = generate_error("deepseek-chat", &options).await;
                assert!(
                    message.contains(
                        "DeepSeek strict tool calls require a beta base URL ending in `/beta`."
                    ),
                    "{message}"
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should send all-strict tools on the beta endpoint
            #[tokio::test]
            async fn text_should_send_all_strict_tools_on_the_beta_endpoint() {
                let server = json_server(text_response()).await;
                let mut options = options();
                options.tools = Some(vec![strict_tool("getWeather", Some(true))]);
                let body = beta_chat(&server, "deepseek-chat")
                    .do_generate(&options)
                    .await
                    .unwrap()
                    .request
                    .and_then(|r| r.body)
                    .unwrap();
                assert_eq!(body["tools"][0]["type"], "function");
                assert_eq!(body["tools"][0]["function"]["name"], "getWeather");
                assert_eq!(body["tools"][0]["function"]["strict"], true);
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should reject mixed strict tools in streaming requests
            #[tokio::test]
            async fn text_should_reject_mixed_strict_tools_in_streaming_requests() {
                let server = sse_server(text_chunks()).await;
                let mut options = options();
                options.tools = Some(vec![
                    strict_tool("strictTool", Some(true)),
                    strict_tool("nonStrictTool", None),
                ]);
                let Err(error) = beta_chat(&server, "deepseek-chat")
                    .do_stream(&options)
                    .await
                else {
                    panic!("the call must be rejected");
                };
                assert!(
        error.to_string().contains(
            "DeepSeek strict mode requires every function tool in the request to set `strict: true`."
        ),
        "{error}"
    );
                assert!(request_paths(&server).await.is_empty());
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should reject invalid userId %j before making an API request
            #[tokio::test]
            async fn text_should_reject_invalid_user_id_before_making_an_api_request() {
                for (user_id, expected) in [
                    ("".to_string(), "userId must match /^[a-zA-Z0-9_-]+$/"),
                    (
                        "contains space".to_string(),
                        "userId must match /^[a-zA-Z0-9_-]+$/",
                    ),
                    (
                        "a".repeat(513),
                        "userId must be at most 512 characters long",
                    ),
                ] {
                    let options = with_provider_options(
                        options(),
                        json!({ "deepseek": { "userId": user_id } }),
                    );
                    let message = generate_error("deepseek-chat", &options).await;
                    assert!(message.contains("invalid provider options"), "{message}");
                    assert!(message.contains(expected), "{message}");
                }
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should include the system fingerprint in provider metadata
            #[tokio::test]
            async fn text_should_include_the_system_fingerprint_in_provider_metadata() {
                let result = generate_result("deepseek-chat", &options()).await;
                assert_eq!(
                    result.provider_metadata.unwrap()["deepseek"]["systemFingerprint"],
                    "fp_eaab8d114b_prod0820_fp8_kvcache"
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should tolerate a %s system fingerprint
            #[tokio::test]
            async fn text_should_tolerate_a_null_or_missing_system_fingerprint() {
                for fingerprint in [Some(Value::Null), None] {
                    let mut body = text_response();
                    match fingerprint {
                        Some(value) => body["system_fingerprint"] = value,
                        None => {
                            body.as_object_mut().unwrap().remove("system_fingerprint");
                        }
                    }
                    let server = json_server(body).await;
                    let result = chat(&server, "deepseek-chat")
                        .do_generate(&options())
                        .await
                        .unwrap();
                    assert!(
                        result.provider_metadata.unwrap()["deepseek"]
                            .get("systemFingerprint")
                            .is_none()
                    );
                }
            }
        }

        mod reasoning {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › reasoning › should extract text content
            #[tokio::test]
            async fn should_extract_text_content() {
                let fetch = json_fetch(&json_value(REASONING_FIXTURE));
                let result = provider(&fetch)
                    .chat("deepseek-chat")
                    .do_generate(&fixture_options())
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&result).unwrap(),
                    json_value(SNAPSHOT_DO_GENERATE_REASONING)
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › reasoning › should send correct request body
            #[tokio::test]
            async fn reasoning_should_send_correct_request_body() {
                let options = with_provider_options(
                    options_for(vec![LanguageModelMessage::user_text(
                        "How many \"r\"s are in the word \"strawberry\"?",
                    )]),
                    json!({ "deepseek": { "thinking": { "type": "enabled" } } }),
                );
                assert_eq!(
                    generate_body("deepseek-reasoner", &options).await,
                    json!({
                        "messages": [{ "content": "How many \"r\"s are in the word \"strawberry\"?", "role": "user" }],
                        "model": "deepseek-reasoner",
                        "thinking": { "type": "enabled" }
                    })
                );
            }
        }

        mod tool_call {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › tool call › should extract tool call content
            #[tokio::test]
            async fn should_extract_tool_call_content() {
                let fetch = json_fetch(&json_value(TOOL_CALL_FIXTURE));
                let result = provider(&fetch)
                    .chat("deepseek-reasoner")
                    .do_generate(&fixture_tool_options())
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&result).unwrap(),
                    json_value(SNAPSHOT_DO_GENERATE_TOOL_CALL)
                );
            }

            mod json_response_format {
                use super::*;

                /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › tool call › json response format › should extract text content
                #[tokio::test]
                async fn should_extract_text_content() {
                    let fetch = json_fetch(&json_value(JSON_FIXTURE));
                    let result = provider(&fetch)
                        .chat("deepseek-reasoner")
                        .do_generate(&fixture_json_options())
                        .await
                        .unwrap();
                    assert_eq!(
                        serde_json::to_value(&result).unwrap(),
                        json_value(SNAPSHOT_DO_GENERATE_JSON_RESPONSE_FORMAT)
                    );
                }

                /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › tool call › json response format › should send correct request body without schema
                #[tokio::test]
                async fn json_response_format_should_send_correct_request_body_without_schema() {
                    assert_eq!(
                        generate_body("deepseek-reasoner", &json_options(None)).await,
                        json!({
                            "messages": [
                                { "content": "Return JSON.", "role": "system" },
                                { "content": "Hello", "role": "user" }
                            ],
                            "model": "deepseek-reasoner",
                            "response_format": { "type": "json_object" },
                            "thinking": { "type": "enabled" },
                            "tools": [weather_wire_tool()]
                        })
                    );
                }

                /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › tool call › json response format › should send correct request body with schema
                #[tokio::test]
                async fn json_response_format_should_send_correct_request_body_with_schema() {
                    let schema = json!({
                        "type": "object",
                        "properties": { "elements": { "type": "array", "items": { "type": "object",
                            "properties": { "location": { "type": "string" }, "temperature": { "type": "number" },
                                            "condition": { "type": "string" } },
                            "required": ["location", "temperature", "condition"], "additionalProperties": false } } },
                        "required": ["elements"], "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#"
                    });
                    let result =
                        generate_result("deepseek-reasoner", &json_options(Some(schema.clone())))
                            .await;
                    let body = result.request.and_then(|r| r.body).unwrap();
                    assert_eq!(
                        body["messages"][0],
                        json!({
                            "role": "system",
                            "content": format!("Return JSON that conforms to the following schema: {schema}")
                        })
                    );
                    assert_eq!(
                        body["messages"][1],
                        json!({ "role": "user", "content": "Hello" })
                    );
                    assert_eq!(body["response_format"], json!({ "type": "json_object" }));
                    // upstream: supportsStructuredOutputs is false, so the schema is injected
                    // into the system message with a compatibility warning.
                    assert_eq!(
                        warning_values(&result.warnings),
                        warning_values(&[compatibility(
                            "responseFormat JSON schema",
                            "JSON response schema is injected into the system message."
                        )])
                    );
                }
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › tool call › should send correct request body
            #[tokio::test]
            async fn tool_call_should_send_correct_request_body() {
                assert_eq!(
                    generate_body("deepseek-reasoner", &tool_options()).await,
                    json!({
                        "messages": [{ "content": "Hello", "role": "user" }],
                        "model": "deepseek-reasoner",
                        "thinking": { "type": "enabled" },
                        "tools": [weather_wire_tool()]
                    })
                );
            }
        }

        /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › should reject a response without choices
        #[tokio::test]
        async fn should_reject_a_response_without_choices() {
            let server = json_server(json!({
                "id": "chatcmpl-empty", "object": "chat.completion", "created": 0,
                "model": "deepseek-chat", "choices": [],
                "usage": { "prompt_tokens": 1, "completion_tokens": 0, "total_tokens": 1 }
            }))
            .await;
            let error = chat(&server, "deepseek-chat")
                .do_generate(&options())
                .await
                .unwrap_err();
            assert!(
                matches!(&error, AiMuxError::InvalidResponseData { message: m, .. } if m == "Response did not contain any choices."),
                "{error:?}"
            );
        }

        mod logprobs {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › logprobs › should send logprobs provider options
            #[tokio::test]
            async fn logprobs_should_send_logprobs_provider_options() {
                let options = with_provider_options(
                    options(),
                    json!({ "deepseek": { "logprobs": true, "topLogprobs": 1 } }),
                );
                assert_eq!(
                    generate_body("deepseek-v4-flash", &options).await,
                    json!({
                        "logprobs": true,
                        "messages": [{ "content": "Hello", "role": "user" }],
                        "model": "deepseek-v4-flash",
                        "top_logprobs": 1
                    })
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › logprobs › should enable logprobs when topLogprobs is set
            #[tokio::test]
            async fn logprobs_should_enable_logprobs_when_top_logprobs_is_set() {
                let options =
                    with_provider_options(options(), json!({ "deepseek": { "topLogprobs": 1 } }));
                let body = generate_body("deepseek-v4-flash", &options).await;
                assert_eq!(body["logprobs"], true);
                assert_eq!(body["top_logprobs"], 1);
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › logprobs › should extract content and reasoning logprobs
            #[tokio::test]
            async fn logprobs_should_extract_content_and_reasoning_logprobs() {
                let server = json_server(logprobs_response()).await;
                let options =
                    with_provider_options(options(), json!({ "deepseek": { "logprobs": true } }));
                let result = chat(&server, "deepseek-v4-flash")
                    .do_generate(&options)
                    .await
                    .unwrap();
                assert_eq!(
                    result.provider_metadata.unwrap()["deepseek"]["logprobs"],
                    json!({
                        "content": [logprob("OK", -0.00002467602, json!([79, 75]))],
                        "reasoning_content": [logprob("Reasoning", -0.1, Value::Null)]
                    })
                );
            }
        }

        mod top_level_reasoning {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should map top-level reasoning to thinking enabled
            #[tokio::test]
            async fn top_level_reasoning_should_map_to_thinking_enabled() {
                let body = generate_body(
                    "deepseek-reasoner",
                    &reasoning_options(ReasoningEffort::High),
                )
                .await;
                assert_eq!(body["thinking"], json!({ "type": "enabled" }));
                assert_eq!(body["reasoning_effort"], "high");
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should map top-level reasoning none to thinking disabled
            #[tokio::test]
            async fn top_level_reasoning_none_should_map_to_thinking_disabled() {
                let body = generate_body(
                    "deepseek-reasoner",
                    &reasoning_options(ReasoningEffort::None),
                )
                .await;
                assert_eq!(body["thinking"], json!({ "type": "disabled" }));
                assert!(body.get("reasoning_effort").is_none());
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should map top-level reasoning xhigh to reasoning_effort max
            #[tokio::test]
            async fn top_level_reasoning_xhigh_should_map_to_reasoning_effort_max() {
                let result = generate_result(
                    "deepseek-reasoner",
                    &reasoning_options(ReasoningEffort::Xhigh),
                )
                .await;
                assert_eq!(
                    result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
                    "max"
                );
                assert!(warning_values(&result.warnings).contains(&serde_json::to_value(compatibility(
        "reasoning",
        "reasoning \"xhigh\" is not directly supported by this model. mapped to effort \"max\"."
    ))
    .unwrap()));
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should map top-level reasoning low to reasoning_effort low without a compatibility warning
            #[tokio::test]
            async fn top_level_reasoning_low_should_map_to_reasoning_effort_low_without_a_compatibility_warning()
             {
                let result = generate_result(
                    "deepseek-reasoner",
                    &reasoning_options(ReasoningEffort::Low),
                )
                .await;
                assert_eq!(
                    result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
                    "low"
                );
                assert!(
        !result
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::Compatibility { feature, .. } if feature == "reasoning"))
    );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should map top-level reasoning medium to reasoning_effort high with a compatibility warning
            #[tokio::test]
            async fn top_level_reasoning_medium_should_map_to_reasoning_effort_high_with_a_compatibility_warning()
             {
                let result = generate_result(
                    "deepseek-reasoner",
                    &reasoning_options(ReasoningEffort::Medium),
                )
                .await;
                assert_eq!(
                    result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
                    "high"
                );
                assert_eq!(
                    warning_values(&result.warnings),
                    warning_values(&[compatibility(
                        "reasoning",
                        "reasoning \"medium\" is not directly supported by this model. mapped to effort \"high\"."
                    )])
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should map top-level reasoning minimal to reasoning_effort low with compatibility warning
            #[tokio::test]
            async fn top_level_reasoning_minimal_should_map_to_reasoning_effort_low_with_compatibility_warning()
             {
                let result = generate_result(
                    "deepseek-reasoner",
                    &reasoning_options(ReasoningEffort::Minimal),
                )
                .await;
                assert_eq!(
                    result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
                    "low"
                );
                assert_eq!(
                    warning_values(&result.warnings),
                    warning_values(&[compatibility(
                        "reasoning",
                        "reasoning \"minimal\" is not directly supported by this model. mapped to effort \"low\"."
                    )])
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should map providerOptions reasoningEffort $input to $output
            #[tokio::test]
            async fn top_level_reasoning_should_map_provider_options_reasoning_effort() {
                for (input, output, warns) in [
                    ("low", "low", false),
                    ("medium", "high", true),
                    ("xhigh", "max", true),
                ] {
                    let options = with_provider_options(
                        options(),
                        json!({ "deepseek": { "reasoningEffort": input } }),
                    );
                    let result = generate_result("deepseek-reasoner", &options).await;
                    assert_eq!(
                        result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
                        output
                    );
                    let expected = if warns {
                        vec![compatibility(
                            "reasoningEffort",
                            &format!(
                                "reasoningEffort \"{input}\" is not a canonical DeepSeek value. mapped to \"{output}\"."
                            ),
                        )]
                    } else {
                        vec![]
                    };
                    assert_eq!(warning_values(&result.warnings), warning_values(&expected));
                }
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should map legacy providerOptions thinking.type=adaptive to enabled with a compatibility warning
            #[tokio::test]
            async fn top_level_reasoning_should_map_legacy_thinking_type_adaptive_to_enabled_with_a_compatibility_warning()
             {
                let options = with_provider_options(
                    options(),
                    json!({ "deepseek": { "thinking": { "type": "adaptive" } } }),
                );
                let result = generate_result("deepseek-reasoner", &options).await;
                assert_eq!(
                    result.request.and_then(|r| r.body).unwrap()["thinking"],
                    json!({ "type": "enabled" })
                );
                assert!(warning_values(&result.warnings).contains(&serde_json::to_value(compatibility(
        "thinking.type",
        "thinking.type \"adaptive\" is not a canonical DeepSeek value. mapped to \"enabled\"."
    ))
    .unwrap()));
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should pass providerOptions reasoningEffort
            #[tokio::test]
            async fn top_level_reasoning_should_pass_provider_options_reasoning_effort() {
                let options = with_provider_options(
                    options(),
                    json!({ "deepseek": { "reasoningEffort": "max" } }),
                );
                let body = generate_body("deepseek-reasoner", &options).await;
                assert_eq!(body["reasoning_effort"], "max");
                // When only reasoningEffort is set without thinking, thinking stays unset.
                assert!(body.get("thinking").is_none());
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should prefer providerOptions thinking over top-level reasoning
            #[tokio::test]
            async fn top_level_reasoning_should_prefer_provider_options_thinking_over_top_level_reasoning()
             {
                let options = with_provider_options(
                    reasoning_options(ReasoningEffort::None),
                    json!({ "deepseek": { "thinking": { "type": "enabled" } } }),
                );
                assert_eq!(
                    generate_body("deepseek-reasoner", &options).await["thinking"],
                    json!({ "type": "enabled" })
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should prefer providerOptions reasoningEffort over top-level reasoning
            #[tokio::test]
            async fn top_level_reasoning_should_prefer_provider_options_reasoning_effort_over_top_level_reasoning()
             {
                let options = with_provider_options(
                    reasoning_options(ReasoningEffort::High),
                    json!({ "deepseek": { "reasoningEffort": "max" } }),
                );
                assert_eq!(
                    generate_body("deepseek-reasoner", &options).await["reasoning_effort"],
                    "max"
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › top-level reasoning › should not set thinking when reasoning is not specified
            #[tokio::test]
            async fn top_level_reasoning_should_not_set_thinking_when_reasoning_is_not_specified() {
                assert!(
                    generate_body("deepseek-reasoner", &options())
                        .await
                        .get("thinking")
                        .is_none()
                );
            }
        }

        mod assistant_prefix_completion {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › assistant prefix completion › should send name and prefix on the final assistant message
            #[tokio::test]
            async fn prefix_should_send_name_and_prefix_on_the_final_assistant_message() {
                let server = json_server(text_response()).await;
                let body = beta_chat(&server, "deepseek-chat")
                    .do_generate(&options_for(prefix_prompt(
                        json!({ "name": "assistant", "prefix": true }),
                    )))
                    .await
                    .unwrap()
                    .request
                    .and_then(|r| r.body)
                    .unwrap();
                assert_eq!(
                    body["messages"],
                    json!([
                        { "role": "user", "content": "Complete this sentence." },
                        { "role": "assistant", "content": "The answer is", "name": "assistant", "prefix": true }
                    ])
                );
                assert_eq!(body["model"], "deepseek-chat");
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › assistant prefix completion › should reject prefix completion with the default base URL
            #[tokio::test]
            async fn prefix_should_reject_prefix_completion_with_the_default_base_url() {
                let message = generate_error(
                    "deepseek-chat",
                    &options_for(vec![with_options(
                        LanguageModelMessage::Assistant {
                            content: vec![AssistantPart::Text(TextPart {
                                text: ("The answer is").into(),
                                provider_options: None,
                            })],
                            provider_options: None,
                        },
                        json!({ "deepseek": { "prefix": true } }),
                    )]),
                )
                .await;
                assert!(
        message.contains(
            "DeepSeek assistant prefix completion requires a beta base URL ending in `/beta`."
        ),
        "{message}"
    );
            }
        }
    }

    mod do_stream {
        use super::*;

        mod text {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should stream text
            #[tokio::test]
            async fn should_stream_text() {
                let fetch = chunks_fetch(TEXT_CHUNKS);
                let result = provider(&fetch)
                    .chat("deepseek-chat")
                    .do_stream(&fixture_options())
                    .await
                    .unwrap();
                assert_eq!(
                    fixture_stream_parts(result).await,
                    json_value(SNAPSHOT_DO_STREAM_TEXT)
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should send model id, settings, and input
            #[tokio::test]
            async fn stream_text_should_send_model_id_settings_and_input() {
                let mut options = options_for(vec![
                    LanguageModelMessage::System {
                        content: ("You are a helpful assistant.").into(),
                        provider_options: None,
                    },
                    LanguageModelMessage::user_text("Hello"),
                ]);
                options.temperature = Some(0.5);
                options.top_p = Some(0.3);
                let (body, _) = stream_parts("deepseek-chat", &options, text_chunks()).await;
                assert_eq!(
                    body,
                    json!({
                        "messages": [
                            { "content": "You are a helpful assistant.", "role": "system" },
                            { "content": "Hello", "role": "user" }
                        ],
                        "model": "deepseek-chat",
                        "stream": true,
                        "stream_options": { "include_usage": true },
                        "temperature": 0.5,
                        "top_p": 0.3
                    })
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should omit deprecated and ineffective sampling options in default V4 thinking mode
            #[tokio::test]
            async fn stream_text_should_omit_deprecated_and_ineffective_sampling_options_in_default_v4_thinking_mode()
             {
                let mut options = options();
                options.temperature = Some(0.2);
                options.top_p = Some(0.4);
                options.frequency_penalty = Some(0.5);
                options.presence_penalty = Some(0.6);
                let (body, parts) =
                    stream_parts("deepseek-v4-flash", &options, text_chunks()).await;
                assert_eq!(
                    body,
                    json!({
                        "model": "deepseek-v4-flash",
                        "messages": [{ "role": "user", "content": "Hello" }],
                        "stream": true,
                        "stream_options": { "include_usage": true }
                    })
                );
                assert_eq!(
                    stream_start_warnings(&parts),
                    warning_values(&[
                        deprecated("frequencyPenalty"),
                        deprecated("presencePenalty"),
                        thinking_unsupported("temperature"),
                        thinking_unsupported("topP"),
                    ])
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should preserve supported sampling options when V4 thinking is disabled
            #[tokio::test]
            async fn stream_text_should_preserve_supported_sampling_options_when_v4_thinking_is_disabled()
             {
                let mut options = with_provider_options(
                    options(),
                    json!({ "deepseek": { "thinking": { "type": "disabled" } } }),
                );
                options.temperature = Some(0.2);
                options.top_p = Some(0.4);
                options.frequency_penalty = Some(0.5);
                options.presence_penalty = Some(0.6);
                let (body, parts) =
                    stream_parts("deepseek-v4-flash", &options, text_chunks()).await;
                assert_eq!(
                    body,
                    json!({
                        "model": "deepseek-v4-flash",
                        "messages": [{ "role": "user", "content": "Hello" }],
                        "temperature": 0.2,
                        "top_p": 0.4,
                        "thinking": { "type": "disabled" },
                        "stream": true,
                        "stream_options": { "include_usage": true }
                    })
                );
                assert_eq!(
                    stream_start_warnings(&parts),
                    warning_values(&[
                        deprecated("frequencyPenalty"),
                        deprecated("presencePenalty")
                    ])
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should send message names
            #[tokio::test]
            async fn stream_text_should_send_message_names() {
                let name = |value: &str| json!({ "deepseek": { "name": value } });
                let options = options_for(vec![
                    with_options(
                        LanguageModelMessage::System {
                            content: ("You are a helpful assistant.").into(),
                            provider_options: None,
                        },
                        name("guide"),
                    ),
                    with_options(LanguageModelMessage::user_text("Hello"), name("alice")),
                    with_options(
                        LanguageModelMessage::Assistant {
                            content: vec![AssistantPart::Text(TextPart {
                                text: ("Hello, Alice.").into(),
                                provider_options: None,
                            })],
                            provider_options: None,
                        },
                        name("assistant"),
                    ),
                ]);
                let (body, _) = stream_parts("deepseek-chat", &options, text_chunks()).await;
                assert_eq!(
                    body["messages"],
                    json!([
                        { "content": "You are a helpful assistant.", "name": "guide", "role": "system" },
                        { "content": "Hello", "name": "alice", "role": "user" },
                        { "content": "Hello, Alice.", "name": "assistant", "role": "assistant" }
                    ])
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should pass providerOptions userId as user_id
            #[tokio::test]
            async fn stream_text_should_pass_provider_options_user_id_as_user_id() {
                let options = with_provider_options(
                    options(),
                    json!({ "deepseek": { "userId": "tenant_123-user" } }),
                );
                let (body, _) = stream_parts("deepseek-chat", &options, text_chunks()).await;
                assert_eq!(body["user_id"], "tenant_123-user");
                assert_eq!(body["stream"], true);
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should include the repeated system fingerprint in provider metadata
            #[tokio::test]
            async fn stream_text_should_include_the_repeated_system_fingerprint_in_provider_metadata()
             {
                let (_, parts) = stream_parts("deepseek-chat", &options(), text_chunks()).await;
                assert_eq!(
                    finish_metadata(&parts)["deepseek"]["systemFingerprint"],
                    "fp_eaab8d114b_prod0820_fp8_kvcache"
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should keep the latest non-null system fingerprint
            #[tokio::test]
            async fn stream_text_should_keep_the_latest_non_null_system_fingerprint() {
                let chunks = vec![
        "data: {\"system_fingerprint\":\"fp_initial\",\"choices\":[{\"delta\":{\"content\":\"OK\"},\"finish_reason\":null}],\"usage\":null}\n\n".to_string(),
        "data: {\"system_fingerprint\":null,\"choices\":[{\"delta\":{},\"finish_reason\":null}],\"usage\":null}\n\n".to_string(),
        "data: {\"system_fingerprint\":\"fp_latest\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":null}\n\n".to_string(),
    ];
                let (_, parts) = stream_parts("deepseek-chat", &options(), chunks).await;
                assert_eq!(
                    finish_metadata(&parts)["deepseek"]["systemFingerprint"],
                    "fp_latest"
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should tolerate a %s system fingerprint
            #[tokio::test]
            async fn stream_text_should_tolerate_a_null_or_missing_system_fingerprint() {
                for fingerprint in [Some(Value::Null), None] {
                    let mut chunk = json!({
                        "choices": [{ "delta": { "content": "OK" }, "finish_reason": "stop" }],
                        "usage": null
                    });
                    if let Some(value) = fingerprint {
                        chunk["system_fingerprint"] = value;
                    }
                    let (_, parts) =
                        stream_parts("deepseek-chat", &options(), vec![data_event(&chunk)]).await;
                    assert!(
                        finish_metadata(&parts)["deepseek"]
                            .get("systemFingerprint")
                            .is_none()
                    );
                }
            }
        }

        mod reasoning {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › reasoning › should stream reasoning
            #[tokio::test]
            async fn should_stream_reasoning() {
                let fetch = chunks_fetch(REASONING_CHUNKS);
                let result = provider(&fetch)
                    .chat("deepseek-reasoning")
                    .do_stream(&fixture_options())
                    .await
                    .unwrap();
                assert_eq!(
                    fixture_stream_parts(result).await,
                    json_value(SNAPSHOT_DO_STREAM_REASONING)
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › reasoning › should keep reasoning active when deltas include empty tool calls
            #[tokio::test]
            async fn stream_reasoning_should_keep_reasoning_active_when_deltas_include_empty_tool_calls()
             {
                let chunk = |delta: Value, finish: Value| {
                    data_event(&json!({
                        "id": "chatcmpl-test", "object": "chat.completion.chunk", "created": 1,
                        "model": "test-model",
                        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
                    }))
                };
                let chunks = vec![
                    chunk(
                        json!({ "role": "assistant", "content": "", "reasoning_content": "Think ", "tool_calls": [] }),
                        Value::Null,
                    ),
                    chunk(
                        json!({ "content": "", "reasoning_content": "more...", "tool_calls": [] }),
                        Value::Null,
                    ),
                    chunk(
                        json!({ "content": "Hello", "reasoning_content": "", "tool_calls": [] }),
                        json!("stop"),
                    ),
                ];
                let (_, parts) = stream_parts("deepseek-reasoner", &options(), chunks).await;
                let reasoning: Vec<String> = parts
                    .iter()
                    .filter_map(|part| match part {
                        StreamPart::ReasoningStart { id, .. } => Some(format!("start {id}")),
                        StreamPart::ReasoningDelta { id, delta, .. } => {
                            Some(format!("delta {id} {delta}"))
                        }
                        StreamPart::ReasoningEnd { id, .. } => Some(format!("end {id}")),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    reasoning,
                    [
                        "start reasoning-0",
                        "delta reasoning-0 Think ",
                        "delta reasoning-0 more...",
                        "end reasoning-0"
                    ]
                );
            }

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › reasoning › should map legacy thinking and generic reasoning to canonical request values
            #[tokio::test]
            async fn stream_reasoning_should_map_legacy_thinking_and_generic_reasoning_to_canonical_request_values()
             {
                let options = with_provider_options(
                    reasoning_options(ReasoningEffort::Medium),
                    json!({ "deepseek": { "thinking": { "type": "adaptive" } } }),
                );
                let (body, parts) =
                    stream_parts("deepseek-reasoner", &options, text_chunks()).await;
                assert_eq!(body["thinking"], json!({ "type": "enabled" }));
                assert_eq!(body["reasoning_effort"], "high");
                assert_eq!(
                    stream_start_warnings(&parts),
                    warning_values(&[
                        compatibility(
                            "thinking.type",
                            "thinking.type \"adaptive\" is not a canonical DeepSeek value. mapped to \"enabled\"."
                        ),
                        compatibility(
                            "reasoning",
                            "reasoning \"medium\" is not directly supported by this model. mapped to effort \"high\"."
                        ),
                    ])
                );
            }
        }

        mod tool_call {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › tool call › should stream tool call
            #[tokio::test]
            async fn should_stream_tool_call() {
                let fetch = chunks_fetch(TOOL_CALL_CHUNKS);
                let result = provider(&fetch)
                    .chat("deepseek-reasoner")
                    .do_stream(&fixture_tool_options())
                    .await
                    .unwrap();
                assert_eq!(
                    fixture_stream_parts(result).await,
                    json_value(SNAPSHOT_DO_STREAM_TOOL_CALL)
                );
            }
        }

        /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › should preserve a provider error envelope in stream errors
        #[tokio::test]
        async fn stream_should_preserve_a_provider_error_envelope_in_stream_errors() {
            let data = json!({ "error": {
        "message": "Rate limit reached", "type": "rate_limit_error", "code": "rate_limit_exceeded"
    } });
            let error = stream_error(&data).await;
            assert_eq!(error.message, "Rate limit reached");
            assert_eq!(error.provider_code.as_deref(), Some("rate_limit_exceeded"));
            assert_eq!(error.status_code, Some(429));
            assert!(error.is_retryable);
            // upstream: `data` is the whole error envelope.
            let envelope: Value =
                serde_json::from_str(error.response_body.as_deref().unwrap()).unwrap();
            assert_eq!(envelope, data);
        }

        /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › should classify insufficient quota as non-retryable
        #[tokio::test]
        async fn stream_should_classify_insufficient_quota_as_non_retryable() {
            let data = json!({ "error": {
        "message": "You exceeded your current quota.", "type": "rate_limit_error",
        "code": "insufficient_quota"
    } });
            let error = stream_error(&data).await;
            assert_eq!(error.status_code, Some(429));
            assert!(!error.is_retryable);
        }

        /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › should preserve the provider type when code is an HTTP status
        #[tokio::test]
        async fn stream_should_preserve_the_provider_type_when_code_is_an_http_status() {
            let data = json!({ "error": {
        "message": "Rate limit reached", "type": "rate_limit_error", "code": "429"
    } });
            let error = stream_error(&data).await;
            assert_eq!(error.provider_code.as_deref(), Some("429"));
            assert_eq!(error.status_code, Some(429));
            assert!(error.is_retryable);
        }

        mod logprobs {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › logprobs › should send logprobs provider options and collect streamed logprobs
            #[tokio::test]
            async fn stream_logprobs_should_send_logprobs_provider_options_and_collect_streamed_logprobs()
             {
                let chunk = |choice: Value, usage: Value| {
                    data_event(&json!({
                        "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "deepseek-v4-flash",
                        "choices": [choice], "usage": usage
                    }))
                };
                let chunks = vec![
                    chunk(
                        json!({ "index": 0, "delta": { "role": "assistant", "reasoning_content": "Reasoning" },
                    "logprobs": { "reasoning_content": [logprob("Reasoning", -0.1, Value::Null)] },
                    "finish_reason": null }),
                        Value::Null,
                    ),
                    chunk(
                        json!({ "index": 0, "delta": { "content": "OK" },
                    "logprobs": { "content": [logprob("OK", -0.00002467602, json!([79, 75]))] },
                    "finish_reason": "stop" }),
                        json!({ "prompt_tokens": 9, "completion_tokens": 2, "total_tokens": 11,
                    "prompt_cache_hit_tokens": 0, "prompt_cache_miss_tokens": 9 }),
                    ),
                ];
                let options = with_provider_options(
                    options(),
                    json!({ "deepseek": { "logprobs": true, "topLogprobs": 1 } }),
                );
                let (body, parts) = stream_parts("deepseek-v4-flash", &options, chunks).await;
                assert_eq!(body["logprobs"], true);
                assert_eq!(body["top_logprobs"], 1);
                assert_eq!(
                    finish_metadata(&parts),
                    json!({ "deepseek": {
            "choiceIndex": 0,
            "logprobs": {
                "content": [logprob("OK", -0.00002467602, json!([79, 75]))],
                "reasoning_content": [logprob("Reasoning", -0.1, Value::Null)]
            },
            "messageRole": "assistant",
            "promptCacheHitTokens": 0,
            "promptCacheMissTokens": 9,
            "responseObject": "chat.completion.chunk"
        } })
                );
            }
        }

        mod assistant_prefix_completion {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › assistant prefix completion › should send prefix true on the final assistant message
            #[tokio::test]
            async fn stream_prefix_should_send_prefix_true_on_the_final_assistant_message() {
                let server = sse_server(text_chunks()).await;
                let result = beta_chat(&server, "deepseek-chat")
                    .do_stream(&options_for(prefix_prompt(json!({ "prefix": true }))))
                    .await
                    .unwrap();
                let body = result.request.and_then(|r| r.body).unwrap();
                assert_eq!(
                    body["messages"],
                    json!([
                        { "role": "user", "content": "Complete this sentence." },
                        { "role": "assistant", "content": "The answer is", "prefix": true }
                    ])
                );
                assert_eq!(body["model"], "deepseek-chat");
                assert_eq!(body["stream"], true);
            }
        }
    }

    mod model_ids {
        use super::*;

        /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › model IDs › should forward the %s model ID
        #[tokio::test]
        async fn should_forward_the_model_id() {
            for model_id in ["deepseek-v4-flash", "deepseek-v4-pro"] {
                assert_eq!(generate_body(model_id, &options()).await["model"], model_id);
            }
        }
    }

    mod supported_urls {
        use super::*;

        /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › supportedUrls › should natively support HTTP image URLs
        #[tokio::test]
        async fn should_natively_support_http_image_urls() {
            let server = MockServer::start().await;
            let urls = chat(&server, "deepseek-chat").supported_urls().await;
            let patterns = &urls.0["image/*"];
            assert_eq!(patterns.len(), 1);
            assert!(patterns[0].is_match("https://example.com/a.png"));
            assert!(patterns[0].is_match("http://example.com/a.png"));
            assert!(!patterns[0].is_match("data:image/png;base64,AAAA"));
        }
    }
}

// ---- Tests without a matching upstream case (aimux-own, or a partial port) ----

#[tokio::test]
async fn text_should_send_max_tokens_and_stop_sequences() {
    let mut options = options();
    options.max_output_tokens = Some(64);
    options.stop_sequences = Some(vec!["END".to_string()]);
    let body = generate_body("deepseek-chat", &options).await;
    assert_eq!(body["max_tokens"], 64);
    assert_eq!(body["stop"], json!(["END"]));
    assert!(body.get("max_completion_tokens").is_none());
}

#[tokio::test]
async fn text_should_not_send_unknown_provider_options() {
    // upstream: `deepseekLanguageModelChatOptions` is a zod object, which
    // strips keys it does not declare.
    let options =
        with_provider_options(options(), json!({ "deepseek": { "user": "u1", "foo": 1 } }));
    let body = generate_body("deepseek-chat", &options).await;
    assert!(body.get("user").is_none() && body.get("foo").is_none());
}

#[tokio::test]
async fn text_should_extract_text_content() {
    let result = generate_result("deepseek-chat", &options()).await;
    assert_eq!(result.content.len(), 1);
    assert!(
        matches!(&result.content[0], GenerateContent::Text { text, .. } if text == "Hello, World!")
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("stop"));
    assert_eq!(result.usage.input_tokens.total, Some(13));
    assert_eq!(result.usage.output_tokens.total, Some(300));
    assert_eq!(
        result.response.as_ref().and_then(|r| r.id.as_deref()),
        Some("00f10ecd-60b3-4707-b5db-e4bcadf7aea1")
    );
    assert_eq!(
        result.response.as_ref().and_then(|r| r.model_id.as_deref()),
        Some("deepseek-chat")
    );
}

#[tokio::test]
async fn text_should_report_usage_with_prompt_cache_tokens() {
    // upstream convert-to-deepseek-usage.ts: cache hits are the cache-read tokens.
    let mut body = text_response();
    body["usage"] = json!({
        "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
        "prompt_cache_hit_tokens": 70, "prompt_cache_miss_tokens": 30,
        "completion_tokens_details": { "reasoning_tokens": 5 }
    });
    let server = json_server(body).await;
    let usage = chat(&server, "deepseek-chat")
        .do_generate(&options())
        .await
        .unwrap()
        .usage;
    assert_eq!(usage.input_tokens.total, Some(100));
    assert_eq!(usage.input_tokens.cache_read, Some(70));
    assert_eq!(usage.input_tokens.no_cache, Some(30));
    assert_eq!(usage.output_tokens.total, Some(20));
    assert_eq!(usage.output_tokens.reasoning, Some(5));
    assert_eq!(usage.output_tokens.text, Some(15));
    assert_eq!(usage.raw.unwrap()["prompt_cache_hit_tokens"], 70);
}

#[tokio::test]
async fn text_should_report_the_provider_metadata() {
    // upstream doGenerate: promptCacheHitTokens, promptCacheMissTokens,
    // responseObject, choiceIndex, messageRole, systemFingerprint.
    let result = generate_result("deepseek-chat", &options()).await;
    assert_eq!(
        serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
        json!({ "deepseek": {
            "promptCacheHitTokens": 0,
            "promptCacheMissTokens": 13,
            "responseObject": "chat.completion",
            "choiceIndex": 0,
            "messageRole": "assistant",
            "systemFingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache"
        } })
    );
}

#[tokio::test]
async fn reasoning_should_extract_reasoning_before_text() {
    let server = json_server(reasoning_response()).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&options())
        .await
        .unwrap();
    assert_eq!(result.content.len(), 2);
    assert!(
        matches!(&result.content[0], GenerateContent::Reasoning(ReasoningOutput { text, .. }) if text == "Let me count the letters.")
    );
    assert!(
        matches!(&result.content[1], GenerateContent::Text { text, .. } if text == "There are 3 r's in strawberry.")
    );
    assert_eq!(result.usage.output_tokens.reasoning, Some(10));
    assert_eq!(result.usage.output_tokens.text, Some(20));
}

#[tokio::test]
async fn reasoning_should_not_read_the_reasoning_field() {
    // upstream deepseekChatResponseSchema only declares `reasoning_content`.
    let mut body = reasoning_response();
    let message = body["choices"][0]["message"].as_object_mut().unwrap();
    let reasoning = message.remove("reasoning_content").unwrap();
    message.insert("reasoning".to_string(), reasoning);
    let server = json_server(body).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&options())
        .await
        .unwrap();
    assert_eq!(result.content.len(), 1);
    assert!(matches!(&result.content[0], GenerateContent::Text { .. }));
}

#[tokio::test]
async fn top_level_reasoning_none_should_keep_the_temperature() {
    // thinking is disabled, so temperature applies and is not warned about.
    let mut options = reasoning_options(ReasoningEffort::None);
    options.temperature = Some(0.4);
    let result = generate_result("deepseek-reasoner", &options).await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["temperature"],
        0.4
    );
    assert!(result.warnings.is_empty());
}

#[tokio::test]
async fn top_level_reasoning_provider_default_sends_neither_thinking_nor_effort() {
    let body = generate_body(
        "deepseek-reasoner",
        &reasoning_options(ReasoningEffort::ProviderDefault),
    )
    .await;
    assert!(body.get("thinking").is_none() && body.get("reasoning_effort").is_none());
}

#[tokio::test]
async fn tool_call_should_send_the_tool_choice() {
    use aimux_core::options::LanguageModelV4ToolChoice;
    let mut options = tool_options();
    options.tool_choice = Some(LanguageModelV4ToolChoice::Tool {
        tool_name: "weather".to_string(),
    });
    assert_eq!(
        generate_body("deepseek-reasoner", &options).await["tool_choice"],
        json!({ "type": "function", "function": { "name": "weather" } })
    );
    options.tool_choice = Some(LanguageModelV4ToolChoice::Required);
    assert_eq!(
        generate_body("deepseek-reasoner", &options).await["tool_choice"],
        "required"
    );
}

#[tokio::test]
async fn tool_call_should_warn_about_provider_defined_tools() {
    use aimux_core::tool::ProviderTool;
    let mut options = options();
    options.tools = Some(vec![Tool::Provider(ProviderTool {
        id: "test.search".parse().unwrap(),
        name: "search".to_string(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
    })]);
    let result = generate_result("deepseek-chat", &options).await;
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[Warning::Unsupported {
            feature: "provider-defined tool test.search".to_string(),
            details: None,
        }])
    );
    assert!(
        result.request.and_then(|r| r.body).unwrap()["tools"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn tool_call_should_extract_tool_call_content() {
    let server = json_server(tool_call_response()).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&tool_options())
        .await
        .unwrap();
    assert!(
        matches!(&result.content[0], GenerateContent::Reasoning(ReasoningOutput { text, .. }) if text == "I should look up the weather.")
    );
    assert!(matches!(
        &result.content[1],
        GenerateContent::ToolCall(RawToolCall { tool_call_id, tool_name, input, .. })
            if tool_call_id == "call_00_abc" && tool_name == "weather"
                && input == "{\"location\":\"San Francisco\"}"
    ));
    assert_eq!(result.content.len(), 2);
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    assert_eq!(
        result.provider_metadata.unwrap()["deepseek"]["toolCallTypes"],
        json!(["function"])
    );
}

#[tokio::test]
async fn tool_call_should_generate_an_id_when_the_server_sends_none() {
    let mut body = tool_call_response();
    body["choices"][0]["message"]["tool_calls"][0]["id"] = Value::Null;
    let server = json_server(body).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&tool_options())
        .await
        .unwrap();
    assert!(matches!(
        &result.content[1],
        GenerateContent::ToolCall(RawToolCall { tool_call_id, .. }) if !tool_call_id.is_empty()
    ));
}

#[tokio::test]
async fn json_response_format_should_extract_text_content() {
    let mut body = text_response();
    body["choices"][0]["message"]["content"] = json!("{\"elements\":[]}");
    let server = json_server(body).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&json_options(None))
        .await
        .unwrap();
    assert!(
        matches!(&result.content[0], GenerateContent::Text { text, .. } if text == "{\"elements\":[]}")
    );
}

/// `deepSeekErrorSchema` as the chat model's failed-response handler
/// (deepseek@3.0.56 `deepseek-chat-api-types.ts:117-124`,
/// `deepseek-chat-language-model.ts:213-217`): `code` may be a number.
#[tokio::test]
async fn a_failed_response_reads_deepseeks_error_schema() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(".*/chat/completions$"))
        .respond_with(ResponseTemplate::new(402).set_body_json(json!({ "error": {
            "message": "Insufficient Balance", "type": "unknown_error", "param": null, "code": 4020
        } })))
        .mount(&server)
        .await;
    let Err(AiMuxError::ApiCall(error)) =
        chat(&server, "deepseek-chat").do_generate(&options()).await
    else {
        panic!("expected an API call error");
    };
    assert_eq!(error.message, "Insufficient Balance");
    assert_eq!(error.provider_code.as_deref(), Some("4020"));
    assert_eq!(error.status_code, Some(402));
}

#[tokio::test]
async fn stream_text_should_stream_text() {
    let (_, parts) = stream_parts("deepseek-chat", &options(), text_chunks()).await;
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    assert!(matches!(
        &parts[1],
        StreamPart::ResponseMetadata(ResponseMetadata { id, model_id, .. })
            if id.as_deref() == Some("c1") && model_id.as_deref() == Some("deepseek-chat")
    ));
    let deltas: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextDelta { id, delta, .. } if id == "txt-0" => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, ["Hello", ", World!"]);
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::TextStart { id, .. } if id == "txt-0"))
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::TextEnd { id, .. } if id == "txt-0"))
    );
    match parts.last().unwrap() {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(finish_reason.raw.as_deref(), Some("stop"));
            // usage with prompt-cache tokens
            assert_eq!(usage.input_tokens.total, Some(13));
            assert_eq!(usage.input_tokens.cache_read, Some(5));
            assert_eq!(usage.input_tokens.no_cache, Some(8));
            assert_eq!(usage.output_tokens.total, Some(3));
        }
        other => panic!("expected finish, got {other:?}"),
    }
    assert_eq!(
        finish_metadata(&parts),
        json!({ "deepseek": {
            "promptCacheHitTokens": 5,
            "promptCacheMissTokens": 8,
            "responseObject": "chat.completion.chunk",
            "choiceIndex": 0,
            "messageRole": "assistant",
            "systemFingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache"
        } })
    );
}

#[tokio::test]
async fn stream_reasoning_should_stream_reasoning_before_text() {
    let chunk = |delta: Value, finish: Value| {
        data_event(&json!({
            "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "deepseek-reasoner",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
        }))
    };
    let chunks = vec![
        chunk(
            json!({ "role": "assistant", "content": null, "reasoning_content": "Let me " }),
            Value::Null,
        ),
        chunk(
            json!({ "content": null, "reasoning_content": "think." }),
            Value::Null,
        ),
        chunk(json!({ "content": "Answer" }), Value::Null),
        chunk(json!({ "content": "" }), json!("stop")),
    ];
    let (_, parts) = stream_parts("deepseek-reasoner", &options(), chunks).await;
    let order: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ReasoningStart { .. } => Some("reasoning-start"),
            StreamPart::ReasoningDelta { .. } => Some("reasoning-delta"),
            StreamPart::ReasoningEnd { .. } => Some("reasoning-end"),
            StreamPart::TextStart { .. } => Some("text-start"),
            StreamPart::TextDelta { .. } => Some("text-delta"),
            StreamPart::TextEnd { .. } => Some("text-end"),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        [
            "reasoning-start",
            "reasoning-delta",
            "reasoning-delta",
            "text-start",
            "reasoning-end",
            "text-delta",
            "text-end"
        ]
    );
}

#[tokio::test]
async fn stream_tool_call_should_stream_tool_call() {
    let chunk = |delta: Value, finish: Value| {
        data_event(&json!({
            "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "deepseek-reasoner",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
        }))
    };
    let chunks = vec![
        chunk(
            json!({ "role": "assistant", "reasoning_content": "Look it up." }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "id": "call_00_abc", "type": "function",
                    "function": { "name": "weather", "arguments": "" } }] }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "{\"location\":" } }] }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "\"San Francisco\"}" } }] }),
            Value::Null,
        ),
        chunk(json!({ "content": "" }), json!("tool_calls")),
    ];
    let (_, parts) = stream_parts("deepseek-reasoner", &tool_options(), chunks).await;
    assert!(parts.iter().any(|p| matches!(
        p,
        StreamPart::ToolInputStart { id, tool_name, .. } if id == "call_00_abc" && tool_name == "weather"
    )));
    let input: String = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolInputDelta { id, delta, .. } if id == "call_00_abc" => {
                Some(delta.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(input, "{\"location\":\"San Francisco\"}");
    assert!(parts.iter().any(|p| matches!(
        p,
        StreamPart::ToolCall(RawToolCall { tool_call_id, tool_name, input, .. })
            if tool_call_id == "call_00_abc" && tool_name == "weather"
                && input == "{\"location\":\"San Francisco\"}"
    )));
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { .. }))
    );
    match parts.last().unwrap() {
        StreamPart::Finish { finish_reason, .. } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
        }
        other => panic!("expected finish, got {other:?}"),
    }
    assert_eq!(
        finish_metadata(&parts)["deepseek"]["toolCallTypes"],
        json!(["function"])
    );
}

/// aimux's own: a usage count that is not a number fails the response, as
/// `tokenUsageSchema` requires numbers or null (deepseek-chat-api-types.ts:95-113).
#[tokio::test]
async fn generate_rejects_a_usage_count_that_is_not_a_number() {
    let mut body = text_response();
    body["usage"]["prompt_tokens"] = json!("100");
    let server = json_server(body).await;
    let error = chat(&server, "deepseek-chat")
        .do_generate(&options())
        .await
        .expect_err("a string count must be rejected");
    assert!(
        error
            .to_string()
            .contains("usage/prompt_tokens must be a number"),
        "{error}"
    );
}

/// aimux's own: the same check on a stream chunk's usage.
#[tokio::test]
async fn stream_rejects_a_usage_count_that_is_not_a_number() {
    let mut chunks = text_chunks();
    chunks.pop();
    chunks.push(data_event(&json!({
        "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
        "model": "deepseek-chat",
        "choices": [{ "index": 0, "delta": { "content": "" }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 13, "completion_tokens": "3", "total_tokens": 16 }
    })));
    let server = sse_server(chunks).await;
    let parts = collect(
        chat(&server, "deepseek-chat")
            .do_stream(&options())
            .await
            .unwrap(),
    )
    .await;
    assert!(
        parts
            .iter()
            .any(|part| matches!(part, StreamPart::Error { .. })),
        "{parts:?}"
    );
}

/// A chunk with `"error": null` is a normal chunk (#397): upstream's chunk
/// schema is a union with the error schema, and a chunk with `choices` takes
/// the first branch, which drops the unknown key.
#[tokio::test]
async fn a_null_error_member_is_not_an_error_frame() {
    let chunks = vec![
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat",
            "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "Hello" }, "finish_reason": null }],
            "error": null
        })),
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat",
            "choices": [{ "index": 0, "delta": { "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 },
            "error": null
        })),
    ];
    let server = sse_server(chunks).await;
    let parts = collect(
        chat(&server, "deepseek-chat")
            .do_stream(&options())
            .await
            .unwrap(),
    )
    .await;
    assert!(
        !parts
            .iter()
            .any(|part| matches!(part, StreamPart::Error { .. })),
        "{parts:?}"
    );
    assert!(
        parts
            .iter()
            .any(|part| matches!(part, StreamPart::TextDelta { delta, .. } if delta == "Hello")),
        "{parts:?}"
    );
    assert!(
        matches!(
            parts.last(),
            Some(StreamPart::Finish { finish_reason, .. })
                if finish_reason.unified == FinishReasonUnified::Stop
        ),
        "{parts:?}"
    );
}

/// aimux's own: an option of the wrong type fails as `parseProviderOptions`
/// fails.
#[tokio::test]
async fn wrong_typed_options_fail_with_the_parse_provider_options_message() {
    let options = with_provider_options(options(), json!({ "deepseek": { "userId": 1 } }));
    assert_eq!(
        generate_error("deepseek-chat", &options).await,
        "invalid argument: Invalid argument for parameter providerOptions: invalid deepseek provider options"
    );
}
