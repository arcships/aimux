//! Ports of `src/groq-chat-language-model.test.ts` from `@ai-sdk/groq@4.0.52`.
//!
//! Two harnesses share the file: the snapshot, response-metadata, header,
//! structured-output, tool-call, reasoning and raw-chunk cases use the
//! [`MockFetch`] transport; the older ones (formerly `tests/groq_test.rs`) a
//! wiremock `MockServer` from [`super::support`]. Some older cases are partial
//! versions of cases ported in full here, and are marked `partial:`.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `toMatchSnapshot` values are the `.snap` entries in `snapshots/`, with
//!   `undefined` properties absent (as `JSON.stringify` drops them).
//! - `undefined` properties in inline snapshots are absent keys in the
//!   `json!` expectations, for the same reason.
//! - The TypeScript test server adds `content-length` to JSON responses and
//!   `cache-control` / `connection` to event streams; the mock does the same.
//!   The length is the byte length of the compact JSON, which is what the test
//!   server sends for the same body.
//! - `vi.mock('./version')` pins the user agent's version to `0.0.0-test`. The
//!   Rust provider sends its pinned `4.0.52`, so the check uses that version.
//! - The TypeScript test server answers only `CHAT_COMPLETIONS_URL`; the mock
//!   answers every request, so the test checks the URL of the one request.
//! - The TypeScript test server does not list `user-agent` in `requestHeaders`;
//!   the Rust mock records it, so the header comparison drops it and checks it
//!   on its own.
//! - Timestamps come from the fixtures' `created` fields, so no test date
//!   provider is needed.
//! - `request.body` is a JSON string upstream; here it is the body value, and
//!   its compact text keeps the key order the request was built in
//!   (`preserve_order`). The snapshot files hold that string parsed.
//! - Dates printed bare in snapshots are their `toISOString()` strings.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::groq::{GroqChatLanguageModel, GroqProviderSettings, create_groq};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, ResponseFormat, Tool};
use aimux_core::result::StreamResult;
use aimux_core::shared::provider_namespace;
use aimux_core::tool::FunctionTool;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

use super::support::*;

const MODEL: &str = "gemma2-9b-it";
const CHAT_COMPLETIONS_URL: &str = "https://api.groq.com/openai/v1/chat/completions";
const TEXT_FIXTURE: &str = include_str!("fixtures/groq-text.json");
const TOOL_CALL_FIXTURE: &str = include_str!("fixtures/groq-tool-call.json");
const REASONING_FIXTURE: &str = include_str!("fixtures/groq-reasoning.json");
const TEXT_CHUNKS: &str = include_str!("fixtures/groq-text.chunks.txt");
const TOOL_CALL_CHUNKS: &str = include_str!("fixtures/groq-tool-call.chunks.txt");
const REASONING_CHUNKS: &str = include_str!("fixtures/groq-reasoning.chunks.txt");
/// The last chunk of the tool-call streaming tests: finish reason and `x_groq` usage.
const X_GROQ_TOOL_CALLS_FINISH: &str = r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"gemma2-9b-it","system_fingerprint":"fp_10c08bf97d","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"tool_calls"}],"x_groq":{"id":"req_01jadadp0femyae9kav1gpkhe8","usage":{"queue_time":0.061348671,"prompt_tokens":18,"prompt_time":0.000211569,"completion_tokens":439,"completion_time":0.798181818,"total_tokens":457,"total_time":0.798393387}}}"#;

const SNAPSHOT_DO_GENERATE_TEXT: &str = include_str!("snapshots/do_generate_text.json");
const SNAPSHOT_DO_GENERATE_TOOL_CALL: &str = include_str!("snapshots/do_generate_tool_call.json");
const SNAPSHOT_DO_GENERATE_REASONING: &str = include_str!("snapshots/do_generate_reasoning.json");
const SNAPSHOT_DO_STREAM_TEXT: &str = include_str!("snapshots/do_stream_text.json");
const SNAPSHOT_DO_STREAM_TOOL_CALL: &str = include_str!("snapshots/do_stream_tool_call.json");
const SNAPSHOT_DO_STREAM_REASONING: &str = include_str!("snapshots/do_stream_reasoning.json");

// ── helpers ─────────────────────────────────────────────────────────────────

fn json_value(source: &str) -> Value {
    serde_json::from_str(source).expect("fixture or snapshot is JSON")
}

/// `createGroq({ apiKey, headers })(modelId)` with a mock transport.
fn groq_chat(
    fetch: &Arc<MockFetch>,
    model_id: &str,
    headers: Option<HashMap<String, Option<String>>>,
) -> GroqChatLanguageModel {
    create_groq(GroqProviderSettings {
        api_key: Some("test-api-key".into()),
        headers,
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .expect("groq provider should build")
    .chat(model_id)
}

/// `provider('gemma2-9b-it')`.
fn model(fetch: &Arc<MockFetch>) -> GroqChatLanguageModel {
    groq_chat(fetch, MODEL, None)
}

/// `prompt: TEST_PROMPT`.
fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// The prompt with `providerOptions: { groq: ... }`.
fn options_with_groq(groq: Value) -> CallOptions {
    let mut opts = options();
    opts.provider_options = Some(provider_namespace("groq", groq).unwrap());
    opts
}

/// `{ type: 'object', properties: { value: { type: 'string' } }, required: ['value'],
/// additionalProperties: false, $schema }`.
fn value_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"value": {"type": "string"}},
        "required": ["value"],
        "additionalProperties": false,
        "$schema": "http://json-schema.org/draft-07/schema#"
    })
}

/// The `searchGoogle` function tool of the duplicate-tool-call test.
fn search_google_tool() -> Tool {
    Tool::Function(FunctionTool::new(
        "searchGoogle",
        json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
    ))
}

/// A non-streaming response with the headers the TypeScript test server sends.
fn json_response(body: &Value, extra_headers: &[(&str, &str)]) -> Canned {
    let bytes = serde_json::to_vec(body).expect("serializable");
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), bytes.len().to_string()),
    ];
    headers.extend(
        extra_headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    Canned {
        status: 200,
        headers,
        body: bytes,
    }
}

/// A single-response mock for a JSON body.
fn json_fetch(body: &Value) -> Arc<MockFetch> {
    MockFetch::new(vec![json_response(body, &[])])
}

