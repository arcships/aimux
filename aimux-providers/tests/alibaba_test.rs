//! Provider-specific tests for the Alibaba (DashScope / Qwen) provider.
//!
//! Translated from the TypeScript suites:
//! - `packages/alibaba/src/alibaba-provider.test.ts`
//! - `packages/alibaba/src/alibaba-chat-language-model.test.ts`
//! - `packages/alibaba/src/convert-alibaba-usage.test.ts`
//!
//! Alibaba is a thin OpenAI-compatible wrapper over [`OpenAIProvider`]. The
//! behaviours verified here are the ones the wrapper is responsible for:
//!
//! - Provider configuration: name, `ALIBABA_API_KEY` env var, custom API key,
//!   custom headers, base-URL override.
//! - Request body shape: `model` + `messages`.
//! - Usage conversion with cache tokens (`cached_tokens` �?cacheRead,
//!   `cache_creation_input_tokens` �?cacheWrite) and reasoning tokens.
//! - Top-level `reasoning` maps to `reasoning_effort` (shared OpenAI behaviour).
//! - Text generation, tool-call extraction, streaming, error mapping.
//!
//! Note: the TS Alibaba provider emits message `content` as an array of
//! `{type,text}` blocks even for single-text messages, and maps `reasoning` to
//! `enable_thinking`/`thinking_budget`. The Rust wrapper delegates to the
//! shared OpenAI converter which emits string `content` for plain-text
//! messages and maps `reasoning` to `reasoning_effort`; both are accepted by
//! DashScope's OpenAI-compatible mode. Those TS-specific behaviours are
//! therefore not asserted here.

use futures::StreamExt;
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};

use aimux_providers::{AlibabaConfig, AlibabaProvider};

// ── shared helpers ───────────────────────────────────────────────────────────

/// TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Standard non-streaming chat-completion body returning "Hello, World!".
fn text_completion_body() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "qwen-plus",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}

fn sse_event(json_str: &str) -> String {
    format!("data: {}\n\n", json_str)
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
            Err(e) => panic!("stream error: {:?}", e),
        }
    }
    parts
}

fn make_provider(server: &MockServer) -> AlibabaProvider {
    let config = AlibabaConfig::new("test-api-key").with_base_url(server.uri());
    AlibabaProvider::new(config)
}

// ════════════════════════════════════════════════════════════════════════════
// Provider configuration (alibaba-provider.test.ts)
// ════════════════════════════════════════════════════════════════════════════

/// TS: `createAlibaba()` produces a provider whose name is "alibaba".
#[test]
fn provider_name_is_alibaba() {
    let config = AlibabaConfig::new("test-key");
    let provider = AlibabaProvider::new(config);
    assert_eq!(provider.name(), "alibaba");
}

/// TS: `createAlibaba({ apiKey: 'custom-key' })` �?the custom key is sent in
/// the `Authorization: Bearer` header.
#[tokio::test]
async fn custom_api_key_used_in_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer my-custom-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let config = AlibabaConfig::new("my-custom-key").with_base_url(server.uri());
    let provider = AlibabaProvider::new(config);
    let model = provider.model("qwen-plus");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed with custom API key");
}

/// TS: `createAlibaba({ headers: { 'Custom-Header': 'value' } })` �?custom
/// headers are forwarded to the HTTP request.
#[tokio::test]
async fn custom_headers_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("x-custom-header", "test-value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let config = AlibabaConfig::new("test-key").with_base_url(server.uri());
    let provider = AlibabaProvider::new(config);
    let model = provider.model("qwen-plus");

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

/// TS: `provider.languageModel(modelId)` constructs a working model via the
/// `Provider` trait.
#[tokio::test]
async fn language_model_via_trait() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let config = AlibabaConfig::new("test-key").with_base_url(server.uri());
    let provider = AlibabaProvider::new(config);
    let model = provider
        .language_model("qwen-plus")
        .expect("language_model should succeed");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");
}

/// TS: `createAlibaba()` loads the key from the `ALIBABA_API_KEY` env var.
#[serial]
#[test]
fn from_env_loads_alibaba_api_key() {
    let saved = std::env::var("ALIBABA_API_KEY").ok();
    unsafe {
        std::env::set_var("ALIBABA_API_KEY", "env-test-key");
    }

    let config = AlibabaConfig::from_env();
    assert!(config.is_ok(), "from_env should succeed with env var set");

    unsafe {
        match saved {
            Some(v) => std::env::set_var("ALIBABA_API_KEY", v),
            None => std::env::remove_var("ALIBABA_API_KEY"),
        }
    }
}

/// TS: without the env var, `createAlibaba()` fails.
#[serial]
#[test]
fn from_env_fails_without_env_var() {
    let saved = std::env::var("ALIBABA_API_KEY").ok();
    unsafe {
        std::env::remove_var("ALIBABA_API_KEY");
    }

    let config = AlibabaConfig::from_env();
    assert!(config.is_err(), "from_env should fail without env var");

    unsafe {
        if let Some(v) = saved {
            std::env::set_var("ALIBABA_API_KEY", v);
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Request body & message conversion (alibaba-chat-language-model.test.ts)
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should send correct request body" �?the body carries the model id and
/// a single user message.
#[tokio::test]
async fn sends_correct_request_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "qwen-plus");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "Hello");
}

