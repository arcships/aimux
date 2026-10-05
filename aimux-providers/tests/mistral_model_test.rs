//! Wiremock tests for the Mistral provider.
//!
//! Translated from `packages/mistral/src/mistral-chat-language-model.test.ts`,
//! focusing on the cases that the Rust data model can express:
//! - doGenerate: text extraction, usage, tool calls, request body
//! - doStream: text streaming, tool call streaming, request body

use aimux_core::tool::RawToolCall;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, ReasoningOutput, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReasonUnified, ResponseMetadata};

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
        body.push_str(&format!("data: {c}\n\n"));
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
            Err(e) => panic!("stream error: {e:?}"),
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
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => {
            assert_eq!(tool_call_id, "gSIMJiOkT");
            assert_eq!(tool_name, "weather");
            assert_eq!(input, r#"{"location": "San Francisco"}"#);
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
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
        other => panic!("expected Finish, got {other:?}"),
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
        StreamPart::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => Some((tool_call_id, tool_name, input)),
        _ => None,
    });
    let (id, name, input) = tool_call.expect("should have a ToolCall");
    assert_eq!(id, "gSIMJiOkT");
    assert_eq!(name, "weather");
    assert_eq!(input, r#"{"location": "San Francisco"}"#);

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
        other => panic!("expected Text, got {other:?}"),
    }
}

/// TS: "should preserve ordering of mixed thinking and text".
///
/// Thinking parts become a `Reasoning` item, pushed ahead of the text to match
/// the upstream ordering; the text parts are joined as before.
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

    // Reasoning first, then the joined text.
    assert_eq!(result.content.len(), 2);
    match &result.content[0] {
        GenerateContent::Reasoning(ReasoningOutput { text, .. }) => {
            assert_eq!(
                text, "First thought.Second thought.",
                "both thinking parts must survive, in order"
            );
        }
        other => panic!("expected Reasoning at [0], got {other:?}"),
    }
    match &result.content[1] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "Partial answer.Final answer.");
        }
        other => panic!("expected Text at [1], got {other:?}"),
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
        other => panic!("expected Text, got {other:?}"),
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
        other => panic!("expected Text, got {other:?}"),
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
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => {
            assert_eq!(tool_call_id, "call-1");
            assert_eq!(tool_name, "weather");
            assert_eq!(input, r#"{"city": "SF"}"#);
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    match &result.content[1] {
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => {
            assert_eq!(tool_call_id, "call-2");
            assert_eq!(tool_name, "time");
            assert_eq!(input, r#"{"zone": "PST"}"#);
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
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
        StreamPart::ResponseMetadata(ResponseMetadata { id, model_id, .. }) => {
            Some((id.clone(), model_id.clone()))
        }
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