/// An event-stream response: one `data:` frame per payload, then `[DONE]`.
fn sse_response(payloads: &[&str], extra_headers: &[(&str, &str)]) -> Canned {
    let mut body: String = payloads
        .iter()
        .map(|payload| format!("data: {payload}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    let mut headers = vec![
        ("content-type".to_string(), "text/event-stream".to_string()),
        ("cache-control".to_string(), "no-cache".to_string()),
        ("connection".to_string(), "keep-alive".to_string()),
    ];
    headers.extend(
        extra_headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    Canned {
        status: 200,
        headers,
        body: body.into_bytes(),
    }
}

/// The non-empty lines of a `.chunks.txt` fixture, one payload each.
fn fixture_payloads(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// A single event-stream mock from a chunks fixture.
fn chunks_fetch(source: &str, extra_headers: &[(&str, &str)]) -> Arc<MockFetch> {
    MockFetch::new(vec![sse_response(&fixture_payloads(source), extra_headers)])
}

/// The one request the model sent; it must have gone to the chat-completions URL.
fn sent_request(fetch: &MockFetch) -> Seen {
    let mut seen = fetch.seen();
    assert_eq!(seen.len(), 1, "exactly one request");
    let request = seen.remove(0);
    assert_eq!(request.url, CHAT_COMPLETIONS_URL);
    request
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// The request headers the TypeScript test checks with `toMatchInlineSnapshot`,
/// and the user agent it checks with `toContain`.
fn assert_sent_headers(fetch: &MockFetch, expected: &[(&str, &str)]) {
    let mut headers = sent_request(fetch).headers;
    let user_agent = headers.remove("user-agent").expect("user-agent is sent");
    assert_eq!(headers, map(expected));
    assert!(
        user_agent.contains("ai-sdk-groq/4.0.52"),
        "user-agent was {user_agent}"
    );
}

/// The provider headers of the `headers` tests.
fn provider_headers() -> HashMap<String, Option<String>> {
    HashMap::from([(
        "Custom-Provider-Header".to_string(),
        Some("provider-header-value".to_string()),
    )])
}

/// The request headers of the `headers` tests.
fn request_headers() -> HashMap<String, String> {
    HashMap::from([(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    )])
}

/// The `test-tool` function tool of the tool-call streaming tests.
fn test_tool() -> Tool {
    Tool::Function(FunctionTool::new("test-tool", value_schema()))
}

/// The usage of the `x_groq` chunk in the tool-call streaming tests.
fn x_groq_tool_call_usage() -> Value {
    json!({
        "inputTokens": {"noCache": 18, "total": 18},
        "outputTokens": {"text": 439, "total": 439},
        "raw": {"completion_tokens": 439, "prompt_tokens": 18, "total_tokens": 457}
    })
}

/// All parts of a stream, each serialized as the AI SDK writes it.
async fn stream_parts(result: StreamResult) -> Vec<Value> {
    result
        .stream
        .map(|part| serde_json::to_value(part.expect("stream part")).unwrap())
        .collect()
        .await
}

// ── doGenerate ──────────────────────────────────────────────────────────────

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should send additional response information
    #[tokio::test]
    async fn should_send_additional_response_information() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let result = model(&fetch).do_generate(&options()).await.unwrap();
        let response = result.response.unwrap();
        assert_eq!(
            json!({
                "id": response.id,
                "timestamp": response.timestamp.map(|timestamp| timestamp.to_string()),
                "modelId": response.model_id,
            }),
            json!({
                "id": "chatcmpl-09d64d2a-ed1c-4473-829f-78db43f45d13",
                "modelId": "llama-3.3-70b-versatile",
                "timestamp": "2026-02-11T00:46:38.000Z",
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let fetch = MockFetch::new(vec![json_response(
            &json_value(TEXT_FIXTURE),
            &[("test-header", "test-value")],
        )]);
        let result = model(&fetch).do_generate(&options()).await.unwrap();
        assert_eq!(
            serde_json::to_value(result.response.unwrap().headers).unwrap(),
            json!({
                "content-length": "3547",
                "content-type": "application/json",
                "test-header": "test-value"
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options();
        opts.headers = Some(request_headers());
        groq_chat(&fetch, MODEL, Some(provider_headers()))
            .do_generate(&opts)
            .await
            .unwrap();
        assert_sent_headers(
            &fetch,
            &[
                ("authorization", "Bearer test-api-key"),
                ("content-type", "application/json"),
                ("custom-provider-header", "provider-header-value"),
                ("custom-request-header", "request-header-value"),
            ],
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should use json_schema format when structuredOutputs explicitly enabled
    #[tokio::test]
    async fn should_use_json_schema_format_when_structured_outputs_explicitly_enabled() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options_with_groq(json!({"structuredOutputs": true}));
        opts.response_format = Some(ResponseFormat::Json {
            schema: Some(value_schema()),
            name: Some("test-name".into()),
            description: Some("test description".into()),
        });
        model(&fetch).do_generate(&opts).await.unwrap();
        assert_eq!(
            sent_request(&fetch).json_body(),
            json!({
                "messages": [{"content": "Hello", "role": "user"}],
                "model": "gemma2-9b-it",
                "response_format": {
                    "json_schema": {
                        "description": "test description",
                        "name": "test-name",
                        "schema": value_schema(),
                        "strict": true
                    },
                    "type": "json_schema"
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should allow explicit structuredOutputs override
    #[tokio::test]
    async fn should_allow_explicit_structured_outputs_override() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options_with_groq(json!({"structuredOutputs": true}));
        opts.response_format = Some(ResponseFormat::Json {
            schema: Some(value_schema()),
            name: None,
            description: None,
        });
        model(&fetch).do_generate(&opts).await.unwrap();
        assert_eq!(
            sent_request(&fetch).json_body(),
            json!({
                "messages": [{"content": "Hello", "role": "user"}],
                "model": "gemma2-9b-it",
                "response_format": {
                    "json_schema": {
                        "name": "response",
                        "schema": value_schema(),
                        "strict": true
                    },
                    "type": "json_schema"
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should handle structured outputs with Kimi K2 model
    #[tokio::test]
    async fn should_handle_structured_outputs_with_kimi_k2_model() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let kimi = groq_chat(&fetch, "moonshotai/kimi-k2-instruct-0905", None);
        let mut opts = options_with_groq(json!({"structuredOutputs": true}));
        opts.prompt = vec![LanguageModelMessage::user_text(
            "Generate a simple pasta recipe",
        )];
        opts.response_format = Some(ResponseFormat::Json {
            schema: Some(json!({
                "type": "object",
                "properties": {
                    "recipe": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "ingredients": {"type": "array", "items": {"type": "string"}},
                            "instructions": {"type": "array", "items": {"type": "string"}}
                        },
                        "required": ["name", "ingredients", "instructions"]
                    }
                },
                "required": ["recipe"],
                "additionalProperties": false,
                "$schema": "http://json-schema.org/draft-07/schema#"
            })),
            name: Some("recipe_response".into()),
            description: Some("A recipe with ingredients and instructions".into()),
        });
        kimi.do_generate(&opts).await.unwrap();
        assert_eq!(
            sent_request(&fetch).json_body(),
            json!({
                "messages": [{"content": "Generate a simple pasta recipe", "role": "user"}],
                "model": "moonshotai/kimi-k2-instruct-0905",
                "response_format": {
                    "json_schema": {
                        "description": "A recipe with ingredients and instructions",
                        "name": "recipe_response",
                        "schema": {
                            "$schema": "http://json-schema.org/draft-07/schema#",
                            "additionalProperties": false,
                            "properties": {
                                "recipe": {
                                    "properties": {
                                        "ingredients": {
                                            "items": {"type": "string"},
                                            "type": "array"
                                        },
                                        "instructions": {
                                            "items": {"type": "string"},
                                            "type": "array"
                                        },
                                        "name": {"type": "string"}
                                    },
                                    "required": ["name", "ingredients", "instructions"],
                                    "type": "object"
                                }
                            },
                            "required": ["recipe"],
                            "type": "object"
                        },
                        "strict": true
                    },
                    "type": "json_schema"
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should include warnings when structured outputs explicitly disabled but schema provided
    #[tokio::test]
    async fn should_include_warnings_when_structured_outputs_explicitly_disabled_but_schema_provided()
     {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options_with_groq(json!({"structuredOutputs": false}));
        opts.response_format = Some(ResponseFormat::Json {
            schema: Some(value_schema()),
            name: None,
            description: None,
        });
        let result = model(&fetch).do_generate(&opts).await.unwrap();
        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([{
                "details": "JSON response format schema is only supported with structuredOutputs",
                "feature": "responseFormat",
                "type": "unsupported"
            }])
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should reject a response without choices
    #[tokio::test]
    async fn rejects_a_response_without_choices() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-empty",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [],
                "usage": {"prompt_tokens": 4, "total_tokens": 4, "completion_tokens": 0}
            }),
        )
        .await;
        let model = make_provider(&server);

        let error = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, aimux_core::AiMuxError::InvalidResponseData { message: m, .. } if m == "Response did not contain any choices."),
            "{error:?}"
        );
    }

    mod text {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › text › should extract text content
        #[tokio::test]
        async fn should_extract_text_content() {
            let fetch = json_fetch(&json_value(TEXT_FIXTURE));
            let result = model(&fetch).do_generate(&options()).await.unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_TEXT)
            );
        }

        /// partial: `should extract text content`; full port in src/groq/tests/chat_language_model.rs.
        #[tokio::test]
        async fn extracts_text_content() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            let text = result.content.iter().find_map(|c| match c {
                GenerateContent::Text { text, .. } => Some(text.clone()),
                _ => None,
            });
            assert!(text.is_some());
            assert!(text.unwrap().contains("Luminaria"));
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › text › should send correct request body
        #[tokio::test]
        async fn correct_request_body() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it"
                })
            );
        }
    }

    mod tool_call {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › tool call › should extract tool call content
        #[tokio::test]
        async fn should_extract_tool_call_content() {
            let fetch = json_fetch(&json_value(TOOL_CALL_FIXTURE));
            let result = model(&fetch).do_generate(&options()).await.unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_TOOL_CALL)
            );
        }

        /// partial: `should extract tool call content`; full port in src/groq/tests/chat_language_model.rs.
        #[tokio::test]
        async fn extracts_tool_call() {
            let server = MockServer::start().await;
            mock_json(&server, groq_tool_call_body()).await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            let tool_call = result.content.iter().find_map(|c| match c {
                GenerateContent::ToolCall(RawToolCall {
                    tool_call_id,
                    tool_name,
                    ..
                }) => Some((tool_call_id.clone(), tool_name.clone())),
                _ => None,
            });
            let (id, name) = tool_call.expect("should have tool call");
            assert_eq!(id, "ax9fskhev");
            assert_eq!(name, "weather");
            assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
        }
    }

    mod reasoning {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › reasoning › should extract reasoning content
        #[tokio::test]
        async fn should_extract_reasoning_content() {
            let fetch = json_fetch(&json_value(REASONING_FIXTURE));
            let result = model(&fetch).do_generate(&options()).await.unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_REASONING)
            );
        }

        /// partial: `should extract reasoning content`; full port in src/groq/tests/chat_language_model.rs.
        #[tokio::test]
        async fn extracts_reasoning() {
            let server = MockServer::start().await;
            mock_json(&server, groq_reasoning_body()).await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            let reasoning = result.content.iter().find_map(|c| match c {
                GenerateContent::Reasoning(ReasoningOutput { text, .. }) => Some(text.clone()),
                _ => None,
            });
            assert!(reasoning.is_some());
            assert!(reasoning.unwrap().contains("strawberry"));
        }
    }

    mod top_level_reasoning {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should map top-level reasoning to reasoning_effort
        #[tokio::test]
        async fn reasoning_effort_high() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let options = CallOptions {
                reasoning: Some(ReasoningEffort::High),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["reasoning_effort"], "high");
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should coerce top-level reasoning minimal to low
        #[tokio::test]
        async fn reasoning_effort_minimal_coerced_to_low() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let options = CallOptions {
                reasoning: Some(ReasoningEffort::Minimal),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["reasoning_effort"], "low");
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should coerce top-level reasoning xhigh to high
        #[tokio::test]
        async fn reasoning_effort_xhigh_coerced_to_high() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let options = CallOptions {
                reasoning: Some(ReasoningEffort::Xhigh),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["reasoning_effort"], "high");
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should map top-level reasoning none to reasoning_effort for Qwen 3.6
        #[tokio::test]
        async fn reasoning_none_maps_for_qwen() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = model_at(&server, "qwen/qwen3.6-27b");

            let options = CallOptions {
                reasoning: Some(ReasoningEffort::None),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["reasoning_effort"], "none");
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should omit unsupported top-level reasoning none and warn
        #[tokio::test]
        async fn reasoning_none_omitted_and_warns_for_other_models() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = model_at(&server, browser_search_model_ids()[1]);

            let options = CallOptions {
                reasoning: Some(ReasoningEffort::None),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert!(body.get("reasoning_effort").is_none());
            assert!(result.warnings.iter().any(|w| matches!(
                w,
                aimux_core::types::Warning::Unsupported { feature, details }
                    if feature == "reasoning"
                        && details.as_deref()
                            == Some("reasoning \"none\" is not supported by this model.")
            )));
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should prefer providerOptions reasoningEffort over top-level reasoning
        #[tokio::test]
        async fn provider_option_reasoning_effort_preferred() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = model_at(&server, "openai/gpt-oss-120b");

            let provider_opts =
                provider_namespace("groq", json!({"reasoningEffort": "high"})).unwrap();
            let options = CallOptions {
                reasoning: Some(ReasoningEffort::Medium),
                provider_options: Some(provider_opts),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["reasoning_effort"], "high");
            assert!(result.warnings.is_empty());
        }
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should extract usage
    #[tokio::test]
    async fn extracts_usage() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 45, "noCache": 45 },
                "outputTokens": { "total": 607, "text": 607 },
                "raw": { "completion_tokens": 607, "prompt_tokens": 45, "total_tokens": 652 }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should support partial usage
    #[tokio::test]
    async fn partial_usage() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 20, "total_tokens": 20 }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 20, "noCache": 20 },
                "outputTokens": { "total": 0, "text": 0 },
                "raw": { "prompt_tokens": 20, "total_tokens": 20 }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should extract cached input tokens
    #[tokio::test]
    async fn cached_input_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "total_tokens": 25,
                    "completion_tokens": 5,
                    "prompt_tokens_details": { "cached_tokens": 15 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 20, "noCache": 5, "cacheRead": 15 },
                "outputTokens": { "total": 5, "text": 5 },
                "raw": {
                    "completion_tokens": 5,
                    "prompt_tokens": 20,
                    "prompt_tokens_details": { "cached_tokens": 15 },
                    "total_tokens": 25
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should extract reasoning tokens from completion_tokens_details
    #[tokio::test]
    async fn reasoning_tokens_from_details() {
        let server = MockServer::start().await;
        mock_json(&server, groq_reasoning_body()).await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 17, "noCache": 17 },
                "outputTokens": { "total": 649, "text": 79, "reasoning": 570 },
                "raw": {
                    "completion_tokens": 649,
                    "completion_tokens_details": { "reasoning_tokens": 570 },
                    "prompt_tokens": 17,
                    "total_tokens": 666
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should support unknown finish reason
    #[tokio::test]
    async fn unknown_finish_reason() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "eos"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("eos"));
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass provider options
    #[tokio::test]
    async fn pass_provider_options() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let provider_opts = provider_namespace(
            "groq",
            json!({
                "reasoningFormat": "hidden",
                "user": "test-user-id",
                "parallelToolCalls": false
            }),
        )
        .unwrap();
        let options = CallOptions {
            provider_options: Some(provider_opts),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body,
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gemma2-9b-it",
                "parallel_tool_calls": false,
                "reasoning_format": "hidden",
                "user": "test-user-id"
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass serviceTier provider option
    #[tokio::test]
    async fn pass_service_tier_flex() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let provider_opts = provider_namespace("groq", json!({"serviceTier": "flex"})).unwrap();
        let options = CallOptions {
            provider_options: Some(provider_opts),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body,
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gemma2-9b-it",
                "service_tier": "flex"
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass performance serviceTier provider option
    #[tokio::test]
    async fn pass_service_tier_performance() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let provider_opts =
            provider_namespace("groq", json!({"serviceTier": "performance"})).unwrap();
        let options = CallOptions {
            provider_options: Some(provider_opts),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body,
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gemma2-9b-it",
                "service_tier": "performance"
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass tools and toolChoice
    #[tokio::test]
    async fn pass_tools_and_tool_choice() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let tool = FunctionTool::new(
            "test-tool",
            json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"],
                "additionalProperties": false,
                "$schema": "http://json-schema.org/draft-07/schema#"
            }),
        );
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: Some(LanguageModelV4ToolChoice::Tool {
                tool_name: "test-tool".to_string(),
            }),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body,
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gemma2-9b-it",
                "tool_choice": { "function": { "name": "test-tool" }, "type": "function" },
                "tools": [{
                    "function": {
                        "name": "test-tool",
                        "parameters": {
                            "$schema": "http://json-schema.org/draft-07/schema#",
                            "additionalProperties": false,
                            "properties": { "value": { "type": "string" } },
                            "required": ["value"],
                            "type": "object"
                        }
                    },
                    "type": "function"
                }]
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass response format information as json_schema when structuredOutputs enabled by default
    #[tokio::test]
    async fn response_format_json_schema_default() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let options = CallOptions {
            response_format: Some(ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#"
                })),
                name: Some("test-name".to_string()),
                description: Some("test description".to_string()),
            }),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body,
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gemma2-9b-it",
                "response_format": {
                    "json_schema": {
                        "description": "test description",
                        "name": "test-name",
                        "schema": {
                            "$schema": "http://json-schema.org/draft-07/schema#",
                            "additionalProperties": false,
                            "properties": { "value": { "type": "string" } },
                            "required": ["value"],
                            "type": "object"
                        },
                        "strict": true
                    },
                    "type": "json_schema"
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass response format information as json_object when structuredOutputs explicitly disabled
    #[tokio::test]
    async fn response_format_json_object_when_disabled() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let provider_opts =
            provider_namespace("groq", json!({"structuredOutputs": false})).unwrap();
        let options = CallOptions {
            provider_options: Some(provider_opts),
            response_format: Some(ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#"
                })),
                name: Some("test-name".to_string()),
                description: Some("test description".to_string()),
            }),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body,
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gemma2-9b-it",
                "response_format": { "type": "json_object" }
            })
        );
        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([{
                "details": "JSON response format schema is only supported with structuredOutputs",
                "feature": "responseFormat",
                "type": "unsupported"
            }])
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should send strict: false when strictJsonSchema is explicitly disabled
    #[tokio::test]
    async fn strict_json_schema_false() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let provider_opts = provider_namespace("groq", json!({"strictJsonSchema": false})).unwrap();
        let options = CallOptions {
            provider_options: Some(provider_opts),
            response_format: Some(ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#"
                })),
                name: Some("test-name".to_string()),
                description: Some("test description".to_string()),
            }),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(
            body,
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gemma2-9b-it",
                "response_format": {
                    "json_schema": {
                        "description": "test description",
                        "name": "test-name",
                        "schema": {
                            "$schema": "http://json-schema.org/draft-07/schema#",
                            "additionalProperties": false,
                            "properties": { "value": { "type": "string" } },
                            "required": ["value"],
                            "type": "object"
                        },
                        "strict": false
                    },
                    "type": "json_schema"
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should send request body
    #[tokio::test]
    async fn request_body_string() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let request_body = result
            .request
            .and_then(|request| request.body)
            .expect("should have request body");
        assert_eq!(
            serde_json::to_string(&request_body).unwrap(),
            r#"{"model":"gemma2-9b-it","messages":[{"role":"user","content":"Hello"}]}"#
        );
    }
}

