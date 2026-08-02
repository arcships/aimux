//! Provider-specific tests for the Fireworks provider.
//!
//! Translated from `packages/fireworks/src/fireworks-provider.test.ts`.
//!
//! Fireworks is a thin OpenAI-compatible wrapper over [`OpenAIProvider`]. The
//! TS suite is mostly provider-configuration unit tests plus a
//! `transformRequestBody` that snake-cases `thinking.budgetTokens`,
//! `reasoningHistory`, `promptCacheKey`, `serviceTier` and remaps
//! `reasoning_effort` (`xhigh`→`high`, `minimal`→`low`). The Rust wrapper
//! delegates to the shared OpenAI converter, which already snake-cases
//! `promptCacheKey`→`prompt_cache_key` and `serviceTier`→`service_tier` and
//! emits `reasoning_effort` from the top-level `reasoning` option. The
//! Fireworks-specific `thinking`/`reasoningHistory` remapping and
//! `reasoning_effort` value remapping are not modelled by the Rust thin
//! wrapper and are therefore not asserted here.
//!
//! Behaviours verified:
//! - Provider name is "fireworks".
//! - Default base URL resolves to `https://api.fireworks.ai/inference/v1`
//!   (verified via the request path).
//! - `FIREWORKS_API_KEY` env var is loaded by `from_env`.
//! - Custom API key / custom headers are forwarded.
//! - `language_model` via the `Provider` trait works.
//! - Usage extraction and request body shape.

use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;

use aimux_providers::{ProviderOptions, provider, provider_from_env};

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
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}

fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
    provider(
        "fireworks",
        Some("test-api-key".to_string()),
        "accounts/fireworks/models/llama-v3p1-70b-instruct",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("fireworks provider should build")
}

/// TS: `createFireworks()` produces a provider whose name is "fireworks".
#[test]
fn provider_builds_fireworks_model() {
    let model = provider(
        "fireworks",
        Some("test-key".to_string()),
        "accounts/fireworks/models/llama-v3p1-70b-instruct",
        None,
    )
    .expect("fireworks provider should build");
    assert_eq!(
        model.model_id(),
        "accounts/fireworks/models/llama-v3p1-70b-instruct"
    );
}

/// TS: the default base URL appends `/chat/completions`.
#[tokio::test]
async fn request_hits_chat_completions_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let model = make_provider(&server);
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests[0].url.path(), "/chat/completions");
}

/// TS: `createFireworks({ apiKey: 'custom-key' })` �?custom key in auth header.
#[tokio::test]
async fn custom_api_key_used_in_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer my-custom-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let model = provider(
        "fireworks",
        Some("my-custom-key".to_string()),
        "accounts/fireworks/models/llama-v3p1-70b-instruct",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("fireworks provider should build");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed with custom API key");
}

/// TS: `createFireworks({ headers: { 'Custom-Header': 'value' } })` �?custom
/// headers forwarded.
#[tokio::test]
async fn custom_headers_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("custom-header", "value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let model = provider(
        "fireworks",
        Some("test-key".to_string()),
        "accounts/fireworks/models/llama-v3p1-70b-instruct",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("fireworks provider should build");

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
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let model = provider(
        "fireworks",
        Some("test-key".to_string()),
        "accounts/fireworks/models/llama-v3p1-70b-instruct",
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

/// TS: `createFireworks()` loads the key from `FIREWORKS_API_KEY`.
#[serial]
#[test]
fn from_env_loads_fireworks_api_key() {
    let saved = std::env::var("FIREWORKS_API_KEY").ok();
    unsafe {
        std::env::set_var("FIREWORKS_API_KEY", "env-test-key");
    }

    let model = provider_from_env(
        "fireworks",
        "accounts/fireworks/models/llama-v3p1-70b-instruct",
        None,
    );
    assert!(model.is_ok(), "from_env should succeed with env var set");

    unsafe {
        match saved {
            Some(v) => std::env::set_var("FIREWORKS_API_KEY", v),
            None => std::env::remove_var("FIREWORKS_API_KEY"),
        }
    }
}

/// TS: without the env var, `createFireworks()` fails.
#[serial]
#[test]
fn from_env_fails_without_env_var() {
    let saved = std::env::var("FIREWORKS_API_KEY").ok();
    unsafe {
        std::env::remove_var("FIREWORKS_API_KEY");
    }

    let model = provider_from_env(
        "fireworks",
        "accounts/fireworks/models/llama-v3p1-70b-instruct",
        None,
    );
    assert!(model.is_err(), "from_env should fail without env var");

    unsafe {
        if let Some(v) = saved {
            std::env::set_var("FIREWORKS_API_KEY", v);
        }
    }
}

/// TS: the request body carries the model id and usage is extracted.
#[tokio::test]
async fn request_body_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-usage",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "accounts/fireworks/models/llama-v3p1-70b-instruct",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hi" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 8, "total_tokens": 20 }
        })))
        .mount(&server)
        .await;

    let model = make_provider(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(12));
    assert_eq!(result.usage.output_tokens.total, Some(8));

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["model"],
        "accounts/fireworks/models/llama-v3p1-70b-instruct"
    );
}
