//! Wiremock tests for the Mistral provider.
//!
//! Translated from `packages/mistral/src/mistral-chat-language-model.test.ts`,
//! focusing on the cases that the Rust data model can express:
//! - doGenerate: text extraction, usage, tool calls, request body
//! - doStream: text streaming, tool call streaming, request body

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, Tool, ToolChoice};
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::{MistralConfig, MistralProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

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

fn test_tool() -> Tool {
    Tool::Function(FunctionTool {
        name: "test-tool".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    })
}

async fn mock_json_response(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_sse_response(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Build an SSE body from raw JSON chunk strings — each becomes
/// `data: <json>\n\n`. Easier than hand-writing `data:` prefixes with the
/// required blank-line terminator.
fn sse_json_body(chunks: &[&str]) -> String {
    let mut body = String::new();
    for c in chunks {
        body.push_str(&format!("data: {}\n\n", c));
    }
    body.push_str("data: [DONE]\n\n");
    body
}

async fn collect_stream(result: StreamResult) -> Vec<StreamPart> {
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

fn text_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should extract text content"
#[tokio::test]
async fn should_extract_text_response() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "5319bd0299614c679a0068a4f2c8ffd0",
            "created": 1769088720,
            "model": "mistral-small-latest",
            "usage": {
                "prompt_tokens": 13,
                "total_tokens": 447,
                "completion_tokens": 434
            },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "tool_calls": null,
                    "content": "Hello, World!"
                }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Hello, World!"),
        other => panic!("expected Text, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("stop"));
    assert_eq!(
        result.response.id.as_deref(),
        Some("5319bd0299614c679a0068a4f2c8ffd0")
    );
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("mistral-small-latest")
    );
}

/// TS: "should extract usage"
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "created": 1711115037,
            "model": "mistral-small-latest",
            "usage": {
                "prompt_tokens": 13,
                "total_tokens": 447,
                "completion_tokens": 434
            },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "Hello"
                }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(13));
    assert_eq!(result.usage.input_tokens.no_cache, Some(13));
    assert_eq!(result.usage.input_tokens.cache_read, None);
    assert_eq!(result.usage.output_tokens.total, Some(434));
}

/// TS: "should extract usage with cached tokens"
#[tokio::test]
async fn should_extract_usage_with_cached_tokens() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": {
                "prompt_tokens": 100,
                "total_tokens": 200,
                "completion_tokens": 100,
                "num_cached_tokens": 30
            },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "Hi" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(100));
    assert_eq!(result.usage.input_tokens.no_cache, Some(70));
    assert_eq!(result.usage.input_tokens.cache_read, Some(30));
}

/// TS: "should extract tool call content"
#[tokio::test]
async fn should_extract_tool_call() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "b3999b8c93e04e11bcbff7bcab829667",
            "created": 1769088854,
            "model": "mistral-small-latest",
            "usage": {
                "prompt_tokens": 124,
                "total_tokens": 146,
                "completion_tokens": 22
            },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "gSIMJiOkT",
                        "function": {
                            "name": "weather",
                            "arguments": "{\"location\": \"San Francisco\"}"
                        }
                    }]
                }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

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
            assert_eq!(tool_call_id, "gSIMJiOkT");
            assert_eq!(tool_name, "weather");
            assert_eq!(input, &json!({"location": "San Francisco"}));
        }
        other => panic!("expected ToolCall, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// TS: "should send correct request body"
#[tokio::test]
async fn should_send_correct_request_body() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "Hello" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    // Verify the request body.
    let request_body = result.request_body.expect("request body should be set");
    assert_eq!(request_body["model"], "mistral-small-latest");
    assert_eq!(
        request_body["messages"],
        json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
    );
    // No extra keys should be present.
    assert!(request_body.get("max_tokens").is_none());
    assert!(request_body.get("temperature").is_none());
    assert!(request_body.get("tools").is_none());
}

/// TS: "should pass tools and toolChoice" — Mistral maps `required` → `"any"`.
#[tokio::test]
async fn should_pass_tools_with_tool_choice_required() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let options = CallOptions {
        prompt: test_prompt(),
        tools: Some(vec![test_tool()]),
        tool_choice: ToolChoice::Required,
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["tool_choice"],
        json!("any"),
        "Mistral uses 'any' for required tool choice"
    );
    assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
}