// ── doStream ────────────────────────────────────────────────────────────────

mod do_stream {
    use super::*;

    mod tool_call {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › tool call › should stream tool call
        #[tokio::test]
        async fn should_stream_tool_call() {
            let fetch = chunks_fetch(TOOL_CALL_CHUNKS, &[]);
            let result = model(&fetch).do_stream(&options()).await.unwrap();
            assert_eq!(
                Value::Array(stream_parts(result).await),
                json_value(SNAPSHOT_DO_STREAM_TOOL_CALL)
            );
        }
    }

    mod reasoning {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › reasoning › should stream reasoning
        #[tokio::test]
        async fn should_stream_reasoning() {
            let fetch = chunks_fetch(REASONING_CHUNKS, &[]);
            let result = model(&fetch).do_stream(&options()).await.unwrap();
            assert_eq!(
                Value::Array(stream_parts(result).await),
                json_value(SNAPSHOT_DO_STREAM_REASONING)
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › reasoning › should keep reasoning active when deltas include empty tool calls
            #[tokio::test]
            #[rustfmt::skip]
            async fn keeps_reasoning_active_with_empty_tool_calls() {
                let server = MockServer::start().await;
                let body = sse_body(&[
                    &sse_event(
                        r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning":"Think ","tool_calls":[]},"finish_reason":null}]}"#,
                    ),
                    &sse_event(
                        r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"content":"","reasoning":"more...","tool_calls":[]},"finish_reason":null}]}"#,
                    ),
                    &sse_event(
                        r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"content":"Hello","reasoning":"","tool_calls":[]},"finish_reason":"stop"}]}"#,
                    ),
                ]);
                mock_sse(&server, body).await;
                let model = make_provider(&server);

                let result = model
                    .do_stream(&default_options(test_prompt()))
                    .await
                    .unwrap();
                let parts = collect_stream(result).await;
                let reasoning: Vec<Value> = parts
                    .iter()
                    .map(|part| serde_json::to_value(part).unwrap())
                    .filter(|part| part["type"].as_str().is_some_and(|kind| kind.starts_with("reasoning-")))
                    .collect();
                assert_eq!(
                    reasoning,
                    [
                        json!({ "type": "reasoning-start", "id": "reasoning-0" }),
                        json!({ "type": "reasoning-delta", "id": "reasoning-0", "delta": "Think " }),
                        json!({ "type": "reasoning-delta", "id": "reasoning-0", "delta": "more..." }),
                        json!({ "type": "reasoning-end", "id": "reasoning-0" }),
                    ]
                );
            }
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › should not duplicate tool calls when there is an additional empty chunk after the tool call has been completed
    #[tokio::test]
    async fn should_not_duplicate_tool_calls_when_there_is_an_additional_empty_chunk_after_the_tool_call_has_been_completed()
     {
        let fetch = MockFetch::new(vec![sse_response(
            &[
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"role":"assistant","content":""},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":226,"completion_tokens":0}}"#,
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"id":"chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa","type":"function","index":0,"function":{"name":"searchGoogle"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":233,"completion_tokens":7}}"#,
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\": \""}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":241,"completion_tokens":15}}"#,
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"latest"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":242,"completion_tokens":16}}"#,
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" news"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":243,"completion_tokens":17}}"#,
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" on"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":244,"completion_tokens":18}}"#,
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" ai\"}"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":245,"completion_tokens":19}}"#,
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"logprobs":null,"finish_reason":"tool_calls","stop_reason":128008}],"usage":{"prompt_tokens":226,"total_tokens":246,"completion_tokens":20}}"#,
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[],"usage":{"prompt_tokens":226,"total_tokens":246,"completion_tokens":20}}"#,
            ],
            &[],
        )]);
        let mut opts = options();
        opts.tools = Some(vec![search_google_tool()]);
        let result = model(&fetch).do_stream(&opts).await.unwrap();
        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {"type": "stream-start", "warnings": []},
                {
                    "id": "chat-2267f7e2910a4254bac0650ba74cfc1c",
                    "modelId": "meta/llama-3.1-8b-instruct:fp8",
                    "timestamp": "2024-12-02T17:57:21.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "toolName": "searchGoogle",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"query\": \"",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "latest",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": " news",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": " on",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": " ai\"}",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"query\": \"latest news on ai\"}",
                    "toolCallId": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "toolName": "searchGoogle",
                    "type": "tool-call"
                },
                {
                    "finishReason": {"raw": "tool_calls", "unified": "tool-calls"},
                    "type": "finish",
                    "usage": {"inputTokens": {}, "outputTokens": {}}
                }
            ])
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › should stream tool call deltas when tool call arguments are passed in the first chunk
    #[tokio::test]
    async fn should_stream_tool_call_deltas_when_tool_call_arguments_are_passed_in_the_first_chunk()
    {
        let deltas = [
            "{\\\"",
            "va",
            "lue",
            "\\\":\\\"",
            "Spark",
            "le",
            " Day",
            "\\\"}",
        ];
        let mut payloads: Vec<String> = deltas
            .iter()
            .enumerate()
            .map(|(index, delta)| {
                let delta = if index == 0 {
                    format!(
                        r#"{{"role":"assistant","content":null,"tool_calls":[{{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{{"name":"test-tool","arguments":"{delta}"}}}}]}}"#
                    )
                } else {
                    format!(r#"{{"tool_calls":[{{"index":0,"function":{{"arguments":"{delta}"}}}}]}}"#)
                };
                format!(
                    r#"{{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"gemma2-9b-it","system_fingerprint":"fp_3bc1b5746c","choices":[{{"index":0,"delta":{delta},"finish_reason":null}}]}}"#
                )
            })
            .collect();
        payloads.push(X_GROQ_TOOL_CALLS_FINISH.to_string());
        let payloads: Vec<&str> = payloads.iter().map(String::as_str).collect();
        let fetch = MockFetch::new(vec![sse_response(&payloads, &[])]);
        let mut opts = options();
        opts.tools = Some(vec![test_tool()]);
        let result = model(&fetch).do_stream(&opts).await.unwrap();

        let id = "call_O17Uplv4lJvD6DVdIvFFeRMw";
        let mut expected = vec![
            json!({"type": "stream-start", "warnings": []}),
            json!({
                "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                "modelId": "gemma2-9b-it",
                "timestamp": "2024-03-25T09:06:38.000Z",
                "type": "response-metadata"
            }),
            json!({"id": id, "toolName": "test-tool", "type": "tool-input-start"}),
        ];
        for delta in ["{\"", "va", "lue", "\":\"", "Spark", "le", " Day", "\"}"] {
            expected.push(json!({"delta": delta, "id": id, "type": "tool-input-delta"}));
        }
        expected.extend([
            json!({"id": id, "type": "tool-input-end"}),
            json!({
                "input": "{\"value\":\"Sparkle Day\"}",
                "toolCallId": id,
                "toolName": "test-tool",
                "type": "tool-call"
            }),
            json!({
                "finishReason": {"raw": "tool_calls", "unified": "tool-calls"},
                "type": "finish",
                "usage": x_groq_tool_call_usage()
            }),
        ]);
        assert_eq!(stream_parts(result).await, expected);
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › should stream tool call that is sent in one chunk
    #[tokio::test]
    async fn should_stream_tool_call_that_is_sent_in_one_chunk() {
        let fetch = MockFetch::new(vec![sse_response(
            &[
                r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"gemma2-9b-it","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":"{\"value\":\"Sparkle Day\"}"}}]},"finish_reason":null}]}"#,
                X_GROQ_TOOL_CALLS_FINISH,
            ],
            &[],
        )]);
        let mut opts = options();
        opts.tools = Some(vec![test_tool()]);
        let result = model(&fetch).do_stream(&opts).await.unwrap();
        let id = "call_O17Uplv4lJvD6DVdIvFFeRMw";
        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {"type": "stream-start", "warnings": []},
                {
                    "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                    "modelId": "gemma2-9b-it",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {"id": id, "toolName": "test-tool", "type": "tool-input-start"},
                {"delta": "{\"value\":\"Sparkle Day\"}", "id": id, "type": "tool-input-delta"},
                {"id": id, "type": "tool-input-end"},
                {
                    "input": "{\"value\":\"Sparkle Day\"}",
                    "toolCallId": id,
                    "toolName": "test-tool",
                    "type": "tool-call"
                },
                {
                    "finishReason": {"raw": "tool_calls", "unified": "tool-calls"},
                    "type": "finish",
                    "usage": x_groq_tool_call_usage()
                }
            ])
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let fetch = chunks_fetch(TEXT_CHUNKS, &[("test-header", "test-value")]);
        let result = model(&fetch).do_stream(&options()).await.unwrap();
        assert_eq!(
            serde_json::to_value(result.response.unwrap().headers).unwrap(),
            json!({
                "cache-control": "no-cache",
                "connection": "keep-alive",
                "content-type": "text/event-stream",
                "test-header": "test-value"
            })
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let fetch = chunks_fetch(TEXT_CHUNKS, &[]);
        let mut opts = options();
        opts.headers = Some(request_headers());
        groq_chat(&fetch, MODEL, Some(provider_headers()))
            .do_stream(&opts)
            .await
            .unwrap();
        assert_sent_headers(
            &fetch,
            &[
                ("authorization", "Bearer test-api-key"),
                ("content-type", "application/json"),
                ("custom-provider-header", "provider-header-value"),
                ("custom-request-header", "request-header-value"),
            ],
        );
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let fetch = chunks_fetch(TEXT_CHUNKS, &[]);
        let result = model(&fetch).do_stream(&options()).await.unwrap();
        let body = result.request.unwrap().body.unwrap();
        assert_eq!(
            body.to_string(),
            r#"{"model":"gemma2-9b-it","messages":[{"role":"user","content":"Hello"}],"stream":true}"#
        );
    }

    mod text {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › text › should stream text
        #[tokio::test]
        async fn should_stream_text() {
            let fetch = chunks_fetch(TEXT_CHUNKS, &[]);
            let result = model(&fetch).do_stream(&options()).await.unwrap();
            assert_eq!(
                Value::Array(stream_parts(result).await),
                json_value(SNAPSHOT_DO_STREAM_TEXT)
            );
        }

        /// partial: `should stream text`; full port in src/groq/tests/chat_language_model.rs.
        #[tokio::test]
        async fn streams_text() {
            let server = MockServer::start().await;
            let body = sse_body(&[
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
                ),
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
                ),
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
                ),
            ]);
            mock_sse(&server, body).await;

            let model = make_provider(&server);

            let result = model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap();
            let parts = collect_stream(result).await;

            // Should have stream-start, response-metadata, text-start, text-delta x2, text-end, finish
            let text_deltas: Vec<String> = parts
                .iter()
                .filter_map(|p| match p {
                    StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(text_deltas, vec!["Hello", " world"]);

            // Check finish reason
            let finish = parts.iter().find_map(|p| match p {
                StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
                _ => None,
            });
            assert!(finish.is_some());
            assert_eq!(finish.unwrap().unified, FinishReasonUnified::Stop);
        }
    }

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › should send correct streaming request body
    #[tokio::test]
    async fn streaming_request_body() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            ),
        ]);
        mock_sse(&server, body).await;

        let model = make_provider(&server);

        model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();

        let req_body = first_request_body(&server).await;
        assert_eq!(
            req_body,
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gemma2-9b-it",
                "stream": true
            })
        );
    }

    /// partial: `should stream tool call deltas when tool call arguments are passed in the first chunk`; full port in src/groq/tests/chat_language_model.rs.
    #[tokio::test]
    async fn streams_tool_call_deltas() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"test-tool","arguments":"{"}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"value\":"}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"Sparkle Day\"}"}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"x_groq":{"usage":{"prompt_tokens":18,"completion_tokens":439,"total_tokens":457}}}"#,
            ),
        ]);
        mock_sse(&server, body).await;

        let model = make_provider(&server);

        let tool = FunctionTool::new(
            "test-tool",
            json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"],
                "additionalProperties": false,
            }),
        );
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        // Should have a ToolCall part
        let tool_call = parts.iter().find_map(|p| match p {
            StreamPart::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        });
        let (id, name, input) = tool_call.expect("should have tool call");
        assert_eq!(id, "call_abc");
        assert_eq!(name, "test-tool");
        assert_eq!(input, Value::String(r#"{"value":"Sparkle Day"}"#.into()));

        // Should have tool-calls finish reason
        let finish = parts.iter().find_map(|p| match p {
            StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
            _ => None,
        });
        assert_eq!(finish.unwrap().unified, FinishReasonUnified::ToolCalls);
    }

    /// partial: `should handle error stream parts`; upstream snapshots the JavaScript `APICallError` object, and aimux's error has its own serialized shape, so the test checks its message, status and retryability.
    #[tokio::test]
    async fn handles_error_stream() {
        let server = MockServer::start().await;
        let body = sse_body(&[&sse_event(
            r#"{"error":{"message":"Rate limit reached","type":"rate_limit_error"}}"#,
        )]);
        mock_sse(&server, body).await;

        let model = make_provider(&server);

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
        match &parts[1] {
            StreamPart::Error {
                error: aimux_core::AiMuxError::ApiCall(error),
            } => {
                assert_eq!(error.message, "Rate limit reached");
                assert_eq!(error.status_code, Some(429));
                assert!(error.is_retryable);
            }
            other => panic!("expected Error, got {other:?}"),
        }
        match &parts[2] {
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
                assert_eq!(finish_reason.raw, None);
                assert_eq!(
                    serde_json::to_value(usage).unwrap(),
                    serde_json::to_value(aimux_core::types::Usage::default()).unwrap()
                );
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    /// partial: `should stream tool call that is sent in one chunk`; full port in src/groq/tests/chat_language_model.rs.
    #[tokio::test]
    async fn streams_tool_call_one_chunk() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"test-tool","arguments":"{\"value\":\"Sparkle Day\"}"}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"x_groq":{"usage":{"prompt_tokens":18,"completion_tokens":439,"total_tokens":457}}}"#,
            ),
        ]);
        mock_sse(&server, body).await;

        let model = make_provider(&server);

        let tool = FunctionTool::new(
            "test-tool",
            json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}),
        );
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        let tool_call = parts.iter().find_map(|p| match p {
            StreamPart::ToolCall(RawToolCall { input, .. }) => Some(input.clone()),
            _ => None,
        });
        assert_eq!(
            tool_call.unwrap(),
            Value::String(r#"{"value":"Sparkle Day"}"#.into())
        );
    }

    /// partial: `should handle unparsable stream parts`; upstream snapshots the JavaScript `JSONParseError` and its `SyntaxError` text, which aimux's parse error does not reproduce, so the test checks the part types and the finish part.
    #[tokio::test]
    async fn unparsable_chunk_is_an_error_part_and_finish_reason_error() {
        let server = MockServer::start().await;
        mock_sse(
            &server,
            "data: {unparsable}\n\ndata: [DONE]\n\n".to_string(),
        )
        .await;
        let model = make_provider(&server);

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        assert_eq!(parts.len(), 3, "{parts:?}");
        assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
        assert!(matches!(parts[1], StreamPart::Error { .. }), "{parts:?}");
        match &parts[2] {
            StreamPart::Finish {
                finish_reason,
                usage,
                provider_metadata,
            } => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
                assert_eq!(finish_reason.raw, None);
                assert_eq!(
                    serde_json::to_value(usage).unwrap(),
                    serde_json::to_value(aimux_core::types::Usage::default()).unwrap()
                );
                assert!(provider_metadata.is_none());
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }
}

