//! Ports of `src/mistral-chat-language-model.test.ts` from `@ai-sdk/mistral@4.0.55`:
//! the `doGenerate`, `doStream`, tool-result and reference-content cases.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `toMatchSnapshot` values are the `.snap` entries in `snapshots/`, with
//!   `undefined` properties absent (as `JSON.stringify` drops them).
//! - The TypeScript test server adds `content-length` to JSON responses and
//!   `cache-control` / `connection` to event streams; the mock does the same.
//! - `vi.mock('./version')` pins the user agent's version to `0.0.0-test`. The
//!   Rust provider sends its pinned `4.0.55`, so the check uses that version.
//! - `mockId()` is one counter shared by the whole upstream file. Each test here
//!   seeds its own generator at the value that shared counter has when the
//!   upstream test runs.
//! - The TypeScript test server does not list `user-agent` in `requestHeaders`;
//!   the Rust mock records it, so the header comparison drops it and checks it
//!   on its own.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, TextPart, ToolCallPart, ToolPart, ToolResultOutput,
    ToolResultPart,
};
use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice};
use aimux_core::result::StreamResult;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{ReasoningEffort, Warning};
use aimux_providers::mistral::{MistralProvider, MistralProviderSettings, create_mistral};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::mock_fetch::{Canned, MockFetch};

const MODEL: &str = "mistral-small-latest";
const TEXT_FIXTURE: &str = include_str!("fixtures/mistral-text.json");
const REASONING_FIXTURE: &str = include_str!("fixtures/mistral-reasoning.json");
const TEXT_CHUNKS: &str = include_str!("fixtures/mistral-text.chunks.txt");
const REASONING_CHUNKS: &str = include_str!("fixtures/mistral-reasoning.chunks.txt");
const TOOL_CALL_CHUNKS: &str = include_str!("fixtures/mistral-tool-call.chunks.txt");
const INCREMENTAL_TOOL_CALL_CHUNKS: &str =
    include_str!("fixtures/mistral-incremental-tool-call.chunks.txt");
const USAGE_DETAILS_CHUNKS: &str = include_str!("fixtures/mistral-usage-details.chunks.txt");

const SNAPSHOT_DO_GENERATE_TEXT: &str = include_str!("snapshots/do_generate_text.json");
const SNAPSHOT_DO_GENERATE_REASONING: &str = include_str!("snapshots/do_generate_reasoning.json");
const SNAPSHOT_DO_STREAM_TOOL_CALL: &str = include_str!("snapshots/do_stream_tool_call.json");
const SNAPSHOT_DO_STREAM_REASONING: &str = include_str!("snapshots/do_stream_reasoning.json");

// ── helpers ─────────────────────────────────────────────────────────────────

fn json_value(source: &str) -> Value {
    serde_json::from_str(source).expect("fixture or snapshot is JSON")
}

/// `createMistral({ apiKey, generateId: mockId() })` with a mock transport.
fn provider(fetch: &Arc<MockFetch>) -> MistralProvider {
    provider_with(fetch, id_generator(0), None)
}

fn provider_with(
    fetch: &Arc<MockFetch>,
    generate_id: Arc<dyn Fn() -> String + Send + Sync>,
    headers: Option<HashMap<String, Option<String>>>,
) -> MistralProvider {
    create_mistral(MistralProviderSettings {
        api_key: Some("test-api-key".into()),
        headers,
        fetch: Some(fetch.transport()),
        generate_id: Some(generate_id),
        ..Default::default()
    })
    .unwrap()
}

/// `mockId()`: `id-0`, `id-1`, ...
fn id_generator(start: usize) -> Arc<dyn Fn() -> String + Send + Sync> {
    let next = AtomicUsize::new(start);
    Arc::new(move || format!("id-{}", next.fetch_add(1, Ordering::SeqCst)))
}

/// `prompt: TEST_PROMPT`.
fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// The `{ model, messages: [user "Hello"] }` body most tests expect.
fn hello_body() -> Value {
    json!({
        "model": MODEL,
        "messages": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}]
    })
}