/// TS: "should pass tools and toolChoice" — `tool` choice filters tools.
#[tokio::test]
async fn should_pass_tools_with_tool_choice_tool() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let options = CallOptions {
        prompt: test_prompt(),
        tools: Some(vec![test_tool()]),
        tool_choice: ToolChoice::Tool {
            tool_name: "test-tool".to_string(),
        },
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["tool_choice"], json!("any"));
    assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
}

/// TS: "should forward stopSequences as the Mistral stop parameter"
#[tokio::test]
async fn should_forward_stop_sequences() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let options = CallOptions {
        prompt: test_prompt(),
        stop_sequences: Some(vec!["foo".to_string(), "bar".to_string()]),
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["stop"], json!(["foo", "bar"]));
}

/// TS: "should map model_length finish reason to length"
#[tokio::test]
async fn should_map_model_length_finish_reason() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "model_length",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Length);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("model_length"));
}

// ════════════════════════════════════════════════════════════════════════════
// doStream
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should stream text"
#[tokio::test]
async fn should_stream_text() {
    let server = MockServer::start().await;
    let sse = sse_body(&[
        r#"data: {"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null,"logprobs":null}]}

"#,
        r#"data: {"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null,"logprobs":null}]}

"#,
        r#"data: {"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":", world!"},"finish_reason":null,"logprobs":null}]}

"#,
        r#"data: {"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":""},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":13,"total_tokens":21,"completion_tokens":8}}

"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;

    // StreamStart, ResponseMetadata, TextStart, TextDelta x2, TextEnd, Finish.
    assert!(matches!(
        parts.first(),
        Some(StreamPart::StreamStart { .. })
    ));

    let deltas = text_deltas(&parts);
    assert_eq!(deltas, vec!["Hello", ", world!"]);

    // Check finish reason and usage.
    let finish = parts.last().expect("should have finish");
    match finish {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.input_tokens.total, Some(13));
            assert_eq!(usage.output_tokens.total, Some(8));
        }
        other => panic!("expected Finish, got {:?}", other),
    }
}

/// TS: "should stream tool call"
#[tokio::test]
async fn should_stream_tool_call() {
    let server = MockServer::start().await;
    let sse = sse_body(&[
        r#"data: {"id":"b3999b8c93e04e11bcbff7bcab829667","object":"chat.completion.chunk","created":1769088854,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null,"logprobs":null}]}

"#,
        r#"data: {"id":"b3999b8c93e04e11bcbff7bcab829667","object":"chat.completion.chunk","created":1769088854,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":null,"tool_calls":[{"id":"gSIMJiOkT","function":{"name":"weather","arguments":"{\"location\": \"San Francisco\"}"}}]},"finish_reason":"tool_calls","logprobs":null}],"usage":{"prompt_tokens":124,"total_tokens":146,"completion_tokens":22}}

"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;

    // Find the ToolCall part.
    let tool_call = parts.iter().find_map(|p| match p {
        StreamPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => Some((tool_call_id, tool_name, input)),
        _ => None,
    });
    let (id, name, input) = tool_call.expect("should have a ToolCall");
    assert_eq!(id, "gSIMJiOkT");
    assert_eq!(name, "weather");
    assert_eq!(input, &json!({"location": "San Francisco"}));

    // Should also have ToolInputStart, ToolInputDelta, ToolInputEnd.
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolInputStart { .. }))
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolInputDelta { .. }))
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolInputEnd { .. }))
    );
}