mod do_stream_with_raw_chunks {
    use super::*;

    /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream with raw chunks › should stream raw chunks when includeRawChunks is true
    #[tokio::test]
    async fn should_stream_raw_chunks_when_include_raw_chunks_is_true() {
        let fetch = MockFetch::new(vec![sse_response(
            &[
                r#"{"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
                r#"{"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
                r#"{"id":"chatcmpl-789","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"x_groq":{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}}"#,
            ],
            &[],
        )]);
        let mut opts = options();
        opts.include_raw_chunks = Some(true);
        let result = model(&fetch).do_stream(&opts).await.unwrap();
        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {"type": "stream-start", "warnings": []},
                {
                    "rawValue": {
                        "choices": [{"delta": {"content": "Hello"}, "finish_reason": null, "index": 0}],
                        "created": 1234567890,
                        "id": "chatcmpl-123",
                        "model": "gemma2-9b-it",
                        "object": "chat.completion.chunk"
                    },
                    "type": "raw"
                },
                {
                    "id": "chatcmpl-123",
                    "modelId": "gemma2-9b-it",
                    "timestamp": "2009-02-13T23:31:30.000Z",
                    "type": "response-metadata"
                },
                {"id": "txt-0", "type": "text-start"},
                {"delta": "Hello", "id": "txt-0", "type": "text-delta"},
                {
                    "rawValue": {
                        "choices": [{"delta": {"content": " world"}, "finish_reason": null, "index": 0}],
                        "created": 1234567890,
                        "id": "chatcmpl-456",
                        "model": "gemma2-9b-it",
                        "object": "chat.completion.chunk"
                    },
                    "type": "raw"
                },
                {"delta": " world", "id": "txt-0", "type": "text-delta"},
                {
                    "rawValue": {
                        "choices": [{"delta": {}, "finish_reason": "stop", "index": 0}],
                        "created": 1234567890,
                        "id": "chatcmpl-789",
                        "model": "gemma2-9b-it",
                        "object": "chat.completion.chunk",
                        "x_groq": {
                            "usage": {"completion_tokens": 5, "prompt_tokens": 10, "total_tokens": 15}
                        }
                    },
                    "type": "raw"
                },
                {"id": "txt-0", "type": "text-end"},
                {
                    "finishReason": {"raw": "stop", "unified": "stop"},
                    "type": "finish",
                    "usage": {
                        "inputTokens": {"noCache": 10, "total": 10},
                        "outputTokens": {"text": 5, "total": 5},
                        "raw": {"completion_tokens": 5, "prompt_tokens": 10, "total_tokens": 15}
                    }
                }
            ])
        );
    }

    /// partial: `should stream raw chunks when includeRawChunks is true`; full port in src/groq/tests/chat_language_model.rs.
    #[tokio::test]
    async fn streams_raw_chunks() {
        let server = MockServer::start().await;
        let chunks = [
            r#"{"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-789","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"x_groq":{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}}"#,
        ];
        mock_sse(
            &server,
            sse_body(
                &chunks
                    .map(sse_event)
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            ),
        )
        .await;
        let model = make_provider(&server);

        let options = CallOptions {
            include_raw_chunks: Some(true),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        let raw: Vec<&Value> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::Raw { raw_value } => Some(raw_value),
                _ => None,
            })
            .collect();
        assert_eq!(raw.len(), 3);
        for (raw, chunk) in raw.iter().zip(chunks) {
            assert_eq!(**raw, serde_json::from_str::<Value>(chunk).unwrap());
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Tests without an upstream case
// ════════════════════════════════════════════════════════════════════════════
/// `groqFailedResponseHandler` (groq@4.0.52 `groq-error.ts:4-16`): the
/// message is `error.message`, and `error.type` is the provider code.
#[tokio::test]
async fn a_failed_response_reads_groqs_error_schema() {
    let error = failed_call(
        400,
        json!({ "error": {
            "message": "The model `x` does not exist",
            "type": "invalid_request_error",
            "code": "model_not_found"
        } }),
    )
    .await;
    assert_eq!(error.message, "The model `x` does not exist");
    assert_eq!(
        error.provider_code.as_deref(),
        Some("invalid_request_error")
    );
    assert_eq!(error.status_code, Some(400));
    assert!(!error.is_retryable);
}

/// A body outside the schema falls back to the status text, as
/// `createJsonErrorResponseHandler` does when the schema does not parse.
#[tokio::test]
async fn a_failed_response_outside_the_schema_uses_the_status_text() {
    let error = failed_call(503, json!({ "message": "top-level only" })).await;
    assert_eq!(error.message, "Service Unavailable");
    assert_eq!(error.provider_code, None);
    assert!(error.is_retryable);
}

/// RFC-0016 M9: warnings computed while building the request body reach
/// `StreamStart` (previously hard-coded empty). Groq does not support
/// `top_k` → Unsupported warning.
#[tokio::test]
async fn stream_start_carries_body_build_warnings() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let model = make_provider(&server);

    let options = CallOptions {
        top_k: Some(0.5),
        ..default_options(test_prompt())
    };
    let result = model.do_stream(&options).await.unwrap();
    let parts = collect_stream(result).await;

    match &parts[0] {
        StreamPart::StreamStart { warnings } => {
            assert!(
                warnings.iter().any(|w| matches!(
                    w,
                    aimux_core::types::Warning::Unsupported { feature, .. }
                        if feature == "topK"
                )),
                "expected topK Unsupported warning in StreamStart, got {warnings:?}"
            );
        }
        other => panic!("expected StreamStart first, got {other:?}"),
    }
}