fn test_tool() -> Tool {
    Tool::Function(FunctionTool::new(
        "test-tool",
        json!({
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
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

/// A single event-stream mock from literal payloads.
fn stream_fetch(payloads: &[&str]) -> Arc<MockFetch> {
    MockFetch::new(vec![sse_response(payloads, &[])])
}

/// A non-streaming chat completion with one choice and `stop`.
fn completion(id: &str, created: u64, message: Value, usage: Value) -> Value {
    json!({
        "object": "chat.completion",
        "id": id,
        "created": created,
        "model": MODEL,
        "choices": [{"index": 0, "message": message, "finish_reason": "stop"}],
        "usage": usage
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

/// `toMatchObject`: every property of `expected` is in `actual`, recursively;
/// arrays have the same length and match element by element.
fn assert_match_object(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(actual_map), Value::Object(expected_map)) => {
            for (key, value) in expected_map {
                let got = actual_map
                    .get(key)
                    .unwrap_or_else(|| panic!("missing key {key} in {actual}"));
                assert_match_object(got, value);
            }
        }
        (Value::Array(actual_items), Value::Array(expected_items)) => {
            assert_eq!(actual_items.len(), expected_items.len(), "in {actual}");
            for (got, want) in actual_items.iter().zip(expected_items) {
                assert_match_object(got, want);
            }
        }
        _ => assert_eq!(actual, expected),
    }
}

fn has_unsupported(warnings: &[Warning], feature: &str) -> bool {
    warnings
        .iter()
        .any(|w| matches!(w, Warning::Unsupported { feature: f, .. } if f.as_str() == feature))
}

fn has_compatibility(warnings: &[Warning], feature: &str) -> bool {
    warnings
        .iter()
        .any(|w| matches!(w, Warning::Compatibility { feature: f, .. } if f.as_str() == feature))
}

fn has_any_feature(warnings: &[Warning], feature: &str) -> bool {
    has_unsupported(warnings, feature) || has_compatibility(warnings, feature)
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// The request headers the TypeScript test checks with `toStrictEqual`, and
/// the user agent it checks with `toContain`.
fn assert_sent_headers(fetch: &MockFetch, expected: &[(&str, &str)]) {
    let seen = fetch.seen();
    let mut headers = seen[0].headers.clone();
    let user_agent = headers.remove("user-agent").expect("user-agent is sent");
    assert_eq!(headers, map(expected));
    assert!(
        user_agent.contains("ai-sdk-mistral/4.0.55"),
        "user-agent was {user_agent}"
    );
}

// ── doGenerate ──────────────────────────────────────────────────────────────

mod do_generate {
    use super::*;

    mod text {
        use super::super::*;

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › text › should extract text content
        #[tokio::test]
        async fn should_extract_text_content() {
            let fetch = json_fetch(&json_value(TEXT_FIXTURE));
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_TEXT)
            );
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › text › should send correct request body
        #[tokio::test]
        async fn should_send_correct_request_body() {
            let fetch = json_fetch(&json_value(TEXT_FIXTURE));
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(fetch.seen()[0].json_body(), hello_body());
        }
    }

    mod reasoning {
        use super::super::*;

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › reasoning › should extract reasoning content
        #[tokio::test]
        async fn should_extract_reasoning_content() {
            let fetch = json_fetch(&json_value(REASONING_FIXTURE));
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_REASONING)
            );
        }
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should pass tools and toolChoice
    #[tokio::test]
    async fn should_pass_tools_and_tool_choice() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options();
        opts.tools = Some(vec![test_tool()]);
        opts.tool_choice = Some(LanguageModelV4ToolChoice::Tool {
            tool_name: "test-tool".into(),
        });
        provider(&fetch)
            .chat(MODEL)
            .do_generate(&opts)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body(),
            json!({
                "model": MODEL,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}],
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "test-tool",
                        "parameters": {
                            "type": "object",
                            "properties": {"value": {"type": "string"}},
                            "required": ["value"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#"
                        }
                    }
                }],
                "tool_choice": "any"
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should forward stopSequences as the Mistral stop parameter and not warn
    #[tokio::test]
    async fn should_forward_stop_sequences_and_not_warn() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options();
        opts.stop_sequences = Some(vec!["foo".into(), "bar".into()]);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&opts)
            .await
            .unwrap();
        let mut expected = hello_body();
        expected["stop"] = json!(["foo", "bar"]);
        assert_eq!(fetch.seen()[0].json_body(), expected);
        assert!(!has_unsupported(&result.warnings, "stopSequences"));
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should forward presencePenalty and frequencyPenalty without unsupported warnings
    #[tokio::test]
    async fn should_forward_presence_and_frequency_penalty_without_unsupported_warnings() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options();
        opts.presence_penalty = Some(0.1);
        opts.frequency_penalty = Some(0.2);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&opts)
            .await
            .unwrap();
        assert!(!has_unsupported(&result.warnings, "presencePenalty"));
        assert!(!has_unsupported(&result.warnings, "frequencyPenalty"));
        assert_match_object(
            &fetch.seen()[0].json_body(),
            &json!({"presence_penalty": 0.1, "frequency_penalty": 0.2}),
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let provider = provider_with(
            &fetch,
            id_generator(0),
            Some(HashMap::from([(
                "Custom-Provider-Header".to_string(),
                Some("provider-header-value".to_string()),
            )])),
        );
        let mut opts = options();
        opts.headers = Some(HashMap::from([(
            "Custom-Request-Header".to_string(),
            "request-header-value".to_string(),
        )]));
        provider.chat(MODEL).do_generate(&opts).await.unwrap();
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

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let fetch = MockFetch::new(vec![json_response(
            &json_value(TEXT_FIXTURE),
            &[("test-header", "test-value")],
        )]);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(result.response.unwrap().headers).unwrap(),
            json!({
                "content-length": "2287",
                "content-type": "application/json",
                "test-header": "test-value"
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should extract usage
    #[tokio::test]
    async fn should_extract_usage() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": {"noCache": 13, "total": 13},
                "outputTokens": {"text": 434, "total": 434},
                "raw": {"completion_tokens": 434, "prompt_tokens": 13, "total_tokens": 447}
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should send additional response information
    #[tokio::test]
    async fn should_send_additional_response_information() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        let response = result.response.unwrap();
        assert_eq!(
            json!({
                "id": response.id,
                "modelId": response.model_id,
                "timestamp": response.timestamp.map(|t| t.to_string()),
            }),
            json!({
                "id": "5319bd0299614c679a0068a4f2c8ffd0",
                "modelId": "mistral-small-latest",
                "timestamp": "2026-01-22T13:32:00.000Z"
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.request).unwrap(),
            json!({"body": hello_body()})
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should pass parallelToolCalls option
    #[tokio::test]
    async fn should_pass_parallel_tool_calls_option() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options();
        opts.tools = Some(vec![test_tool()]);
        opts.provider_options =
            Some(serde_json::from_value(json!({"mistral": {"parallelToolCalls": false}})).unwrap());
        provider(&fetch)
            .chat(MODEL)
            .do_generate(&opts)
            .await
            .unwrap();
        assert_match_object(
            &fetch.seen()[0].json_body(),
            &json!({
                "model": MODEL,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}],
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "test-tool",
                        "parameters": {
                            "type": "object",
                            "properties": {"value": {"type": "string"}},
                            "required": ["value"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#"
                        }
                    }
                }],
                "parallel_tool_calls": false
            }),
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should pass promptCacheKey as prompt_cache_key
    #[tokio::test]
    async fn should_pass_prompt_cache_key_as_prompt_cache_key() {
        let fetch = json_fetch(&json_value(TEXT_FIXTURE));
        let mut opts = options();
        opts.provider_options = Some(
            serde_json::from_value(
                json!({"mistral": {"promptCacheKey": "classification-workflow-123"}}),
            )
            .unwrap(),
        );
        provider(&fetch)
            .chat(MODEL)
            .do_generate(&opts)
            .await
            .unwrap();
        assert_match_object(
            &fetch.seen()[0].json_body(),
            &json!({"prompt_cache_key": "classification-workflow-123"}),
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should avoid duplication when trailing assistant message
    #[tokio::test]
    async fn should_avoid_duplication_when_trailing_assistant_message() {
        let fetch = json_fetch(&completion(
            "16362f24e60340d0994dd205c267a43a",
            1711113008,
            json!({"role": "assistant", "content": "prefix and more content", "tool_calls": null}),
            json!({"prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30}),
        ));
        let mut opts = options();
        opts.prompt.push(LanguageModelMessage::Assistant {
            content: vec![AssistantPart::Text(TextPart {
                text: "prefix ".into(),
                provider_options: None,
            })],
            provider_options: None,
        });
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&opts)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{"text": "prefix and more content", "type": "text"}])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should preserve ordering of mixed thinking and text
    #[tokio::test]
    async fn should_preserve_ordering_of_mixed_thinking_and_text() {
        let fetch = json_fetch(&json!({
            "id": "mixed-content-test",
            "object": "chat.completion",
            "created": 1722349660,
            "model": "magistral-medium-2507",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        {"type": "thinking", "thinking": [{"type": "text", "text": "First thought."}]},
                        {"type": "text", "text": "Partial answer."},
                        {"type": "thinking", "thinking": [{"type": "text", "text": "Second thought."}]},
                        {"type": "text", "text": "Final answer."}
                    ]
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "total_tokens": 30, "completion_tokens": 20}
        }));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([
                {"text": "First thought.", "type": "reasoning"},
                {"text": "Partial answer.", "type": "text"},
                {"text": "Second thought.", "type": "reasoning"},
                {"text": "Final answer.", "type": "text"}
            ])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should handle empty thinking content
    #[tokio::test]
    async fn should_handle_empty_thinking_content() {
        let fetch = json_fetch(&json!({
            "id": "empty-thinking-test",
            "object": "chat.completion",
            "created": 1722349660,
            "model": "magistral-medium-2507",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        {"type": "thinking", "thinking": []},
                        {"type": "text", "text": "Just the answer."}
                    ]
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "total_tokens": 30, "completion_tokens": 20}
        }));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{"text": "Just the answer.", "type": "text"}])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should extract content when message content is a content object
    #[tokio::test]
    async fn should_extract_content_when_message_content_is_a_content_object() {
        let fetch = json_fetch(&json!({
            "object": "chat.completion",
            "id": "object-id",
            "created": 1711113008,
            "model": "mistral-small-latest",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Hello from object"}],
                    "tool_calls": null
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30}
        }));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{"text": "Hello from object", "type": "text"}])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › should return raw text with think tags
    #[tokio::test]
    async fn should_return_raw_text_with_think_tags() {
        let fetch = json_fetch(&json!({
            "object": "chat.completion",
            "id": "raw-think-id",
            "created": 1711113008,
            "model": "magistral-small-2506",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "<think>\nLet me think about this problem step by step.\nFirst, I need to understand what the user is asking.\nThen I can provide a helpful response.\n</think>\n\nHello! I'm ready to help you with your question.",
                    "tool_calls": null
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30}
        }));
        let result = provider(&fetch)
            .chat("magistral-small-2506")
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{
                "text": "<think>\nLet me think about this problem step by step.\nFirst, I need to understand what the user is asking.\nThen I can provide a helpful response.\n</think>\n\nHello! I'm ready to help you with your question.",
                "type": "text"
            }])
        );
    }

    mod warnings {
        use super::super::*;

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › warnings › should warn about unsupported reasoning for non-supporting models
        #[tokio::test]
        async fn should_warn_about_unsupported_reasoning_for_non_supporting_models() {
            let fetch = json_fetch(&json_value(TEXT_FIXTURE));
            let mut opts = options();
            opts.reasoning = Some(ReasoningEffort::Medium);
            let result = provider(&fetch)
                .chat("mistral-large-latest")
                .do_generate(&opts)
                .await
                .unwrap();
            assert!(has_unsupported(&result.warnings, "reasoning"));
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › warnings › should emit compatibility warning for reasoning medium on supporting model
        #[tokio::test]
        async fn should_emit_compatibility_warning_for_reasoning_medium_on_supporting_model() {
            let fetch = json_fetch(&json_value(TEXT_FIXTURE));
            let mut opts = options();
            opts.reasoning = Some(ReasoningEffort::Medium);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&opts)
                .await
                .unwrap();
            assert!(has_compatibility(&result.warnings, "reasoning"));
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › warnings › should not warn for reasoning high on supporting model
        #[tokio::test]
        async fn should_not_warn_for_reasoning_high_on_supporting_model() {
            let fetch = json_fetch(&json_value(TEXT_FIXTURE));
            let mut opts = options();
            opts.reasoning = Some(ReasoningEffort::High);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&opts)
                .await
                .unwrap();
            assert!(!has_any_feature(&result.warnings, "reasoning"));
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › warnings › should not warn about reasoning for %s
        #[tokio::test]
        async fn should_not_warn_about_reasoning_for_s() {
            for model_id in [
                "mistral-medium-3-5",
                "mistral-medium-latest",
                "mistral-vibe-cli-fast",
                "zai-glm-5-2",
                "glm-5-2",
                "labs-leanstral-1-5",
            ] {
                let fetch = json_fetch(&json_value(TEXT_FIXTURE));
                let mut opts = options();
                opts.reasoning = Some(ReasoningEffort::High);
                let result = provider(&fetch)
                    .chat(model_id)
                    .do_generate(&opts)
                    .await
                    .unwrap();
                assert!(
                    !has_unsupported(&result.warnings, "reasoning"),
                    "model {model_id}"
                );
            }
        }
    }

    mod reasoning_effort {
        use super::super::*;

        /// Generate with `reasoning` set to `effort` and return the request body.
        async fn body_for_reasoning(model_id: &str, effort: ReasoningEffort) -> Value {
            let fetch = json_fetch(&json_value(TEXT_FIXTURE));
            let mut opts = options();
            opts.reasoning = Some(effort);
            provider(&fetch)
                .chat(model_id)
                .do_generate(&opts)
                .await
                .unwrap();
            fetch.seen()[0].json_body()
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › reasoning_effort › should send reasoning_effort high for reasoning high
        #[tokio::test]
        async fn should_send_reasoning_effort_high_for_reasoning_high() {
            let body = body_for_reasoning(MODEL, ReasoningEffort::High).await;
            assert_match_object(&body, &json!({"reasoning_effort": "high"}));
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › reasoning_effort › should send reasoning_effort high for reasoning medium
        #[tokio::test]
        async fn should_send_reasoning_effort_high_for_reasoning_medium() {
            let body = body_for_reasoning(MODEL, ReasoningEffort::Medium).await;
            assert_match_object(&body, &json!({"reasoning_effort": "high"}));
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › reasoning_effort › should send reasoning_effort high for reasoning minimal
        #[tokio::test]
        async fn should_send_reasoning_effort_high_for_reasoning_minimal() {
            let body = body_for_reasoning(MODEL, ReasoningEffort::Minimal).await;
            assert_match_object(&body, &json!({"reasoning_effort": "high"}));
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › reasoning_effort › should send reasoning_effort none for reasoning none
        #[tokio::test]
        async fn should_send_reasoning_effort_none_for_reasoning_none() {
            let body = body_for_reasoning(MODEL, ReasoningEffort::None).await;
            assert_match_object(&body, &json!({"reasoning_effort": "none"}));
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › reasoning_effort › should allow provider option to override reasoning
        #[tokio::test]
        async fn should_allow_provider_option_to_override_reasoning() {
            let fetch = json_fetch(&json_value(TEXT_FIXTURE));
            let mut opts = options();
            opts.reasoning = Some(ReasoningEffort::High);
            opts.provider_options = Some(
                serde_json::from_value(json!({"mistral": {"reasoningEffort": "none"}})).unwrap(),
            );
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&opts)
                .await
                .unwrap();
            assert_match_object(
                &fetch.seen()[0].json_body(),
                &json!({"reasoning_effort": "none"}),
            );
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › reasoning_effort › should not send reasoning_effort for non-supporting models
        #[tokio::test]
        async fn should_not_send_reasoning_effort_for_non_supporting_models() {
            let body = body_for_reasoning("mistral-large-latest", ReasoningEffort::High).await;
            assert!(body.get("reasoning_effort").is_none());
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doGenerate › reasoning_effort › should send reasoning_effort for %s
        #[tokio::test]
        async fn should_send_reasoning_effort_for_s() {
            for model_id in [
                "mistral-medium-3-5",
                "mistral-medium-latest",
                "mistral-vibe-cli-fast",
                "zai-glm-5-2",
                "glm-5-2",
                "labs-leanstral-1-5",
            ] {
                let body = body_for_reasoning(model_id, ReasoningEffort::High).await;
                assert_match_object(&body, &json!({"reasoning_effort": "high"}));
            }
        }
    }
}

// ── doStream ────────────────────────────────────────────────────────────────

mod do_stream {
    use super::*;

    mod tool_call {
        use super::super::*;

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › tool call › should stream tool call
        #[tokio::test]
        async fn should_stream_tool_call() {
            let fetch = chunks_fetch(TOOL_CALL_CHUNKS, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&options())
                .await
                .unwrap();
            assert_eq!(
                Value::Array(stream_parts(result).await),
                json_value(SNAPSHOT_DO_STREAM_TOOL_CALL)
            );
        }

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › tool call › should accumulate incremental tool call arguments
        #[tokio::test]
        async fn should_accumulate_incremental_tool_call_arguments() {
            let fetch = chunks_fetch(INCREMENTAL_TOOL_CALL_CHUNKS, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&options())
                .await
                .unwrap();
            let parts = stream_parts(result).await;
            let tool_parts: Vec<Value> = parts
                .into_iter()
                .filter(|part| {
                    let kind = part["type"].as_str().unwrap_or_default();
                    kind == "error" || kind.starts_with("tool-")
                })
                .collect();
            assert_eq!(
                tool_parts,
                vec![
                    json!({
                        "type": "tool-input-start",
                        "id": "chatcmpl-tool-9f149c74c42f265b",
                        "toolName": "webSearchTool"
                    }),
                    json!({
                        "type": "tool-input-delta",
                        "id": "chatcmpl-tool-9f149c74c42f265b",
                        "delta": "{\"query\": \"current Berlin weather\"}"
                    }),
                    json!({
                        "type": "tool-input-end",
                        "id": "chatcmpl-tool-9f149c74c42f265b"
                    }),
                    json!({
                        "type": "tool-call",
                        "toolCallId": "chatcmpl-tool-9f149c74c42f265b",
                        "toolName": "webSearchTool",
                        "input": "{\"query\": \"current Berlin weather\"}"
                    }),
                ]
            );
        }
    }

    mod reasoning {
        use super::super::*;

        /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › reasoning › should stream reasoning
        #[tokio::test]
        async fn should_stream_reasoning() {
            let fetch = chunks_fetch(REASONING_CHUNKS, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&options())
                .await
                .unwrap();
            assert_eq!(
                Value::Array(stream_parts(result).await),
                json_value(SNAPSHOT_DO_STREAM_REASONING)
            );
        }
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should pass the messages
    #[tokio::test]
    async fn should_pass_the_messages() {
        let fetch = chunks_fetch(TEXT_CHUNKS, &[]);
        provider(&fetch)
            .chat(MODEL)
            .do_stream(&options())
            .await
            .unwrap();
        let mut expected = hello_body();
        expected["stream"] = json!(true);
        assert_eq!(fetch.seen()[0].json_body(), expected);
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should pass promptCacheKey as prompt_cache_key
    #[tokio::test]
    async fn should_pass_prompt_cache_key_as_prompt_cache_key() {
        let fetch = chunks_fetch(TEXT_CHUNKS, &[]);
        let mut opts = options();
        opts.provider_options = Some(
            serde_json::from_value(
                json!({"mistral": {"promptCacheKey": "classification-workflow-123"}}),
            )
            .unwrap(),
        );
        provider(&fetch).chat(MODEL).do_stream(&opts).await.unwrap();
        assert_match_object(
            &fetch.seen()[0].json_body(),
            &json!({"prompt_cache_key": "classification-workflow-123", "stream": true}),
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let fetch = chunks_fetch(TEXT_CHUNKS, &[]);
        let provider = provider_with(
            &fetch,
            id_generator(0),
            Some(HashMap::from([(
                "Custom-Provider-Header".to_string(),
                Some("provider-header-value".to_string()),
            )])),
        );
        let mut opts = options();
        opts.headers = Some(HashMap::from([(
            "Custom-Request-Header".to_string(),
            "request-header-value".to_string(),
        )]));
        provider.chat(MODEL).do_stream(&opts).await.unwrap();
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

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let fetch = chunks_fetch(TEXT_CHUNKS, &[("test-header", "test-value")]);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_stream(&options())
            .await
            .unwrap();
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

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should preserve the final complete raw usage object without changing normalized usage
    #[tokio::test]
    async fn should_preserve_the_final_complete_raw_usage_object_without_changing_normalized_usage()
    {
        let fetch = chunks_fetch(USAGE_DETAILS_CHUNKS, &[]);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_stream(&options())
            .await
            .unwrap();
        let parts = stream_parts(result).await;
        let finish = parts
            .iter()
            .find(|part| part["type"] == "finish")
            .expect("a finish part");
        assert_eq!(
            finish,
            &json!({
                "type": "finish",
                "finishReason": {"unified": "stop", "raw": "stop"},
                "usage": {
                    "inputTokens": {"total": 20, "noCache": 20},
                    "outputTokens": {"total": 2, "text": 2},
                    "raw": {
                        "prompt_tokens": 20,
                        "completion_tokens": 2,
                        "total_tokens": 22,
                        "prompt_audio_seconds": 1,
                        "request_count": 1,
                        "service_tier": "standard",
                        "num_cached_tokens": 0,
                        "prompt_tokens_details": {
                            "cached_tokens": 0,
                            "audio_tokens": 1,
                            "messages": [{
                                "role": "user",
                                "total_tokens": 20,
                                "settings_tokens": null,
                                "truncated": false,
                                "usage_count": 1
                            }],
                            "additional_prompt_detail": {"value": true}
                        },
                        "prompt_token_details": {
                            "cached_tokens": 0,
                            "audio_tokens": 1,
                            "additional_prompt_token_detail": ["value"]
                        },
                        "completion_tokens_details": {
                            "reasoning_tokens": 7,
                            "additional_completion_detail": "value"
                        },
                        "additional_usage_field": {"nested": true}
                    }
                }
            })
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let fetch = chunks_fetch(TEXT_CHUNKS, &[]);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_stream(&options())
            .await
            .unwrap();
        let mut expected = hello_body();
        expected["stream"] = json!(true);
        assert_eq!(
            serde_json::to_value(&result.request).unwrap(),
            json!({"body": expected})
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should avoid duplication when trailing assistant message
    #[tokio::test]
    async fn should_avoid_duplication_when_trailing_assistant_message() {
        let fetch = stream_fetch(&[
            r#"{"id":"53ff663126294946a6b7a4747b70597e","object":"chat.completion.chunk","created":1750537996,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"53ff663126294946a6b7a4747b70597e","object":"chat.completion.chunk","created":1750537996,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":"prefix"},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"53ff663126294946a6b7a4747b70597e","object":"chat.completion.chunk","created":1750537996,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":" and"},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"53ff663126294946a6b7a4747b70597e","object":"chat.completion.chunk","created":1750537996,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":" more content"},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"53ff663126294946a6b7a4747b70597e","object":"chat.completion.chunk","created":1750537996,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":""},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":4,"total_tokens":36,"completion_tokens":32}}"#,
        ]);
        let mut opts = options();
        opts.prompt.push(LanguageModelMessage::Assistant {
            content: vec![AssistantPart::Text(TextPart {
                text: "prefix ".into(),
                provider_options: None,
            })],
            provider_options: None,
        });
        let result = provider(&fetch).chat(MODEL).do_stream(&opts).await.unwrap();
        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {"type": "stream-start", "warnings": []},
                {
                    "id": "53ff663126294946a6b7a4747b70597e",
                    "modelId": "mistral-small-latest",
                    "timestamp": "2025-06-21T20:33:16.000Z",
                    "type": "response-metadata"
                },
                {"id": "0", "type": "text-start"},
                {"delta": "prefix", "id": "0", "type": "text-delta"},
                {"delta": " and", "id": "0", "type": "text-delta"},
                {"delta": " more content", "id": "0", "type": "text-delta"},
                {"id": "0", "type": "text-end"},
                {
                    "finishReason": {"raw": "stop", "unified": "stop"},
                    "type": "finish",
                    "usage": {
                        "inputTokens": {"noCache": 4, "total": 4},
                        "outputTokens": {"text": 32, "total": 32},
                        "raw": {"completion_tokens": 32, "prompt_tokens": 4, "total_tokens": 36}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should stream text with content objects
    #[tokio::test]
    async fn should_stream_text_with_content_objects() {
        let fetch = stream_fetch(&[
            r#"{"id":"b9e43f82d6c74a1e9f5b2c8e7a9d4f6b","object":"chat.completion.chunk","created":1750538500,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"text","text":""}]},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"b9e43f82d6c74a1e9f5b2c8e7a9d4f6b","object":"chat.completion.chunk","created":1750538500,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":[{"type":"text","text":"Hello"}]},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"b9e43f82d6c74a1e9f5b2c8e7a9d4f6b","object":"chat.completion.chunk","created":1750538500,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":[{"type":"text","text":", world!"}]},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":4,"total_tokens":36,"completion_tokens":32}}"#,
        ]);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_stream(&options())
            .await
            .unwrap();
        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {"type": "stream-start", "warnings": []},
                {
                    "id": "b9e43f82d6c74a1e9f5b2c8e7a9d4f6b",
                    "modelId": "mistral-small-latest",
                    "timestamp": "2025-06-21T20:41:40.000Z",
                    "type": "response-metadata"
                },
                {"id": "0", "type": "text-start"},
                {"delta": "Hello", "id": "0", "type": "text-delta"},
                {"delta": ", world!", "id": "0", "type": "text-delta"},
                {"id": "0", "type": "text-end"},
                {
                    "finishReason": {"raw": "stop", "unified": "stop"},
                    "type": "finish",
                    "usage": {
                        "inputTokens": {"noCache": 4, "total": 4},
                        "outputTokens": {"text": 32, "total": 32},
                        "raw": {"completion_tokens": 32, "prompt_tokens": 4, "total_tokens": 36}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should handle interleaved thinking and text
    #[tokio::test]
    async fn should_handle_interleaved_thinking_and_text() {
        // The second reasoning block is `id-2`: the shared `mockId()` counter
        // has already handed out `id-0` to the reasoning stream test.
        let fetch = stream_fetch(&[
            r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"thinking","thinking":[{"type":"text","text":"First thought."}]}]},"finish_reason":null}]}"#,
            r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"text","text":"Partial answer."}]},"finish_reason":null}]}"#,
            r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"thinking","thinking":[{"type":"text","text":"Second thought."}]}]},"finish_reason":null}]}"#,
            r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"text","text":"Final answer."}]},"finish_reason":null}]}"#,
            r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"content":""},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"total_tokens":40,"completion_tokens":30}}"#,
        ]);
        let provider = provider_with(&fetch, id_generator(1), None);
        let result = provider.chat(MODEL).do_stream(&options()).await.unwrap();
        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {"type": "stream-start", "warnings": []},
                {
                    "id": "interleaved-test",
                    "modelId": "magistral-small-2507",
                    "timestamp": "2025-06-21T20:33:20.000Z",
                    "type": "response-metadata"
                },
                {"id": "id-1", "type": "reasoning-start"},
                {"delta": "First thought.", "id": "id-1", "type": "reasoning-delta"},
                {"id": "id-1", "type": "reasoning-end"},
                {"id": "0", "type": "text-start"},
                {"delta": "Partial answer.", "id": "0", "type": "text-delta"},
                {"id": "0", "type": "text-end"},
                {"id": "id-2", "type": "reasoning-start"},
                {"delta": "Second thought.", "id": "id-2", "type": "reasoning-delta"},
                {"id": "id-2", "type": "reasoning-end"},
                {"id": "0", "type": "text-start"},
                {"delta": "Final answer.", "id": "0", "type": "text-delta"},
                {"id": "0", "type": "text-end"},
                {
                    "finishReason": {"raw": "stop", "unified": "stop"},
                    "type": "finish",
                    "usage": {
                        "inputTokens": {"noCache": 10, "total": 10},
                        "outputTokens": {"text": 30, "total": 30},
                        "raw": {"completion_tokens": 30, "prompt_tokens": 10, "total_tokens": 40}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › doStream › should stream raw chunks
    #[tokio::test]
    async fn should_stream_raw_chunks() {
        let fetch = stream_fetch(&[
            r#"{"id":"c7d54e93f8a64b2e9c1f5a8b7d9e2f4c","object":"chat.completion.chunk","created":1750538600,"model":"mistral-large-latest","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"d8e65fa4g9b75c3f0d2g6b9c8e0f3g5d","object":"chat.completion.chunk","created":1750538601,"model":"mistral-large-latest","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"e9f76gb5h0c86d4g1e3h7c0d9f1g4h6e","object":"chat.completion.chunk","created":1750538602,"model":"mistral-large-latest","choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
        ]);
        let mut opts = options();
        opts.include_raw_chunks = Some(true);
        let result = provider(&fetch).chat(MODEL).do_stream(&opts).await.unwrap();
        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {"type": "stream-start", "warnings": []},
                {
                    "type": "raw",
                    "rawValue": {
                        "choices": [{
                            "delta": {"content": "Hello", "role": "assistant"},
                            "finish_reason": null,
                            "index": 0,
                            "logprobs": null
                        }],
                        "created": 1750538600,
                        "id": "c7d54e93f8a64b2e9c1f5a8b7d9e2f4c",
                        "model": "mistral-large-latest",
                        "object": "chat.completion.chunk"
                    }
                },
                {
                    "id": "c7d54e93f8a64b2e9c1f5a8b7d9e2f4c",
                    "modelId": "mistral-large-latest",
                    "timestamp": "2025-06-21T20:43:20.000Z",
                    "type": "response-metadata"
                },
                {"id": "0", "type": "text-start"},
                {"delta": "Hello", "id": "0", "type": "text-delta"},
                {
                    "type": "raw",
                    "rawValue": {
                        "choices": [{
                            "delta": {"content": " world"},
                            "finish_reason": null,
                            "index": 0,
                            "logprobs": null
                        }],
                        "created": 1750538601,
                        "id": "d8e65fa4g9b75c3f0d2g6b9c8e0f3g5d",
                        "model": "mistral-large-latest",
                        "object": "chat.completion.chunk"
                    }
                },
                {"delta": " world", "id": "0", "type": "text-delta"},
                {
                    "type": "raw",
                    "rawValue": {
                        "choices": [{
                            "delta": {},
                            "finish_reason": "stop",
                            "index": 0,
                            "logprobs": null
                        }],
                        "created": 1750538602,
                        "id": "e9f76gb5h0c86d4g1e3h7c0d9f1g4h6e",
                        "model": "mistral-large-latest",
                        "object": "chat.completion.chunk",
                        "usage": {"completion_tokens": 5, "prompt_tokens": 10, "total_tokens": 15}
                    }
                },
                {"id": "0", "type": "text-end"},
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

// ── tool result format support ──────────────────────────────────────────────

mod tool_result_format_support {
    use super::*;

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › tool result format support › should handle new LanguageModelV4ToolResultOutput format
    #[tokio::test]
    async fn should_handle_new_language_model_v4_tool_result_output_format() {
        let fetch = json_fetch(&json!({
            "id": "test-id",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "mistral-small",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Here is the result",
                    "tool_calls": null
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }));
        let opts = CallOptions::new(vec![
            LanguageModelMessage::user_text("Hello"),
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call-1".into(),
                    tool_name: "test-tool".into(),
                    input: json!({"query": "test"}),
                    provider_executed: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call-1".into(),
                    tool_name: "test-tool".into(),
                    output: ToolResultOutput::Json {
                        value: json!({"result": "success"}),
                        provider_options: None,
                    },
                    provider_options: None,
                })],
                provider_options: None,
            },
        ]);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&opts)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{"type": "text", "text": "Here is the result"}])
        );
        assert_eq!(
            serde_json::to_value(&result.finish_reason).unwrap(),
            json!({"raw": "stop", "unified": "stop"})
        );
    }
}