/// TS: "should pass the messages" (streaming request body).
#[tokio::test]
async fn should_send_streaming_request_body() {
    let server = MockServer::start().await;
    mock_sse_response(
        &server,
        &sse_body(&[
            r#"data: {"id":"test","model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"total_tokens":2,"completion_tokens":1}}

"#,
        ]),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let body = result.request_body.expect("body");
    assert_eq!(body["model"], "mistral-small-latest");
    assert_eq!(body["stream"], json!(true));
    assert_eq!(
        body["messages"],
        json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
    );
    // No stream_options (Mistral doesn't use it).
    assert!(body.get("stream_options").is_none());
}

/// TS: "should stream text with content objects" (array content format)
#[tokio::test]
async fn should_stream_text_with_array_content() {
    let server = MockServer::start().await;
    let sse = sse_body(&[
        r#"data: {"id":"b9e43f82d6c74a1e9f5b2c8e7a9d4f6b","object":"chat.completion.chunk","created":1750538500,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"text","text":""}]},"finish_reason":null,"logprobs":null}]}

"#,
        r#"data: {"id":"b9e43f82d6c74a1e9f5b2c8e7a9d4f6b","object":"chat.completion.chunk","created":1750538500,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":[{"type":"text","text":"Hello"}]},"finish_reason":null,"logprobs":null}]}

"#,
        r#"data: {"id":"b9e43f82d6c74a1e9f5b2c8e7a9d4f6b","object":"chat.completion.chunk","created":1750538500,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":[{"type":"text","text":", world!"}]},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":4,"total_tokens":36,"completion_tokens":32}}

"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;
    let deltas = text_deltas(&parts);
    assert_eq!(deltas, vec!["Hello", ", world!"]);
}

/// TS: "should handle 401 auth error"
#[tokio::test]
async fn should_handle_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({ "message": "Invalid API key", "type": "auth_error" })),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model.do_generate(&default_options(test_prompt())).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.status_code(), Some(401), "got {err:?}");
    assert!(err.to_string().contains("Invalid API key"));
}

// ════════════════════════════════════════════════════════════════════════════
// Additional doGenerate cases — request construction, headers, penalties,
// response_format, parallel_tool_calls, content-shape variants, errors.
// ════════════════════════════════════════════════════════════════════════════

/// A minimal "ok" chat-completion body reused by request-body assertions.
fn ok_text_body() -> Value {
    json!({
        "id": "test-id",
        "model": "mistral-small-latest",
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "message": { "role": "assistant", "content": "ok" }
        }]
    })
}

/// TS: "should forward presencePenalty and frequencyPenalty without unsupported warnings"
#[tokio::test]
async fn should_forward_presence_and_frequency_penalty() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_text_body()).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let options = CallOptions {
        prompt: test_prompt(),
        presence_penalty: Some(0.1),
        frequency_penalty: Some(0.2),
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    // f32 round-trip: compare with tolerance rather than exact equality.
    let pp = body["presence_penalty"].as_f64().unwrap();
    let fp = body["frequency_penalty"].as_f64().unwrap();
    assert!((pp - 0.1).abs() < 1e-6, "presence_penalty={pp}");
    assert!((fp - 0.2).abs() < 1e-6, "frequency_penalty={fp}");
}

/// TS: "should pass headers" — request-level custom headers reach the server.
#[tokio::test]
async fn should_pass_request_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(wiremock::matchers::header(
            "custom-request-header",
            "request-header-value",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_text_body()))
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );

    let options = CallOptions {
        prompt: test_prompt(),
        headers: Some(headers),
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await;
    assert!(result.is_ok(), "request should match the header mock");
}

/// TS: "should expose the raw response headers"
#[tokio::test]
async fn should_expose_raw_response_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(ok_text_body()),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

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

/// TS: "should send additional response information" (id, modelId)
#[tokio::test]
async fn should_send_additional_response_information() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_text_body()).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.response.id.as_deref(), Some("test-id"));
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("mistral-small-latest")
    );
}

/// TS: "should inject JSON instruction for JSON response format" — Rust sets
/// `response_format: { type: "json_object" }` (no schema). The Rust impl does
/// not inject a system message, so we assert only the response_format field.
#[tokio::test]
async fn should_set_json_object_response_format() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_text_body()).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let options = CallOptions {
        prompt: test_prompt(),
        response_format: Some(aimux_core::options::ResponseFormat::Json {
            schema: None,
            name: None,
            description: None,
        }),
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["response_format"], json!({ "type": "json_object" }));
}

/// TS: "should inject JSON instruction for JSON response format with schema"
#[tokio::test]
async fn should_set_json_schema_response_format() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_text_body()).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let schema = json!({
        "type": "object",
        "properties": { "name": { "type": "string" } }
    });
    let options = CallOptions {
        prompt: test_prompt(),
        response_format: Some(aimux_core::options::ResponseFormat::Json {
            schema: Some(schema.clone()),
            name: None,
            description: None,
        }),
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["response_format"]["type"], json!("json_schema"));
    assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
    assert_eq!(
        body["response_format"]["json_schema"]["name"],
        json!("response")
    );
    assert_eq!(
        body["response_format"]["json_schema"]["strict"],
        json!(false)
    );
}

