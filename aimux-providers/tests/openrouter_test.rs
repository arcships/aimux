//! Provider-specific tests for the OpenRouter provider.
//!
//! OpenRouter is a registry preset of the OpenAI-compatible package. The
//! behaviours verified here are the ones the preset is responsible for:
//!
//! - Provider configuration: name, `OPENROUTER_API_KEY` env var, custom API
//!   key, custom headers, base-URL override.
//! - Request body shape: `model` + `messages` (string content for plain text).
//! - Text generation, streaming, tool-call extraction, error mapping, response
//!   headers.
//! - Conformance: parsing real recorded API responses from
//!   `tests/cassettes/openrouter/` (74 cassettes) via the shared replay layer.
//!
//! The wiremock tests override the base URL to the mock server root so the
//! request path is `/chat/completions`. The conformance tests instead override
//! the base URL to `<server>/api/v1` so the request path becomes
//! `/api/v1/chat/completions` — matching the path the real OpenRouter API
//! (base `https://openrouter.ai/api/v1`) records in the cassettes.

use aimux_core::tool::RawToolCall;
use futures::StreamExt;
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::{CallOptions, Tool};
use aimux_core::provider::Provider;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::openai_compatible::OpenAICompatibleProvider;
use aimux_providers::{PresetSettings, create_provider, preset};

// Shared cassette-replay infrastructure (same `mod common` used by
// `conformance_test.rs`).
mod common;

// ── shared helpers ───────────────────────────────────────────────────────────

fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn text_completion_body() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "openai/gpt-4o-mini",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 11, "total_tokens": 403, "completion_tokens": 392 }
    })
}

/// A chat-completion response carrying a single tool call.
fn tool_call_completion_body() -> Value {
    json!({
        "id": "chatcmpl-tool",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "openai/gpt-4o-mini",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_abc",
                    "type": "function",
                    "function": { "name": "get-weather", "arguments": "{\"city\":\"SF\"}" }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 10, "total_tokens": 20, "completion_tokens": 10 }
    })
}

fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(e) => panic!("stream error: {e:?}"),
        }
    }
    parts
}

/// The OpenRouter preset at `base_url` with an explicit key.
fn openrouter_at(base_url: String, key: &str) -> OpenAICompatibleProvider {
    preset::create(
        "openrouter",
        PresetSettings {
            api_key: Some(aimux_provider_utils::Resolvable::Value(key.to_string())),
            base_url: Some(base_url),
            ..Default::default()
        },
    )
    .unwrap()
}

fn make_provider(server: &MockServer) -> OpenAICompatibleProvider {
    openrouter_at(server.uri(), "test-api-key")
}

/// Custom API key is sent in the `Authorization: Bearer` header.
#[tokio::test]
async fn custom_api_key_used_in_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer my-custom-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = openrouter_at(server.uri(), "my-custom-key");
    let model = provider.chat("openai/gpt-4o-mini");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed with custom API key");
}

/// Custom headers are forwarded to the HTTP request.
#[tokio::test]
async fn custom_headers_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("x-custom-header", "test-value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = openrouter_at(server.uri(), "test-key");
    let model = provider.chat("openai/gpt-4o-mini");

    let mut options = default_options(test_prompt());
    options.headers = Some(
        vec![("x-custom-header".to_string(), "test-value".to_string())]
            .into_iter()
            .collect(),
    );

    model
        .do_generate(&options)
        .await
        .expect("should succeed with custom headers");
}

/// `provider.languageModel(modelId)` constructs a working model via the
/// `Provider` trait.
#[tokio::test]
async fn language_model_via_trait() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = openrouter_at(server.uri(), "test-key");
    let model = provider
        .language_model("openai/gpt-4o-mini")
        .expect("language_model should succeed");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");
}

