//! The DeepSeek package (`aimux_providers::deepseek`, RFC-0036 section 5).
//!
//! DeepSeek is its own package on the OpenAI-compatible chat model:
//! `deepseek.chat` identity, the `deepseek` providerOptions namespace and
//! metadata key, prompt-cache accounting (`prompt_cache_hit_tokens` becomes the
//! cache-read input tokens), usage in streams requested through
//! `stream_options.include_usage`, `max_tokens`, and JSON-schema response
//! formats. The DeepSeek wire tests (chat, reasoning, tools) live in
//! `deepseek_chat_test.rs` and `deepseek_reasoning_test.rs`; this file covers
//! what the package adds around them and replays the recorded cassettes.

mod common;

use futures::StreamExt;
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::Resolvable;
use aimux_providers::deepseek::{
    DeepSeekProvider, DeepSeekProviderSettings, create_deepseek, deepseek,
};

fn prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
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
    options.provider_options = Some(
        provider_options
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    options
}

#[tokio::test]
async fn the_deepseek_namespace_is_read_and_the_openai_one_is_not() {
    let server = MockServer::start().await;
    mock_json(&server, completion(json!({"prompt_tokens": 1}))).await;
    let model = deepseek_at(&server).chat("deepseek-chat");

    let result = model
        .do_generate(&options_with(json!({
            "deepseek": {"user": "u1", "reasoningEffort": "high", "thinking": {"type": "enabled"}},
            "openai": {"user": "ignored", "store": true},
        })))
        .await
        .unwrap();

    let body = result.request_body.unwrap();
    assert_eq!(body["user"], "u1");
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["thinking"], json!({"type": "enabled"}));
    assert!(body.get("store").is_none());
}

#[tokio::test]
async fn metadata_is_reported_under_deepseek() {
    let server = MockServer::start().await;
    mock_json(&server, completion(json!({"prompt_tokens": 1}))).await;
    let result = deepseek_at(&server)
        .chat("deepseek-chat")
        .do_generate(&CallOptions::new(prompt()))
        .await
        .unwrap();
    assert_eq!(result.provider_metadata.unwrap(), json!({"deepseek": {}}));
}

#[tokio::test]
async fn prompt_cache_hits_become_cache_read_tokens() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        completion(json!({
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "total_tokens": 120,
            "prompt_cache_hit_tokens": 70,
            "prompt_cache_miss_tokens": 30,
            "completion_tokens_details": {"reasoning_tokens": 5}
        })),
    )
    .await;
    let result = deepseek_at(&server)
        .chat("deepseek-chat")
        .do_generate(&CallOptions::new(prompt()))
        .await
        .unwrap();
    let usage = result.usage;
    assert_eq!(usage.input_tokens.total, Some(100));
    assert_eq!(usage.input_tokens.cache_read, Some(70));
    assert_eq!(usage.input_tokens.no_cache, Some(30));
    assert_eq!(usage.output_tokens.total, Some(20));
    assert_eq!(usage.output_tokens.reasoning, Some(5));
    assert_eq!(usage.output_tokens.text, Some(15));
    // The vendor fields survive in the raw object.
    assert_eq!(usage.raw.unwrap()["prompt_cache_hit_tokens"], 70);
}

#[tokio::test]
async fn a_missing_miss_count_is_the_prompt_minus_the_hits() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        completion(
            json!({"prompt_tokens": 100, "completion_tokens": 1, "prompt_cache_hit_tokens": 40}),
        ),
    )
    .await;
    let usage = deepseek_at(&server)
        .chat("deepseek-chat")
        .do_generate(&CallOptions::new(prompt()))
        .await
        .unwrap()
        .usage;
    assert_eq!(usage.input_tokens.cache_read, Some(40));
    assert_eq!(usage.input_tokens.no_cache, Some(60));
}

#[tokio::test]
async fn without_cache_fields_the_standard_usage_applies() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        completion(json!({"prompt_tokens": 12, "completion_tokens": 3})),
    )
    .await;
    let usage = deepseek_at(&server)
        .chat("deepseek-chat")
        .do_generate(&CallOptions::new(prompt()))
        .await
        .unwrap()
        .usage;
    assert_eq!(usage.input_tokens.total, Some(12));
    assert_eq!(usage.input_tokens.cache_read, Some(0));
    assert_eq!(usage.input_tokens.no_cache, Some(12));
}

#[tokio::test]
async fn streaming_requests_usage_and_converts_cache_hits() {
    let server = MockServer::start().await;
    let sse = |json: &str| format!("data: {json}\n\n");
    let body = format!(
        "{}{}{}data: [DONE]\n\n",
        sse(
            r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#
        ),
        sse(r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#),
        sse(
            r#"{"id":"c","model":"m","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":8,"prompt_cache_miss_tokens":2}}"#
        ),
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let result = deepseek_at(&server)
        .chat("deepseek-chat")
        .do_stream(&CallOptions::new(prompt()))
        .await
        .unwrap();
    assert_eq!(
        result.request_body.as_ref().unwrap()["stream_options"],
        json!({"include_usage": true})
    );
    let mut stream = result.stream;
    let mut finish = None;
    while let Some(part) = stream.next().await {
        if let StreamPart::Finish {
            usage,
            provider_metadata,
            ..
        } = part.unwrap()
        {
            finish = Some((usage, provider_metadata));
        }
    }
    let (usage, metadata) = finish.expect("a finish part");
    assert_eq!(usage.input_tokens.cache_read, Some(8));
    assert_eq!(usage.input_tokens.no_cache, Some(2));
    assert_eq!(metadata.unwrap(), json!({"deepseek": {}}));
}

#[test]
fn the_token_limit_is_max_tokens_and_top_k_is_sent() {
    let mut options = CallOptions::new(prompt());
    options.max_output_tokens = Some(64);
    options.top_k = Some(40.0);
    let body = deepseek()
        .chat("deepseek-chat")
        .request_body(&options, false)
        .unwrap()
        .body;
    assert_eq!(body["max_tokens"], 64);
    assert!(body.get("max_completion_tokens").is_none());
    assert_eq!(body["top_k"], 40.0);
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