/// TS: "should avoid duplication when trailing assistant message" — continuation
/// mode sets `prefix: true` on the trailing assistant message.
#[tokio::test]
async fn should_mark_trailing_assistant_message_as_prefix() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_text_body()).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hello")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::text("prefix ")],
            ..Default::default()
        },
    ];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");
    let body = result.request_body.expect("body");
    let messages = body["messages"].as_array().expect("messages array");
    let last = messages.last().expect("last message");
    assert_eq!(last["role"], json!("assistant"));
    assert_eq!(last["content"], json!("prefix "));
    assert_eq!(last["prefix"], json!(true));
}

/// TS: "should extract content when message content is a content object"
#[tokio::test]
async fn should_extract_content_when_message_content_is_object() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "object": "chat.completion",
            "id": "object-id",
            "created": 1711113008,
            "model": "mistral-small-latest",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Hello from object" }],
                    "tool_calls": null
                },
                "finish_reason": "stop",
                "logprobs": null
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Hello from object"),
        other => panic!("expected Text, got {:?}", other),
    }
}

/// TS: "should preserve ordering of mixed thinking and text" — the Rust
/// `GenerateContent` enum has no reasoning variant, so thinking parts are
/// skipped and only text parts are surfaced.
#[tokio::test]
async fn should_extract_text_from_mixed_thinking_and_text() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "mixed-content-test",
            "object": "chat.completion",
            "created": 1722349660,
            "model": "magistral-medium-2507",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        { "type": "thinking", "thinking": [{ "type": "text", "text": "First thought." }] },
                        { "type": "text", "text": "Partial answer." },
                        { "type": "thinking", "thinking": [{ "type": "text", "text": "Second thought." }] },
                        { "type": "text", "text": "Final answer." }
                    ]
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "total_tokens": 30, "completion_tokens": 20 }
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("magistral-medium-2507");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // The Rust `GenerateContent` enum has no reasoning variant; the impl joins
    // all text parts into a single Text content (thinking parts are skipped).
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "Partial answer.Final answer.");
        }
        other => panic!("expected Text, got {:?}", other),
    }
}

/// TS: "should handle empty thinking content" — empty thinking is skipped.
#[tokio::test]
async fn should_handle_empty_thinking_content() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "empty-thinking-test",
            "object": "chat.completion",
            "created": 1722349660,
            "model": "magistral-medium-2507",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        { "type": "thinking", "thinking": [] },
                        { "type": "text", "text": "Just the answer." }
                    ]
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "total_tokens": 30, "completion_tokens": 20 }
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("magistral-medium-2507");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Just the answer."),
        other => panic!("expected Text, got {:?}", other),
    }
}

/// TS: "should return raw text with think tags"
#[tokio::test]
async fn should_return_raw_text_with_think_tags() {
    let server = MockServer::start().await;
    let raw = "Let me think.\n\n\nHello! I'm ready to help.";
    mock_json_response(
        &server,
        json!({
            "object": "chat.completion",
            "id": "raw-think-id",
            "created": 1711113008,
            "model": "magistral-small-2506",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": raw,
                    "tool_calls": null
                },
                "finish_reason": "stop",
                "logprobs": null
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("magistral-small-2506");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert_eq!(text, raw),
        other => panic!("expected Text, got {:?}", other),
    }
}

/// TS: "should map content_filter finish reason"
#[tokio::test]
async fn should_map_content_filter_finish_reason() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "content_filter",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(
        result.finish_reason.unified,
        FinishReasonUnified::ContentFilter
    );
    assert_eq!(result.finish_reason.raw.as_deref(), Some("content_filter"));
}