/// TS: a tool-call assistant message round-trips as `tool_calls` and a tool
/// result as a `tool` role message (shared OpenAI conversion).
#[tokio::test]
async fn converts_tool_call_and_tool_result_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");

    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call_id: "call-1".to_string(),
                tool_name: "get_weather".to_string(),
                input: json!({"location": "SF"}),
                provider_options: None,
            }],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: "call-1".to_string(),
                result: json!({"temp": 72}),
                tool_name: None,
                is_error: None,
                preliminary: None,
                dynamic: None,
                provider_options: None,
            }],
            ..Default::default()
        },
    ];
    let _ = model.do_generate(&default_options(prompt)).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    // Assistant tool-call message.
    assert_eq!(body["messages"][0]["role"], "assistant");
    assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call-1");
    assert_eq!(
        body["messages"][0]["tool_calls"][0]["function"]["name"],
        "get_weather"
    );
    // Tool result message.
    assert_eq!(body["messages"][1]["role"], "tool");
    assert_eq!(body["messages"][1]["tool_call_id"], "call-1");
}

// ════════════════════════════════════════════════════════════════════════════
// Usage conversion (convert-alibaba-usage.test.ts + chat model usage tests)
// ════════════════════════════════════════════════════════════════════════════

/// TS (convert-alibaba-usage): cache tokens distribute correctly �?/// `cached_tokens` �?cacheRead, `cache_creation_input_tokens` �?cacheWrite,
/// and `noCache = prompt - cacheRead - cacheWrite`.
#[tokio::test]
async fn extracts_usage_with_cache_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-cache-test",
            "object": "chat.completion",
            "created": 1770764844,
            "model": "qwen-plus",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello" },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 200,
                "completion_tokens": 75,
                "total_tokens": 275,
                "prompt_tokens_details": {
                    "cached_tokens": 120,
                    "cache_creation_input_tokens": 50
                },
                "completion_tokens_details": {
                    "reasoning_tokens": 25
                }
            }
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let usage = result.usage;
    assert_eq!(usage.input_tokens.total, Some(200));
    assert_eq!(usage.input_tokens.cache_read, Some(120));
    assert_eq!(usage.input_tokens.cache_write, Some(50));
    assert_eq!(usage.input_tokens.no_cache, Some(30));
    assert_eq!(usage.output_tokens.total, Some(75));
    assert_eq!(usage.output_tokens.reasoning, Some(25));
    assert_eq!(usage.output_tokens.text, Some(50));
}

/// TS (alibaba-reasoning fixture): reasoning tokens are surfaced on the
/// output side.
#[tokio::test]
async fn extracts_usage_with_reasoning_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-reasoning",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "qwen-plus",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Answer" },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 24,
                "completion_tokens": 1668,
                "total_tokens": 1692,
                "completion_tokens_details": { "reasoning_tokens": 1353 }
            }
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(24));
    assert_eq!(result.usage.output_tokens.total, Some(1668));
    assert_eq!(result.usage.output_tokens.reasoning, Some(1353));
    assert_eq!(result.usage.output_tokens.text, Some(315));
}

// ════════════════════════════════════════════════════════════════════════════
// Reasoning (top-level reasoning �?reasoning_effort, shared OpenAI behaviour)
// ════════════════════════════════════════════════════════════════════════════

/// TS: top-level `reasoning: 'high'` is forwarded as `reasoning_effort`.
/// (Alibaba's TS maps this to `enable_thinking`; the Rust shared converter
/// maps it to `reasoning_effort` instead.)
#[tokio::test]
async fn top_level_reasoning_maps_to_reasoning_effort() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::High);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["reasoning_effort"], json!("high"));
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate / doStream / errors
// ════════════════════════════════════════════════════════════════════════════

/// TS (alibaba-text fixture): do_generate returns text content.
#[tokio::test]
async fn do_generate_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text } => assert_eq!(text, "Hello, World!"),
        other => panic!("expected Text, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
}

/// TS (alibaba-tool-call fixture): do_generate extracts a tool call.
#[tokio::test]
async fn do_generate_extracts_tool_call() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "qwen-plus",
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
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            assert_eq!(tool_call_id, "call_abc");
            assert_eq!(tool_name, "get-weather");
            assert_eq!(input, &json!({"city": "SF"}));
        }
        other => panic!("expected ToolCall, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// TS (alibaba-text stream fixture): do_stream returns text deltas.
#[tokio::test]
async fn do_stream_returns_text() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"qwen-plus","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"qwen-plus","choices":[{"index":0,"delta":{"content":", World!"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"qwen-plus","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":30,"total_tokens":34}}"#,
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
    let model = provider.model("qwen-plus");

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
    assert_eq!(
        text_deltas,
        vec!["Hello".to_string(), ", World!".to_string()]
    );

    let finish = parts
        .iter()
        .find(|p| matches!(p, StreamPart::Finish { .. }));
    match finish {
        Some(StreamPart::Finish { finish_reason, .. }) => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
        }
        other => panic!("expected Finish, got {:?}", other),
    }
}

/// TS: a 401 response maps to `AiMuxError::Auth`.
#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {
                "message": "Incorrect API key provided",
                "type": "invalid_request_error",
                "param": null,
                "code": "invalid_api_key"
            }
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(ref m)) if m == "Incorrect API key provided"),
        "expected Auth error, got {result:?}"
    );
}

/// TS: a 429 response maps to `AiMuxError::RateLimited`.
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
    let model = provider.model("qwen-plus");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::RateLimited { .. })),
        "expected RateLimited, got {result:?}"
    );
}

/// TS: response headers are exposed on the generate result.
#[tokio::test]
async fn exposes_response_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(text_completion_body()),
        )
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("qwen-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let headers = result
        .response_headers
        .as_ref()
        .expect("response_headers should be Some");
    assert_eq!(headers.get("test-header"), Some(&"test-value".to_string()));
}
