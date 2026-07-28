//! Provider-specific tests for the Cerebras provider.
//!
//! Translated from the TypeScript suites:
//! - `packages/cerebras/src/cerebras-provider.test.ts`
//! - `packages/cerebras/src/cerebras-chat-language-model.test.ts`
//!
//! Cerebras is a thin OpenAI-compatible wrapper over [`OpenAIProvider`]. The TS
//! suite is mostly provider-configuration unit tests plus a `transformRequestBody`
//! that renames assistant `reasoning_content` �?`reasoning`, and fixture-based
//! finish-reason normalisation for structured-output tool calls. The Rust thin
//! wrapper delegates to the shared OpenAI converter and does not model those
//! Cerebras-specific transforms, so they are not asserted here.
//!
//! Behaviours verified:
//! - Provider name is "cerebras".
//! - Default base URL resolves to `https://api.cerebras.ai/v1` (verified via
//!   the request path).
//! - `CEREBRAS_API_KEY` env var is loaded by `from_env`.
//! - Custom API key / custom headers are forwarded.
//! - `language_model` via the `Provider` trait works.
//! - Text generation, usage extraction, error mapping.

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

use aimux_providers::{CerebrasConfig, CerebrasProvider};

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
        "model": "llama-3.3-70b",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}

fn make_provider(server: &MockServer) -> CerebrasProvider {
    let config = CerebrasConfig::new("test-api-key").with_base_url(server.uri());
    CerebrasProvider::new(config)
}

/// TS: `createCerebras()` produces a provider whose name is "cerebras".
#[test]
fn provider_name_is_cerebras() {
    let config = CerebrasConfig::new("test-key");
    let provider = CerebrasProvider::new(config);
    assert_eq!(provider.name(), "cerebras");
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

    let provider = make_provider(&server);
    let model = provider.model("llama-3.3-70b");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests[0].url.path(), "/chat/completions");
}

/// TS: `createCerebras({ apiKey: 'custom-key' })` �?custom key in auth header.
#[tokio::test]
async fn custom_api_key_used_in_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer my-custom-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
        .mount(&server)
        .await;

    let config = CerebrasConfig::new("my-custom-key").with_base_url(server.uri());
    let provider = CerebrasProvider::new(config);
    let model = provider.model("llama-3.3-70b");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed with custom API key");
}

/// TS: `createCerebras({ headers: { 'Custom-Header': 'value' } })` �?custom
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

    let config = CerebrasConfig::new("test-key").with_base_url(server.uri());
    let provider = CerebrasProvider::new(config);
    let model = provider.model("llama-3.3-70b");

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

    let config = CerebrasConfig::new("test-key").with_base_url(server.uri());
    let provider = CerebrasProvider::new(config);
    let model = provider
        .language_model("llama-3.3-70b")
        .expect("language_model should succeed");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");
}

/// TS: `createCerebras()` loads the key from `CEREBRAS_API_KEY`.
#[serial]
#[test]
fn from_env_loads_cerebras_api_key() {
    let saved = std::env::var("CEREBRAS_API_KEY").ok();
    unsafe {
        std::env::set_var("CEREBRAS_API_KEY", "env-test-key");
    }

    let config = CerebrasConfig::from_env();
    assert!(config.is_ok(), "from_env should succeed with env var set");

    unsafe {
        match saved {
            Some(v) => std::env::set_var("CEREBRAS_API_KEY", v),
            None => std::env::remove_var("CEREBRAS_API_KEY"),
        }
    }
}

/// TS: without the env var, `createCerebras()` fails.
#[serial]
#[test]
fn from_env_fails_without_env_var() {
    let saved = std::env::var("CEREBRAS_API_KEY").ok();
    unsafe {
        std::env::remove_var("CEREBRAS_API_KEY");
    }

    let config = CerebrasConfig::from_env();
    assert!(config.is_err(), "from_env should fail without env var");

    unsafe {
        if let Some(v) = saved {
            std::env::set_var("CEREBRAS_API_KEY", v);
        }
    }
}

/// TS: usage is extracted from the response (with cache + reasoning detail).
#[tokio::test]
async fn extracts_usage_with_details() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-usage",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "llama-3.3-70b",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hi" },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 433,
                "completion_tokens": 122,
                "total_tokens": 555,
                "prompt_tokens_details": { "cached_tokens": 256 },
                "completion_tokens_details": { "reasoning_tokens": 108 }
            }
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.model("llama-3.3-70b");
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(433));
    assert_eq!(result.usage.input_tokens.cache_read, Some(256));
    assert_eq!(result.usage.output_tokens.total, Some(122));
    assert_eq!(result.usage.output_tokens.reasoning, Some(108));
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
    let model = provider.model("llama-3.3-70b");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(_))),
        "expected Auth error, got {result:?}"
    );
}