/// TS: a single response with several tool calls yields multiple ToolCall
/// entries in order.
#[tokio::test]
async fn should_extract_multiple_tool_calls() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "multi-tc",
            "model": "mistral-small-latest",
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [
                        { "id": "call-1", "function": { "name": "weather", "arguments": "{\"city\": \"SF\"}" } },
                        { "id": "call-2", "function": { "name": "time", "arguments": "{\"zone\": \"PST\"}" } }
                    ]
                }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 2);
    match &result.content[0] {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            assert_eq!(tool_call_id, "call-1");
            assert_eq!(tool_name, "weather");
            assert_eq!(input, &json!({"city": "SF"}));
        }
        other => panic!("expected ToolCall, got {:?}", other),
    }
    match &result.content[1] {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            assert_eq!(tool_call_id, "call-2");
            assert_eq!(tool_name, "time");
            assert_eq!(input, &json!({"zone": "PST"}));
        }
        other => panic!("expected ToolCall, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// TS: "should send request body" — verify all standard optional fields.
#[tokio::test]
async fn should_send_default_request_body_shape() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_text_body()).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let options = CallOptions {
        prompt: test_prompt(),
        max_output_tokens: Some(100),
        temperature: Some(0.7),
        top_p: Some(0.9),
        seed: Some(42),
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["model"], "mistral-small-latest");
    assert_eq!(body["max_tokens"], json!(100));
    // f32 round-trip: compare with tolerance.
    assert!((body["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-6);
    assert!((body["top_p"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(body["random_seed"], json!(42));
    assert_eq!(
        body["messages"],
        json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
    );
    assert!(body.get("stop").is_none());
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    assert!(body.get("response_format").is_none());
    assert!(body.get("stream").is_none());
}

/// TS: a 429 response maps to `AiMuxError::ApiCall` (429 in `status_code`).
#[tokio::test]
async fn should_handle_rate_limit_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_json(json!({ "message": "Too many requests", "type": "rate_limit" })),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(429)),
        "expected a 429, got {result:?}"
    );
}

/// TS: a 404 response maps to `AiMuxError::ApiCall` (404 in `status_code`).
#[tokio::test]
async fn should_handle_model_not_found_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({ "message": "Model not found", "type": "not_found" })),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(404)),
        "expected ModelNotFound, got {result:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Additional doStream cases — reasoning, interleaved thinking, headers,
// response metadata, error-in-chunk, prefix continuation.
// ════════════════════════════════════════════════════════════════════════════

/// Extract reasoning deltas from a list of stream parts.
fn reasoning_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ReasoningDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

/// TS: "should stream reasoning" — thinking parts in array content become
/// ReasoningStart/Delta/End stream parts.
#[tokio::test]
async fn should_stream_reasoning() {
    let server = MockServer::start().await;
    let sse = sse_json_body(&[
        r#"{"id":"reasoning-1","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"thinking","thinking":[{"type":"text","text":"Let me think."}]}]},"finish_reason":null}]}"#,
        r#"{"id":"reasoning-1","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"content":[{"type":"text","text":"Answer."}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"total_tokens":15,"completion_tokens":10}}"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("magistral-small-2507");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;
    assert_eq!(reasoning_deltas(&parts), vec!["Let me think.".to_string()]);
    assert_eq!(text_deltas(&parts), vec!["Answer.".to_string()]);
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningStart { .. }))
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { .. }))
    );
}

/// TS: "should handle interleaved thinking and text"
#[tokio::test]
async fn should_stream_interleaved_thinking_and_text() {
    let server = MockServer::start().await;
    let sse = sse_json_body(&[
        r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"thinking","thinking":[{"type":"text","text":"First thought."}]}]},"finish_reason":null}]}"#,
        r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"text","text":"Partial answer."}]},"finish_reason":null}]}"#,
        r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"thinking","thinking":[{"type":"text","text":"Second thought."}]}]},"finish_reason":null}]}"#,
        r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"text","text":"Final answer."}]},"finish_reason":null}]}"#,
        r#"{"id":"interleaved-test","object":"chat.completion.chunk","created":1750538000,"model":"magistral-small-2507","choices":[{"index":0,"delta":{"content":""},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"total_tokens":40,"completion_tokens":30}}"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("magistral-small-2507");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;
    assert_eq!(
        reasoning_deltas(&parts),
        vec!["First thought.".to_string(), "Second thought.".to_string()]
    );
    assert_eq!(
        text_deltas(&parts),
        vec!["Partial answer.".to_string(), "Final answer.".to_string()]
    );
    assert_eq!(
        parts
            .iter()
            .filter(|p| matches!(p, StreamPart::ReasoningStart { .. }))
            .count(),
        2
    );
    assert_eq!(
        parts
            .iter()
            .filter(|p| matches!(p, StreamPart::ReasoningEnd { .. }))
            .count(),
        2
    );
}

