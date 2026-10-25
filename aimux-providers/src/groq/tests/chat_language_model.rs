//! Ports of `src/groq-chat-language-model.test.ts` from `@ai-sdk/groq@4.0.52`:
//! the snapshot, response-metadata, header, structured-output, tool-call,
//! reasoning and raw-chunk cases. `groq_test.rs` has the request-body cases and
//! older, partial versions of some cases here (marked `partial:` there).
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
    }

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
}

// ── doStream ────────────────────────────────────────────────────────────────

mod do_stream {
    use super::*;

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
    }

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
}

// ── doStream with raw chunks ────────────────────────────────────────────────

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
}