/// An error chunk after a content chunk becomes an Error part with its status; the stream then ends with an Error finish.
#[tokio::test]
async fn error_chunk_is_an_error_part_with_status() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(r#"{"error":{"message":"Rate limit reached","type":"rate_limit_error"}}"#),
    ]);
    mock_sse(&server, body).await;
    let model = make_provider(&server);

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();
    let parts = collect_stream(result).await;

    let error = parts
        .iter()
        .find_map(|p| match p {
            StreamPart::Error { error } => Some(error),
            _ => None,
        })
        .expect("an error part");
    match error {
        aimux_core::AiMuxError::ApiCall(call) => {
            assert_eq!(call.message, "Rate limit reached");
            assert_eq!(call.status_code, Some(429));
            assert!(error.is_retryable());
        }
        other => panic!("expected ApiCall, got {other:?}"),
    }
    match parts.last().unwrap() {
        StreamPart::Finish { finish_reason, .. } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// Usage streamed in x_groq.usage is reported on the finish part, raw object included.
#[tokio::test]
async fn streams_usage_from_x_groq() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"x_groq":{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let model = make_provider(&server);

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();
    let parts = collect_stream(result).await;

    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { usage, .. } => Some(usage.clone()),
        _ => None,
    });
    let usage = finish.expect("should have finish");
    assert_eq!(usage.input_tokens.total, Some(10));
    assert_eq!(usage.output_tokens.total, Some(5));
    // RFC-0016 M10: the raw object from the x_groq.usage sub-path is
    // preserved verbatim (vendor fields included).
    let raw = usage.raw.as_ref().expect("usage.raw must be populated");
    assert_eq!(raw["prompt_tokens"], json!(10));
    assert_eq!(raw["total_tokens"], json!(15));
}