/// TS: "should expose the raw response headers" (streaming)
#[tokio::test]
async fn should_expose_raw_response_headers_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .insert_header("test-header", "test-value")
                .set_body_string(sse_json_body(&[
                    r#"{"id":"test","model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"total_tokens":2,"completion_tokens":1}}"#,
                ])),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let headers = result
        .response_headers
        .as_ref()
        .expect("response_headers should be Some");
    assert_eq!(headers.get("test-header"), Some(&"test-value".to_string()));
}

/// TS: "should pass headers" (streaming) — request-level custom headers.
#[tokio::test]
async fn should_pass_request_headers_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(wiremock::matchers::header("custom-stream-header", "stream-value"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_json_body(&[
                    r#"{"id":"test","model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"total_tokens":2,"completion_tokens":1}}"#,
                ])),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Custom-Stream-Header".to_string(),
        "stream-value".to_string(),
    );

    let options = CallOptions {
        prompt: test_prompt(),
        headers: Some(headers),
        ..default_options(Vec::new())
    };

    let result = model.do_stream(&options).await;
    assert!(result.is_ok(), "request should match the header mock");
}

/// TS: streaming ResponseMetadata carries the chunk id and model id.
#[tokio::test]
async fn should_stream_response_metadata() {
    let server = MockServer::start().await;
    mock_sse_response(
        &server,
        &sse_json_body(&[
            r#"{"id":"meta-id","object":"chat.completion.chunk","created":1750538000,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            r#"{"id":"meta-id","object":"chat.completion.chunk","created":1750538000,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"total_tokens":2,"completion_tokens":1}}"#,
        ]),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;
    let meta = parts.iter().find_map(|p| match p {
        StreamPart::ResponseMetadata { id, model_id, .. } => Some((id.clone(), model_id.clone())),
        _ => None,
    });
    let (id, model_id) = meta.expect("should have ResponseMetadata");
    assert_eq!(id.as_deref(), Some("meta-id"));
    assert_eq!(model_id.as_deref(), Some("mistral-small-latest"));
}

/// TS: a 429 HTTP response surfaces as `AiMuxError::ApiCall` (429 in `status_code`) from
/// `do_stream` (the stream is never opened).
#[tokio::test]
async fn should_stream_rate_limit_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_json(json!({ "message": "Too many requests", "type": "rate_limit" })),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model.do_stream(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(429)),
        "expected a 429, got {result:?}"
    );
}

/// TS: an `error` object embedded in a non-first SSE chunk surfaces as a
/// stream `Error` part.
#[tokio::test]
async fn should_stream_error_in_chunk() {
    let server = MockServer::start().await;
    let sse = sse_json_body(&[
        r#"{"id":"ok","object":"chat.completion.chunk","created":1750538000,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
        r#"{"error":{"message":"rate limited","code":429}}"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;
    assert!(
        parts.iter().any(|p| matches!(
            p,
            StreamPart::Error { error } if error.status_code() == Some(429)
        )),
        "expected a 429 stream error, got {parts:?}"
    );
}

/// TS: "should avoid duplication when trailing assistant message" (streaming)
/// — the trailing assistant message is sent with `prefix: true`.
#[tokio::test]
async fn should_stream_with_trailing_assistant_prefix() {
    let server = MockServer::start().await;
    mock_sse_response(
        &server,
        &sse_json_body(&[
            r#"{"id":"53ff663126294946a6b7a4747b70597e","object":"chat.completion.chunk","created":1750537996,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"53ff663126294946a6b7a4747b70597e","object":"chat.completion.chunk","created":1750537996,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":"prefix"},"finish_reason":null,"logprobs":null}]}"#,
            r#"{"id":"53ff663126294946a6b7a4747b70597e","object":"chat.completion.chunk","created":1750537996,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":" and more content"},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":4,"total_tokens":36,"completion_tokens":32}}"#,
        ]),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hello")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::text("prefix ")],
            ..Default::default()
        },
    ];

    let result = model
        .do_stream(&default_options(prompt))
        .await
        .expect("do_stream should succeed");

    let body = result.request_body.clone().expect("body");
    let messages = body["messages"].as_array().expect("messages array");
    let last = messages.last().expect("last message");
    assert_eq!(last["role"], json!("assistant"));
    assert_eq!(last["prefix"], json!(true));

    let parts = collect_stream(result).await;
    assert_eq!(
        text_deltas(&parts),
        vec!["prefix".to_string(), " and more content".to_string()]
    );
}
