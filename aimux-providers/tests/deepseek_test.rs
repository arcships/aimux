//! The DeepSeek provider (`aimux_providers::deepseek`): `create_deepseek`
//! settings, the per-request key, and the recorded cassette replay. The chat
//! model's wire behaviour (ported from the `@ai-sdk/deepseek` tests) is in
//! `deepseek_chat_test.rs`.

mod common;

use futures::StreamExt;
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::Resolvable;
use aimux_providers::deepseek::{DeepSeekProvider, DeepSeekProviderSettings, create_deepseek};

fn prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

fn deepseek_at(server: &MockServer) -> DeepSeekProvider {
    create_deepseek(DeepSeekProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        ..Default::default()
    })
    .unwrap()
}

fn completion(usage: Value) -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": usage
    })
}

async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn options_with(provider_options: Value) -> CallOptions {
    let mut options = CallOptions::new(prompt());
    options.provider_options = Some(serde_json::from_value(provider_options).unwrap());
    options
}

#[tokio::test]
async fn the_deepseek_namespace_is_read_and_the_openai_one_is_not() {
    let server = MockServer::start().await;
    mock_json(&server, completion(json!({"prompt_tokens": 1}))).await;
    let model = deepseek_at(&server).chat("deepseek-chat");

    let result = model
        .do_generate(&options_with(json!({
            "deepseek": {"userId": "u1", "reasoningEffort": "high", "thinking": {"type": "enabled"}},
            "openai": {"userId": "ignored", "reasoningEffort": "low"},
        })))
        .await
        .unwrap();

    let body = result.request_body.unwrap();
    // upstream: providerOptions are read from the provider name's namespace only.
    assert_eq!(body["user_id"], "u1");
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["thinking"], json!({"type": "enabled"}));
}

#[serial]
#[tokio::test]
async fn the_key_is_read_from_the_environment_per_request_not_at_creation() {
    let saved = std::env::var("DEEPSEEK_API_KEY").ok();
    unsafe { std::env::remove_var("DEEPSEEK_API_KEY") };
    let server = MockServer::start().await;
    mock_json(&server, completion(json!({"prompt_tokens": 1}))).await;
    let model = create_deepseek(DeepSeekProviderSettings {
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .expect("no key is read at creation")
    .chat("deepseek-chat");

    let error = model
        .do_generate(&CallOptions::new(prompt()))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AiMuxError::LoadApiKey { env_var, .. } if env_var == "DEEPSEEK_API_KEY"),
        "{error:?}"
    );

    unsafe { std::env::set_var("DEEPSEEK_API_KEY", "env-key") };
    model
        .do_generate(&CallOptions::new(prompt()))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer env-key"
    );
    unsafe {
        match saved {
            Some(v) => std::env::set_var("DEEPSEEK_API_KEY", v),
            None => std::env::remove_var("DEEPSEEK_API_KEY"),
        }
    }
}

/// The recorded DeepSeek exchanges (`tests/cassettes/deepseek`) replayed
/// through the package.
#[tokio::test]
async fn recorded_deepseek_cassettes_replay_through_the_package() {
    let server = MockServer::start().await;
    let n = common::replay::mount_cassettes(&server, "tests/cassettes/deepseek").await;
    assert!(n > 0, "no deepseek cassettes");
    let model = deepseek_at(&server).chat("deepseek-chat");

    let result = generate_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed with cassette replay");
    assert!(!result.text.is_empty() || !result.tool_calls.is_empty());
    assert!(result.usage.input_tokens.total.is_some());

    let result = stream_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("stream_text should succeed");
    let mut stream = result.stream;
    let mut finished = false;
    while let Some(part) = stream.next().await {
        if let StreamPart::Finish { .. } = part.expect("stream part") {
            finished = true;
        }
    }
    assert!(finished, "stream should finish");
}