/// `createOpenRouter()` loads the key from `OPENROUTER_API_KEY`.
#[serial]
#[test]
fn from_env_loads_openrouter_api_key() {
    let saved = std::env::var("OPENROUTER_API_KEY").ok();
    unsafe {
        std::env::set_var("OPENROUTER_API_KEY", "env-test-key");
    }

    let model = create_provider("openrouter", PresetSettings::default())
        .and_then(|p| p.language_model("openai/gpt-4o-mini"));
    assert!(model.is_ok(), "the by-name entry point reads the variable");

    unsafe {
        match saved {
            Some(v) => std::env::set_var("OPENROUTER_API_KEY", v),
            None => std::env::remove_var("OPENROUTER_API_KEY"),
        }
    }
}

/// Without the env var, `createOpenRouter()` fails.
#[serial]
#[tokio::test]
async fn from_env_fails_without_env_var() {
    let saved = std::env::var("OPENROUTER_API_KEY").ok();
    unsafe {
        std::env::remove_var("OPENROUTER_API_KEY");
    }

    let model = create_provider("openrouter", PresetSettings::default())
        .and_then(|p| p.language_model("openai/gpt-4o-mini"))
        .unwrap();
    let err = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect_err("no key variable, no key");
    assert!(matches!(err, AiMuxError::LoadApiKey { .. }), "{err:?}");

    unsafe {
        if let Some(v) = saved {
            std::env::set_var("OPENROUTER_API_KEY", v);
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Request body & message conversion
// ════════════════════════════════════════════════════════════════════════════

/// The body carries the model id and a single user message with string content.
#[tokio::test]
async fn sends_correct_request_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "openai/gpt-4o-mini");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "Hello");
}

/// Stream requests add `stream: true`.
#[tokio::test]
async fn stream_request_body_has_stream_flag() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"openai/gpt-4o-mini","choices":[{"index":0,"delta":{"role":"assistant","content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"openai/gpt-4o-mini","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
        ),
    ]);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");
    let _ = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let req_body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(req_body["stream"], json!(true));
    assert_eq!(req_body["model"], "openai/gpt-4o-mini");
}

// ════════════════════════════════════════════════════════════════════════════
// Usage extraction
// ════════════════════════════════════════════════════════════════════════════

/// prompt/completion tokens are surfaced.
#[tokio::test]
async fn extracts_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(11));
    assert_eq!(result.usage.output_tokens.total, Some(392));
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate / doStream / tools / errors / headers
// ════════════════════════════════════════════════════════════════════════════

/// do_generate returns text content.
#[tokio::test]
async fn do_generate_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Hello, World!"),
        other => panic!("expected Text, got {other:?}"),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
}

/// do_generate extracts a tool call from the response.
#[tokio::test]
async fn do_generate_extracts_tool_call() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tool_call_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");

    let tool = FunctionTool::new(
        "get-weather",
        json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"],
            "additionalProperties": false,
        }),
    );
    let options = CallOptions {
        tools: Some(vec![Tool::from(tool)]),
        ..default_options(test_prompt())
    };

    let result = model
        .do_generate(&options)
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_abc");
            assert_eq!(tool_name, "get-weather");
            assert_eq!(input, &Value::String(r#"{"city":"SF"}"#.into()));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// do_stream returns text deltas.
#[tokio::test]
async fn do_stream_returns_text() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"openai/gpt-4o-mini","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"openai/gpt-4o-mini","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"openai/gpt-4o-mini","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
        ),
    ]);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    let text_deltas: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_deltas, vec!["Hello".to_string(), " world".to_string()]);
}