// ── reference content parsing ───────────────────────────────────────────────

mod reference_content_parsing {
    use super::*;

    /// A completion whose content is `Here is the info` plus a reference part.
    fn reference_completion(id: &str, reference: Value) -> Value {
        completion(
            id,
            1711113008,
            json!({
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Here is the info"},
                    reference
                ],
                "tool_calls": null
            }),
            json!({"prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30}),
        )
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › reference content parsing › should handle reference_ids as numbers
    #[tokio::test]
    async fn should_handle_reference_ids_as_numbers() {
        let fetch = json_fetch(&reference_completion(
            "test-id",
            json!({"type": "reference", "reference_ids": [1, 2, 3]}),
        ));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{"text": "Here is the info", "type": "text"}])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › reference content parsing › should handle reference_ids as strings
    #[tokio::test]
    async fn should_handle_reference_ids_as_strings() {
        let fetch = json_fetch(&reference_completion(
            "test-id",
            json!({"type": "reference", "reference_ids": ["ref-1", "ref-2", "ref-3"]}),
        ));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{"text": "Here is the info", "type": "text"}])
        );
    }

    /// upstream: @ai-sdk/mistral src/mistral-chat-language-model.test.ts › reference content parsing › should handle mixed reference_ids
    #[tokio::test]
    async fn should_handle_mixed_reference_ids() {
        let fetch = json_fetch(&reference_completion(
            "test-id",
            json!({"type": "reference", "reference_ids": [1, "ref-2", 3]}),
        ));
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{"text": "Here is the info", "type": "text"}])
        );
    }
}
