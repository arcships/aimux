//! Provider-specific tests for the Moonshot AI (Kimi) provider.
//!
//! Translated from the TypeScript suites:
//! - `packages/moonshotai/src/moonshotai-provider.test.ts`
//! - `packages/moonshotai/src/convert-moonshotai-chat-usage.test.ts`
//!
//! Moonshot AI is a thin OpenAI-compatible wrapper over [`OpenAIProvider`].
//! The TS suite has a custom usage converter that recognises a **top-level**
//! `cached_tokens` field (Moonshot's wire format) and prioritises it over the
//! nested `prompt_tokens_details.cached_tokens`. The shared OpenAI
//! `convert_usage` now does the same (preferring the top-level field), so these
//! usage-conversion tests are translated as HTTP-mocked round-trips through the
//! provider.
//!
//! Note: the TS provider also installs a `transformRequestBody` that
//! snake-cases `thinking.budgetTokens`/`reasoningHistory`. The Rust thin wrapper
//! does not model that and delegates to the shared OpenAI converter, so those
//! TS-specific transforms are not asserted here.

use futures::StreamExt;
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::{provider, provider_from_env, ProviderOptions};

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

/// Build a completion body with a custom `usage` object.
fn body_with_usage(usage: Value) -> Value {
    json!({
        "id": "chatcmpl-usage",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "moonshot-v1-8k",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hi" },
            "finish_reason": "stop"
        }],
        "usage": usage
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

fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
    provider(
        "moonshotai",
        Some("test-api-key".to_string()),
        "moonshot-v1-8k",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("moonshotai provider should build")
}

/// Run a do_generate against a mock returning the given usage, returning the
/// parsed `Usage`.
async fn usage_for(usage: Value) -> aimux_core::types::Usage {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body_with_usage(usage)))
        .mount(&server)
        .await;

    let model = make_provider(&server);
    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed")
        .usage
}

// ════════════════════════════════════════════════════════════════════════════
// Provider configuration (moonshotai-provider.test.ts)
// ════════════════════════════════════════════════════════════════════════════

/// TS: `createMoonshotAI()` produces a provider whose name is "moonshotai".
#[test]
fn provider_builds_moonshotai_model() {
    let model = provider(
        "moonshotai",
        Some("test-key".to_string()),
        "moonshot-v1-8k",
        None,
    )
    .expect("moonshotai provider should build");
    assert_eq!(model.model_id(), "moonshot-v1-8k");
}

/// TS: custom API key is sent in the `Authorization: Bearer` header.
#[tokio::test]
async fn custom_api_key_used_in_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer my-custom-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(body_with_usage(json!({
                "prompt_tokens": 4, "completion_tokens": 30, "total_tokens": 34
            }))),
        )
        .mount(&server)
        .await;

    let model = provider(
        "moonshotai",
        Some("my-custom-key".to_string()),
        "moonshot-v1-8k",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("moonshotai provider should build");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed with custom API key");
}

/// TS: custom headers are forwarded.
#[tokio::test]
async fn custom_headers_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("custom-header", "value"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(body_with_usage(json!({
                "prompt_tokens": 4, "completion_tokens": 30, "total_tokens": 34
            }))),
        )
        .mount(&server)
        .await;

    let model = provider(
        "moonshotai",
        Some("test-key".to_string()),
        "moonshot-v1-8k",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("moonshotai provider should build");

    let mut options = default_options(test_prompt());
    options.headers = Some(
        vec![("custom-header".to_string(), "value".to_string())]
            .into_iter()
            .collect(),
    );

    model
        .do_generate(&options)
        .await
        .expect("should succeed with custom headers");
}

/// TS: `provider.languageModel(modelId)` via the `Provider` trait.
#[tokio::test]
async fn language_model_via_trait() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(body_with_usage(json!({
                "prompt_tokens": 4, "completion_tokens": 30, "total_tokens": 34
            }))),
        )
        .mount(&server)
        .await;

    let model = provider(
        "moonshotai",
        Some("test-key".to_string()),
        "moonshot-v1-8k",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("provider should build a model");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");
}

/// TS: `createMoonshotAI()` loads the key from `MOONSHOT_API_KEY`.
#[serial]
#[test]
fn from_env_loads_moonshot_api_key() {
    let saved = std::env::var("MOONSHOT_API_KEY").ok();
    unsafe {
        std::env::set_var("MOONSHOT_API_KEY", "env-test-key");
    }

    let model = provider_from_env("moonshotai", "moonshot-v1-8k", None);
    assert!(model.is_ok(), "from_env should succeed with env var set");

    unsafe {
        match saved {
            Some(v) => std::env::set_var("MOONSHOT_API_KEY", v),
            None => std::env::remove_var("MOONSHOT_API_KEY"),
        }
    }
}