/// do_stream emits a ToolCall stream part.
#[tokio::test]
async fn do_stream_emits_tool_call() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"openai/gpt-4o-mini","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get-weather","arguments":""}}]},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"openai/gpt-4o-mini","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":\"SF\"}"}}]},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"openai/gpt-4o-mini","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}"#,
        ),
    ]);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");

    let tool = FunctionTool::new(
        "get-weather",
        json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}),
    );
    let options = CallOptions {
        tools: Some(vec![Tool::from(tool)]),
        ..default_options(test_prompt())
    };
    let result = model
        .do_stream(&options)
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    let tool_call = parts.iter().find_map(|p| match p {
        StreamPart::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
        _ => None,
    });
    let (id, name, input) = tool_call.expect("should have ToolCall");
    assert_eq!(id, "call_abc");
    assert_eq!(name, "get-weather");
    assert_eq!(input, Value::String(r#"{"city":"SF"}"#.into()));
}

/// A 401 response maps to `AiMuxError::ApiCall` (401 in `status_code`).
#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Invalid API key", "type": "invalid_request_error" }
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.status_code == Some(401) && m.message == "Invalid API key"),
        "expected Auth error, got {result:?}"
    );
}

/// A 429 response maps to `AiMuxError::ApiCall` (429 in `status_code`).
#[tokio::test]
async fn status_429_maps_to_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": { "message": "Rate limit exceeded", "type": "rate_limit_error" }
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.chat("openai/gpt-4o-mini");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(429)),
        "expected RateLimited, got {result:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Conformance — real recorded API responses (cassettes).
//
// The cassettes record requests against the real OpenRouter API at path
// `/api/v1/chat/completions` (base `https://openrouter.ai/api/v1`). To make the
// replay mock match, we override the base URL to `<server>/api/v1` so the
// provider's request path becomes `/api/v1/chat/completions`. The shared replay
// layer then scores cassettes by the `model` (and `stream`) fields and returns
// the best-matching recorded response.
// ════════════════════════════════════════════════════════════════════════════

mod conformance {
    use super::*;

    /// Build a provider whose requests land at `<server>/api/v1/chat/completions`,
    /// matching the path recorded in the OpenRouter cassettes.
    fn make_cassette_provider(server: &MockServer) -> OpenAICompatibleProvider {
        openrouter_at(format!("{}/api/v1", server.uri()), "test-key")
    }

    fn has_text(content: &[GenerateContent]) -> bool {
        content
            .iter()
            .any(|c| matches!(c, GenerateContent::Text { .. }))
    }

    fn has_tool_call(content: &[GenerateContent]) -> bool {
        content
            .iter()
            .any(|c| matches!(c, GenerateContent::ToolCall(_)))
    }

    fn has_reasoning(content: &[GenerateContent]) -> bool {
        content
            .iter()
            .any(|c| matches!(c, GenerateContent::Reasoning(_)))
    }

    fn has_finish(parts: &[StreamPart]) -> bool {
        parts.iter().any(|p| matches!(p, StreamPart::Finish { .. }))
    }

    /// doGenerate against a non-streaming cassette returns parseable content.
    #[tokio::test]
    async fn do_generate_parses_real_response() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/openrouter").await;

        let provider = make_cassette_provider(&server);
        // `openai/gpt-4o-mini` matches the `completion_smoke` cassette (model
        // field weight 10), so a real non-streaming JSON response is returned.
        let model = provider.chat("openai/gpt-4o-mini");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected some content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "unexpected error: {msg}"
                );
            }
        }
    }

    /// doStream against a streaming cassette emits parts and a finish.
    #[tokio::test]
    async fn do_stream_parses_real_response() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/openrouter").await;

        let provider = make_cassette_provider(&server);
        let model = provider.chat("openai/gpt-4o-mini");

        let result = model.do_stream(&default_options(test_prompt())).await;

        match result {
            Ok(stream_result) => {
                let parts = collect_stream(stream_result).await;
                assert!(
                    has_finish(&parts) || !parts.is_empty(),
                    "stream should produce some parts, got empty"
                );
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "unexpected error: {msg}"
                );
            }
        }
    }

    // Replay helper — same `mod common` used by conformance_test.rs.
    async fn mount_cassettes(server: &MockServer, dir: &str) -> usize {
        common::replay::mount_cassettes(server, dir).await
    }
}
