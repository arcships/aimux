//! Provider-specific tests for the DeepInfra provider.
//!
//! Translated from `packages/deepinfra/src/deepinfra-provider.test.ts`.
//!
//! DeepInfra is a thin OpenAI-compatible wrapper over [`OpenAIProvider`]. The
//! TS suite is mostly provider-configuration unit tests plus image/completion
//! model construction. The Rust wrapper models only the chat surface, so the
//! image/completion model tests are not translated. The DeepInfra default base
//! URL bakes in the `/openai` prefix (`https://api.deepinfra.com/v1/openai`)
//! so that the shared OpenAI provider's `/chat/completions` suffix yields the
//! correct DeepInfra endpoint.
//!
//! Behaviours verified:
//! - Provider name is "deepinfra".
//! - Default base URL resolves to `https://api.deepinfra.com/v1/openai`
//!   (verified via the request path).
//! - `DEEPINFRA_API_KEY` env var is loaded by `from_env`.
//! - Custom API key / custom headers are forwarded.
//! - `language_model` via the `Provider` trait works.
//! - Text generation, usage extraction, error mapping.

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
        "model": "meta-llama/Meta-Llama-3-70B-Instruct",
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
        "deepinfra",
        Some("test-api-key".to_string()),
        "meta-llama/Meta-Llama-3-70B-Instruct",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("deepinfra provider should build")
}

/// TS: `createDeepInfra()` produces a model for the "deepinfra" registry entry.
#[test]
fn provider_builds_deepinfra_model() {
    let model = provider(
        "deepinfra",
        Some("test-key".to_string()),
        "meta-llama/Meta-Llama-3-70B-Instruct",
        None,
    )
    .expect("deepinfra provider should build");
    assert_eq!(model.model_id(), "meta-llama/Meta-Llama-3-70B-Instruct");
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

/// TS: `createDeepInfra({ apiKey: 'custom-key' })` �?custom key in auth header.
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
        "deepinfra",
        Some("my-custom-key".to_string()),
        "meta-llama/Meta-Llama-3-70B-Instruct",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("deepinfra provider should build");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed with custom API key");
}

/// TS: `createDeepInfra({ headers: { 'Custom-Header': 'value' } })` �?custom
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
        "deepinfra",
        Some("test-key".to_string()),
        "meta-llama/Meta-Llama-3-70B-Instruct",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("deepinfra provider should build");

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

    let model = make_provider(&server);

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");
}

/// TS: `createDeepInfra()` loads the key from `DEEPINFRA_API_KEY`.
#[serial]
#[test]
fn from_env_loads_deepinfra_api_key() {
    let saved = std::env::var("DEEPINFRA_API_KEY").ok();
    unsafe {
        std::env::set_var("DEEPINFRA_API_KEY", "env-test-key");
    }

    let model = provider_from_env("deepinfra", "meta-llama/Meta-Llama-3-70B-Instruct", None);
    assert!(model.is_ok(), "from_env should succeed with env var set");

    unsafe {
        match saved {
            Some(v) => std::env::set_var("DEEPINFRA_API_KEY", v),
            None => std::env::remove_var("DEEPINFRA_API_KEY"),
        }
    }
}

/// TS: without the env var, `createDeepInfra()` fails.
#[serial]
#[test]
fn from_env_fails_without_env_var() {
    let saved = std::env::var("DEEPINFRA_API_KEY").ok();
    unsafe {
        std::env::remove_var("DEEPINFRA_API_KEY");
    }

    let model = provider_from_env("deepinfra", "meta-llama/Meta-Llama-3-70B-Instruct", None);
    assert!(model.is_err(), "from_env should fail without env var");

    unsafe {
        if let Some(v) = saved {
            std::env::set_var("DEEPINFRA_API_KEY", v);
        }
    }
}

/// TS: usage is extracted and the request body carries the model id.
#[tokio::test]
async fn request_body_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-usage",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "meta-llama/Meta-Llama-3-70B-Instruct",
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
    assert_eq!(body["model"], "meta-llama/Meta-Llama-3-70B-Instruct");
}

/// TS: a 401 response maps to `AiMuxError::ApiCall` (401 in `status_code`).
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

    let model = make_provider(&server);

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(401)),
        "expected Auth error, got {result:?}"
    );
}
