//! Provider-specific tests for the Perplexity provider.
//!
//! Translated from the TypeScript suites:
//! - `packages/perplexity/src/perplexity-language-model.test.ts`
//! - `packages/perplexity/src/convert-to-perplexity-messages.test.ts`
//!
//! Perplexity is a thin OpenAI-compatible wrapper over [`OpenAIProvider`]. The
//! behaviours verified here are the ones the wrapper is responsible for:
//!
//! - Provider configuration: name, `PERPLEXITY_API_KEY` env var, custom API
//!   key, custom headers, base-URL override.
//! - Request body shape: `model` + `messages` (string content for plain text).
//! - Usage extraction (`prompt_tokens` / `completion_tokens`).
//! - Text generation, streaming, error mapping, response headers.
//!
//! Note: the TS Perplexity provider adds provider-specific behaviour not
//! modelled by the Rust thin wrapper �?`search_recency_filter` / `return_images`
//! provider-option passthrough, top-level `citations`/`images` extraction into
//! `providerMetadata`, and extended usage (`citation_tokens`, `num_search_queries`).
//! Those TS-specific behaviours are not asserted here because the Rust wrapper
//! delegates to the shared OpenAI converter which does not implement them.

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
use aimux_core::types::FinishReasonUnified;

use aimux_providers::{PerplexityConfig, PerplexityProvider};

// ── shared helpers ───────────────────────────────────────────────────────────

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

fn text_completion_body() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "sonar",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 11, "total_tokens": 403, "completion_tokens": 392 }
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

fn make_provider(server: &MockServer) -> PerplexityProvider {
    let config = PerplexityConfig::new("test-api-key").with_base_url(server.uri());
    PerplexityProvider::new(config)
}

// ════════════════════════════════════════════════════════════════════════════
// Provider configuration
// ════════════════════════════════════════════════════════════════════════════

/// TS: `createPerplexity()` produces a provider whose name is "perplexity".
#[test]
fn provider_name_is_perplexity() {
    let config = PerplexityConfig::new("test-key");
    let provider = PerplexityProvider::new(config);
    assert_eq!(provider.name(), "perplexity");
}

/// TS: custom API key is sent in the `Authorization: Bearer` header.
#[tokio::test]
async fn custom_api_key_used_in_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer my-custom-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let config = PerplexityConfig::new("my-custom-key").with_base_url(server.uri());
    let provider = PerplexityProvider::new(config);
    let model = provider.model("sonar");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed with custom API key");
}

/// TS: custom headers are forwarded to the HTTP request.
#[tokio::test]
async fn custom_headers_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("custom-provider-header", "provider-header-value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let config = PerplexityConfig::new("test-key").with_base_url(server.uri());
    let provider = PerplexityProvider::new(config);
    let model = provider.model("sonar");

    let mut options = default_options(test_prompt());
    options.headers = Some(
        vec![(
            "custom-provider-header".to_string(),
            "provider-header-value".to_string(),
        )]
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

    let config = PerplexityConfig::new("test-key").with_base_url(server.uri());
    let provider = PerplexityProvider::new(config);
    let model = provider
        .language_model("sonar")
        .expect("language_model should succeed");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");
}

/// TS: `createPerplexity()` loads the key from `PERPLEXITY_API_KEY`.
#[serial]
#[test]
fn from_env_loads_perplexity_api_key() {
    let saved = std::env::var("PERPLEXITY_API_KEY").ok();
    unsafe {
        std::env::set_var("PERPLEXITY_API_KEY", "env-test-key");
    }

    let config = PerplexityConfig::from_env();
    assert!(config.is_ok(), "from_env should succeed with env var set");

    unsafe {
        match saved {
            Some(v) => std::env::set_var("PERPLEXITY_API_KEY", v),
            None => std::env::remove_var("PERPLEXITY_API_KEY"),
        }
    }
}

/// TS: without the env var, `createPerplexity()` fails.
#[serial]
#[test]
fn from_env_fails_without_env_var() {
    let saved = std::env::var("PERPLEXITY_API_KEY").ok();
    unsafe {
        std::env::remove_var("PERPLEXITY_API_KEY");
    }

    let config = PerplexityConfig::from_env();
    assert!(config.is_err(), "from_env should fail without env var");

    unsafe {
        if let Some(v) = saved {
            std::env::set_var("PERPLEXITY_API_KEY", v);
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Request body & message conversion (perplexity-language-model.test.ts)
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should send correct request body" �?the body carries the model id and
/// a single user message with string content.
#[tokio::test]
async fn sends_correct_request_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("sonar");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "sonar");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "Hello");
}

/// TS: "should send correct streaming request body" �?stream requests add
/// `stream: true`.
#[tokio::test]
async fn stream_request_body_has_stream_flag() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"sonar","choices":[{"index":0,"delta":{"role":"assistant","content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"sonar","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
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
    let model = provider.model("sonar");
    let _ = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let req_body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(req_body["stream"], json!(true));
    assert_eq!(req_body["model"], "sonar");
}

// ════════════════════════════════════════════════════════════════════════════
// Usage extraction
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should extract usage" �?prompt/completion tokens are surfaced.
#[tokio::test]
async fn extracts_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("sonar");
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(11));
    assert_eq!(result.usage.output_tokens.total, Some(392));
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate / doStream / errors / headers
// ════════════════════════════════════════════════════════════════════════════

/// TS (perplexity-text fixture): do_generate returns text content.
#[tokio::test]
async fn do_generate_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("sonar");

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

/// TS: do_stream returns text deltas.
#[tokio::test]
async fn do_stream_returns_text() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"sonar","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"sonar","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"sonar","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
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
    let model = provider.model("sonar");

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

/// TS: a 401 response maps to `AiMuxError::Auth`.
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
    let model = provider.model("sonar");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(ref m)) if m == "Invalid API key"),
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
    let model = provider.model("sonar");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::RateLimited { .. })),
        "expected RateLimited, got {result:?}"
    );
}

/// TS: "should expose the raw response headers".
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
    let model = provider.model("sonar");

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