/// A 401 response maps to an auth error.
#[tokio::test]
async fn auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error": {"message": "Invalid API key"}})),
        )
        .mount(&server)
        .await;

    let model = make_provider(&server);

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(result.is_err());
    match result {
        Err(ref e) if e.status_code() == Some(401) => {}
        Err(e) => panic!("expected a 401, got {e:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

/// The request should carry the Authorization: Bearer header.
#[tokio::test]
async fn authorization_header() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let model = make_provider(&server);

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let auth = requests[0]
        .headers
        .get("authorization")
        .expect("missing authorization header");
    assert_eq!(auth, "Bearer test-api-key");
}

#[test]
fn every_entry_point_reports_groq_chat() {
    assert_eq!(
        groq().chat("llama-3.3-70b-versatile").provider(),
        "groq.chat"
    );
    assert_eq!(groq().call("m").provider(), "groq.chat");
    assert_eq!(groq().language_model("m").unwrap().provider(), "groq.chat");
    // The registry entry point and the preset build the same identity.
    let model = crate::create_provider("groq", crate::PresetSettings::default())
        .unwrap()
        .language_model("m")
        .unwrap();
    assert_eq!(model.provider(), "groq.chat");
}

#[test]
fn embedding_and_image_models_are_no_such_model() {
    let provider = groq();
    for result in [
        provider.embedding_model("e").map(|_| ()),
        provider.image_model("i").map(|_| ()),
    ] {
        assert!(matches!(result, Err(AiMuxError::NoSuchModel { .. })));
    }
}

#[tokio::test]
async fn only_the_groq_namespace_is_read() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;
    let model = groq_at(&server).chat("gemma2-9b-it");

    let result = model
        .do_generate(&options_with(json!({
            "groq": {"user": "u1", "reasoningFormat": "parsed", "serviceTier": "flex",
                     "parallelToolCalls": false},
            "openai": {"user": "ignored"},
            "openaiCompatible": {"reasoningEffort": "low"},
        })))
        .await
        .unwrap();

    let body = result.request.and_then(|r| r.body).unwrap();
    assert_eq!(body["user"], "u1");
    assert_eq!(body["reasoning_format"], "parsed");
    assert_eq!(body["service_tier"], "flex");
    assert_eq!(body["parallel_tool_calls"], json!(false));
    // upstream: parseProviderOptions({ provider: 'groq' }) reads that namespace only
    assert!(body.get("reasoning_effort").is_none());
}

