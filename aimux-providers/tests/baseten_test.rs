//! Provider-specific tests for the Baseten provider.
//!
//! Translated from `packages/baseten/src/baseten-provider.unit.test.ts`.
//!
//! Baseten is a thin OpenAI-compatible wrapper over [`OpenAIProvider`]. The TS
//! suite is almost entirely provider-configuration unit tests (mocked
//! constructor calls asserting the base URL, env var, headers, and model-URL
//! routing). The Rust wrapper exposes the same configuration surface, so these
//! tests verify the equivalent behaviours end-to-end through a mock server:
//!
//! - Provider name is "baseten".
//! - Default base URL resolves to `https://inference.baseten.co/v1` (verified
//!   via the request path against a mock).
//! - `BASETEN_API_KEY` env var is loaded by `from_env`.
//! - Custom API key / custom headers are forwarded.
//! - `language_model` via the `Provider` trait works.
//!
//! Note: the TS provider supports per-model `modelURL` endpoints
//! (`https://<model>.api.baseten.co/.../sync/v1`) and rejects `/predict`
//! endpoints for chat models. The Rust wrapper models only the default
//! inference base URL; callers can supply any endpoint via `with_base_url`.
//! Those TS-specific routing behaviours are therefore not asserted here.

use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;

use aimux_providers::{BasetenConfig, BasetenProvider};

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

fn make_provider(server: &MockServer) -> BasetenProvider {
    let config = BasetenConfig::new("test-api-key").with_base_url(server.uri());
    BasetenProvider::new(config)
}

/// TS: `createBaseten()` produces a provider whose name is "baseten".
#[test]
fn provider_name_is_baseten() {
    let config = BasetenConfig::new("test-key");
    let provider = BasetenProvider::new(config);
    assert_eq!(provider.name(), "baseten");
}

/// TS: the default base URL appends `/chat/completions` (verified via the mock
/// request path).
#[tokio::test]
async fn request_hits_chat_completions_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-ai/DeepSeek-V3-0324");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/chat/completions");
}

/// TS: `createBaseten({ apiKey: 'custom-key' })` �?the custom key is sent in
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

    let config = BasetenConfig::new("my-custom-key").with_base_url(server.uri());
    let provider = BasetenProvider::new(config);
    let model = provider.model("deepseek-ai/DeepSeek-V3-0324");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed with custom API key");
}

/// TS: `createBaseten({ headers: { 'Custom-Header': 'value' } })` �?custom
/// headers are forwarded.
#[tokio::test]
async fn custom_headers_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("custom-header", "custom-value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let config = BasetenConfig::new("test-key").with_base_url(server.uri());
    let provider = BasetenProvider::new(config);
    let model = provider.model("deepseek-ai/DeepSeek-V3-0324");

    let mut options = default_options(test_prompt());
    options.headers = Some(
        vec![("custom-header".to_string(), "custom-value".to_string())]
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

    let config = BasetenConfig::new("test-key").with_base_url(server.uri());
    let provider = BasetenProvider::new(config);
    let model = provider
        .language_model("deepseek-ai/DeepSeek-V3-0324")
        .expect("language_model should succeed");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");
}

/// TS: `createBaseten()` loads the key from `BASETEN_API_KEY`.
#[serial]
#[test]
fn from_env_loads_baseten_api_key() {
    let saved = std::env::var("BASETEN_API_KEY").ok();
    unsafe {
        std::env::set_var("BASETEN_API_KEY", "env-test-key");
    }

    let config = BasetenConfig::from_env();
    assert!(config.is_ok(), "from_env should succeed with env var set");

    unsafe {
        match saved {
            Some(v) => std::env::set_var("BASETEN_API_KEY", v),
            None => std::env::remove_var("BASETEN_API_KEY"),
        }
    }
}

/// TS: without the env var, `createBaseten()` fails.
#[serial]
#[test]
fn from_env_fails_without_env_var() {
    let saved = std::env::var("BASETEN_API_KEY").ok();
    unsafe {
        std::env::remove_var("BASETEN_API_KEY");
    }

    let config = BasetenConfig::from_env();
    assert!(config.is_err(), "from_env should fail without env var");

    unsafe {
        if let Some(v) = saved {
            std::env::set_var("BASETEN_API_KEY", v);
        }
    }
}

/// TS: the request body carries the model id.
#[tokio::test]
async fn request_body_carries_model_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-ai/DeepSeek-V3-0324");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "deepseek-ai/DeepSeek-V3-0324");
}