/// TS: without the env var, `createMoonshotAI()` fails.
#[serial]
#[test]
fn from_env_fails_without_env_var() {
    let saved = std::env::var("MOONSHOT_API_KEY").ok();
    unsafe {
        std::env::remove_var("MOONSHOT_API_KEY");
    }

    let model = provider_from_env("moonshotai", "moonshot-v1-8k", None);
    assert!(model.is_err(), "from_env should fail without env var");

    unsafe {
        if let Some(v) = saved {
            std::env::set_var("MOONSHOT_API_KEY", v);
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Usage conversion (convert-moonshotai-chat-usage.test.ts)
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should convert basic usage without caching or reasoning".
#[tokio::test]
async fn converts_basic_usage() {
    let usage = usage_for(json!({ "prompt_tokens": 100, "completion_tokens": 50 })).await;
    assert_eq!(usage.input_tokens.total, Some(100));
    assert_eq!(usage.input_tokens.no_cache, Some(100));
    assert_eq!(usage.input_tokens.cache_read, Some(0));
    assert_eq!(usage.output_tokens.total, Some(50));
    assert_eq!(usage.output_tokens.text, Some(50));
    assert_eq!(usage.output_tokens.reasoning, Some(0));
}

/// TS: "should convert usage with top-level cached_tokens (Moonshot format)".
#[tokio::test]
async fn converts_top_level_cached_tokens() {
    let usage = usage_for(json!({
        "prompt_tokens": 100, "completion_tokens": 50, "cached_tokens": 30
    }))
    .await;
    assert_eq!(usage.input_tokens.total, Some(100));
    assert_eq!(usage.input_tokens.cache_read, Some(30));
    assert_eq!(usage.input_tokens.no_cache, Some(70));
    assert_eq!(usage.output_tokens.total, Some(50));
    assert_eq!(usage.output_tokens.text, Some(50));
}

/// TS: "should convert usage with nested cached_tokens (OpenAI format)".
#[tokio::test]
async fn converts_nested_cached_tokens() {
    let usage = usage_for(json!({
        "prompt_tokens": 100,
        "completion_tokens": 50,
        "prompt_tokens_details": { "cached_tokens": 25 }
    }))
    .await;
    assert_eq!(usage.input_tokens.cache_read, Some(25));
    assert_eq!(usage.input_tokens.no_cache, Some(75));
}

/// TS: "should prioritize top-level cached_tokens over nested".
#[tokio::test]
async fn prioritizes_top_level_cached_tokens() {
    let usage = usage_for(json!({
        "prompt_tokens": 100,
        "completion_tokens": 50,
        "cached_tokens": 40,
        "prompt_tokens_details": { "cached_tokens": 25 }
    }))
    .await;
    assert_eq!(usage.input_tokens.cache_read, Some(40));
    assert_eq!(usage.input_tokens.no_cache, Some(60));
}

/// TS: "should convert usage with reasoning tokens".
#[tokio::test]
async fn converts_reasoning_tokens() {
    let usage = usage_for(json!({
        "prompt_tokens": 100,
        "completion_tokens": 80,
        "completion_tokens_details": { "reasoning_tokens": 30 }
    }))
    .await;
    assert_eq!(usage.output_tokens.total, Some(80));
    assert_eq!(usage.output_tokens.reasoning, Some(30));
    assert_eq!(usage.output_tokens.text, Some(50));
}

/// TS: "should convert usage with both cached and reasoning tokens".
#[tokio::test]
async fn converts_cached_and_reasoning_tokens() {
    let usage = usage_for(json!({
        "prompt_tokens": 100,
        "completion_tokens": 80,
        "cached_tokens": 35,
        "completion_tokens_details": { "reasoning_tokens": 30 }
    }))
    .await;
    assert_eq!(usage.input_tokens.cache_read, Some(35));
    assert_eq!(usage.input_tokens.no_cache, Some(65));
    assert_eq!(usage.output_tokens.reasoning, Some(30));
    assert_eq!(usage.output_tokens.text, Some(50));
}

/// TS: "should handle null values in usage fields".
#[tokio::test]
async fn handles_null_usage_field_values() {
    let usage = usage_for(json!({
        "prompt_tokens": null,
        "completion_tokens": null,
        "cached_tokens": null
    }))
    .await;
    assert_eq!(usage.input_tokens.total, Some(0));
    assert_eq!(usage.input_tokens.no_cache, Some(0));
    assert_eq!(usage.input_tokens.cache_read, Some(0));
    assert_eq!(usage.output_tokens.total, Some(0));
    assert_eq!(usage.output_tokens.text, Some(0));
    assert_eq!(usage.output_tokens.reasoning, Some(0));
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate / doStream
// ════════════════════════════════════════════════════════════════════════════

/// TS: do_generate returns text content.
#[tokio::test]
async fn do_generate_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(body_with_usage(json!({
                "prompt_tokens": 4, "completion_tokens": 30, "total_tokens": 34
            }))),
        )
        .mount(&server)
        .await;

    let model = make_provider(&server);

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Hi"),
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
            r#"{"id":"chatcmpl-1","model":"moonshot-v1-8k","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"moonshot-v1-8k","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":30,"total_tokens":34}}"#,
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

    let model = make_provider(&server);

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
    assert_eq!(text_deltas, vec!["Hello".to_string()]);
}