#[tokio::test]
async fn streaming_asks_for_no_stream_options_and_reports_x_groq_usage() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"x_groq":{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cached_tokens":4}}}}"#,
        ),
    ]);
    mock_sse(&server, body).await;
    let model = groq_at(&server).chat("m");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();
    assert!(
        result
            .request
            .as_ref()
            .and_then(|r| r.body.as_ref())
            .unwrap()
            .get("stream_options")
            .is_none()
    );
    let parts = collect_stream(result).await;

    let (usage, metadata) = parts
        .iter()
        .find_map(|p| match p {
            StreamPart::Finish {
                usage,
                provider_metadata,
                ..
            } => Some((usage.clone(), provider_metadata.clone())),
            _ => None,
        })
        .expect("a finish part");
    assert_eq!(usage.input_tokens.total, Some(10));
    assert_eq!(usage.input_tokens.cache_read, Some(4));
    assert_eq!(usage.input_tokens.no_cache, Some(6));
    assert_eq!(usage.output_tokens.total, Some(5));
    // upstream: the finish part carries no providerMetadata
    assert!(metadata.is_none());
}

#[tokio::test]
async fn the_token_limit_is_max_tokens() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;
    let model = groq_at(&server).chat("gemma2-9b-it");
    let mut options = default_options(test_prompt());
    options.max_output_tokens = Some(64);
    options.top_k = Some(40.0);
    let result = model.do_generate(&options).await.unwrap();
    let body = result.request.and_then(|r| r.body).unwrap();
    // upstream: getArgs sends `max_tokens: maxOutputTokens`
    assert_eq!(body["max_tokens"], 64);
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("top_k").is_none(), "Groq has no top_k");
    assert!(
        result
            .warnings
            .iter()
            .any(|w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. } if feature == "topK"))
    );
}

#[serial]
#[tokio::test]
async fn the_key_is_read_from_the_environment_per_request_not_at_creation() {
    let saved = std::env::var("GROQ_API_KEY").ok();
    unsafe { std::env::remove_var("GROQ_API_KEY") };
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;
    let provider = create_groq(GroqProviderSettings {
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .expect("no key is read at creation");
    let model = provider.chat("m");

    let error = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AiMuxError::LoadApiKey { env_var, .. } if env_var == "GROQ_API_KEY"),
        "{error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());

    unsafe { std::env::set_var("GROQ_API_KEY", "env-key") };
    model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer env-key"
    );
    unsafe {
        match saved {
            Some(v) => std::env::set_var("GROQ_API_KEY", v),
            None => std::env::remove_var("GROQ_API_KEY"),
        }
    }
}

/// aimux's own: `usage.raw` keeps a `null` usage field as `null`, as
/// `convertGroqUsage` returns the parsed usage object (convert-groq-usage.ts:53);
/// keys outside the usage schema are dropped (groq-chat-language-model.ts:564-580).
#[tokio::test]
async fn usage_raw_keeps_null_fields() {
    let server = MockServer::start().await;
    let mut body = groq_text_body();
    body["usage"] = json!({
        "prompt_tokens": 45,
        "completion_tokens": 607,
        "total_tokens": 652,
        "prompt_tokens_details": { "cached_tokens": null },
        "completion_tokens_details": null,
        "queue_time": 0.01
    });
    mock_json(&server, body).await;

    let result = make_provider(&server)
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(
        result.usage.raw.map(Value::Object),
        Some(json!({
            "prompt_tokens": 45,
            "completion_tokens": 607,
            "total_tokens": 652,
            "prompt_tokens_details": { "cached_tokens": null },
            "completion_tokens_details": null
        }))
    );
}
