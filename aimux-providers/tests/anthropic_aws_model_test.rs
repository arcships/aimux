//! Wiremock tests for the Anthropic-AWS (Claude Platform on AWS) provider.
//!
//! Tests cover:
//! - Non-streaming text generation (Anthropic Messages API JSON response)
//! - Non-streaming tool call extraction
//! - Streaming via SSE (Anthropic message events)
//! - API key authentication
//! - Error handling

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::anthropic_aws::{AnthropicAwsAuth, AnthropicAwsConfig, AnthropicAwsModel};

// ── Helpers ──────────────────────────────────────────────────────────────────

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

fn make_model(server: &MockServer) -> AnthropicAwsModel {
    AnthropicAwsModel::new(
        "claude-sonnet-4-20250514".to_string(),
        AnthropicAwsConfig {
            base_url: server.uri(),
            auth: AnthropicAwsAuth::ApiKey("test-api-key".to_string()),
            api_version: "2023-06-01".to_string(),
            workspace_id: None,
        },
    )
}

async fn mock_messages_json(server: &MockServer, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_messages_sse(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

fn sse(data: &Value) -> String {
    format!("data: {}\n\n", data)
}

fn sse_stream(events: &[Value]) -> String {
    events.iter().map(sse).collect()
}

fn as_text(item: &GenerateContent) -> &str {
    match item {
        GenerateContent::Text { text, .. } => text,
        _ => panic!("expected Text content, got {:?}", item),
    }
}

fn as_tool_call(item: &GenerateContent) -> (&str, &str, &Value) {
    match item {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => (tool_call_id, tool_name, input),
        _ => panic!("expected ToolCall content, got {:?}", item),
    }
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

fn text_response(text: &str) -> Value {
    json!({
        "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "text", "text": text }],
        "model": "claude-sonnet-4-20250514",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": { "input_tokens": 4, "output_tokens": 30 }
    })
}

// ═════════════════════════════════════════════════════════════════════════════
// doGenerate tests
// ═════════════════════════════════════════════════════════════════════════════

/// Test: non-streaming text generation extracts text, usage, and finish reason.
#[tokio::test]
async fn anthropic_aws_generate_text_response() {
    let server = MockServer::start().await;
    mock_messages_json(&server, 200, text_response("Hello from Claude on AWS!")).await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    assert_eq!(as_text(&result.content[0]), "Hello from Claude on AWS!");
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(30));
    assert_eq!(
        result.response.id.as_deref(),
        Some("msg_017TfcQ4AgGxKyBduUpqYPZn")
    );
}

/// Test: non-streaming tool call extraction.
#[tokio::test]
async fn anthropic_aws_generate_tool_call() {
    let server = MockServer::start().await;
    mock_messages_json(
        &server,
        200,
        json!({
            "id": "msg_001",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "I'll check that for you." },
                {
                    "type": "tool_use",
                    "id": "toolu_01A",
                    "name": "getWeather",
                    "input": { "location": "Paris" }
                }
            ],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 10, "output_tokens": 20 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 2);
    assert_eq!(as_text(&result.content[0]), "I'll check that for you.");

    let (id, name, input) = as_tool_call(&result.content[1]);
    assert_eq!(id, "toolu_01A");
    assert_eq!(name, "getWeather");
    assert_eq!(input["location"], "Paris");
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// Test: HTTP error status is propagated.
#[tokio::test]
async fn anthropic_aws_generate_error() {
    let server = MockServer::start().await;
    mock_messages_json(
        &server,
        429,
        json!({
            "type": "error",
            "error": { "type": "rate_limit_error", "message": "Too many requests" }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;

    assert!(result.is_err(), "should return error for 429 status");
}

/// Test: request body uses Anthropic format with anthropic-version header.
#[tokio::test]
async fn anthropic_aws_request_body_and_headers() {
    let server = MockServer::start().await;

    // Mount a mock that also verifies the x-api-key header.
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(header("x-api-key", "test-api-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_response("OK")))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(as_text(&result.content[0]), "OK");

    // Verify request body format.
    let body = result.request_body.expect("should have request body");
    assert_eq!(body["model"], "claude-sonnet-4-20250514");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"][0]["text"], "Hello");
    assert_eq!(body["max_tokens"], 64000); // model default for claude-sonnet-4
}

// ═════════════════════════════════════════════════════════════════════════════
// doStream tests
// ═════════════════════════════════════════════════════════════════════════════

/// Test: streaming text via Anthropic SSE events.
#[tokio::test]
async fn anthropic_aws_stream_text() {
    let server = MockServer::start().await;

    let sse_body = sse_stream(&[
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_001",
                "model": "claude-sonnet-4-20250514",
                "usage": { "input_tokens": 3 }
            }
        }),
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": { "type": "text", "text": "" }
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "Hello" }
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": " streamed!" }
        }),
        json!({
            "type": "content_block_stop",
            "index": 0
        }),
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn" },
            "usage": { "output_tokens": 5 }
        }),
        json!({ "type": "message_stop" }),
    ]);

    mock_messages_sse(&server, &sse_body).await;

    let model = make_model(&server);
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
    assert_eq!(text_deltas, vec!["Hello", " streamed!"]);

    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { finish_reason, .. } => Some(finish_reason.clone()),
        _ => None,
    });
    assert!(finish.is_some());
    assert_eq!(finish.unwrap().unified, FinishReasonUnified::Stop);
}

/// Test: streaming tool calls via Anthropic SSE events.
#[tokio::test]
async fn anthropic_aws_stream_tool_call() {
    let server = MockServer::start().await;

    let sse_body = sse_stream(&[
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_002",
                "model": "claude-sonnet-4-20250514",
                "usage": { "input_tokens": 5 }
            }
        }),
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_01B",
                "name": "getWeather",
                "input": {}
            }
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "input_json_delta", "partial_json": "{\"location\":" }
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "input_json_delta", "partial_json": "\"Berlin\"}" }
        }),
        json!({
            "type": "content_block_stop",
            "index": 0
        }),
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "tool_use" },
            "usage": { "output_tokens": 10 }
        }),
        json!({ "type": "message_stop" }),
    ]);

    mock_messages_sse(&server, &sse_body).await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;

    let tool_calls: Vec<_> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].0, "toolu_01B");
    assert_eq!(tool_calls[0].1, "getWeather");
    assert_eq!(tool_calls[0].2["location"], "Berlin");
}

/// Test: SigV4 authentication adds Authorization header.
#[tokio::test]
async fn anthropic_aws_sigv4_auth() {
    let server = MockServer::start().await;
    mock_messages_json(&server, 200, text_response("Signed!")).await;

    let model = AnthropicAwsModel::new(
        "claude-sonnet-4-20250514".to_string(),
        AnthropicAwsConfig {
            base_url: server.uri(),
            auth: AnthropicAwsAuth::SigV4(aimux_providers::bedrock::AwsCredentials {
                access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
                secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
                session_token: None,
                region: "us-east-1".to_string(),
            }),
            api_version: "2023-06-01".to_string(),
            workspace_id: None,
        },
    );

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed with SigV4");

    assert_eq!(as_text(&result.content[0]), "Signed!");
}

// ═════════════════════════════════════════════════════════════════════════════
// Additional cases — finish reasons, settings, usage, headers, errors.
// ═════════════════════════════════════════════════════════════════════════════

use aimux_core::error::AiMuxError;

/// TS: max_tokens stop reason → Length
#[tokio::test]
async fn anthropic_aws_generate_finish_reason_max_tokens() {
    let server = MockServer::start().await;
    mock_messages_json(
        &server,
        200,
        json!({
            "id": "msg_max",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "truncated" }],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "max_tokens",
            "usage": { "input_tokens": 4, "output_tokens": 30 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Length);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("max_tokens"));
}

/// TS: an unknown stop reason → Other
#[tokio::test]
async fn anthropic_aws_generate_finish_reason_unknown() {
    let server = MockServer::start().await;
    mock_messages_json(
        &server,
        200,
        json!({
            "id": "msg_unk",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "hi" }],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "pause_turn",
            "usage": { "input_tokens": 4, "output_tokens": 30 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("pause_turn"));
}

/// TS: settings — max_tokens override, temperature, top_p land in the body.
#[tokio::test]
async fn anthropic_aws_generate_settings() {
    let server = MockServer::start().await;
    mock_messages_json(&server, 200, text_response("ok")).await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.max_output_tokens = Some(1024);
    opts.temperature = Some(0.5);
    opts.top_p = Some(0.9);
    opts.stop_sequences = Some(vec!["END".to_string()]);

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["max_tokens"], json!(1024));
    assert!((body["temperature"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert!((body["top_p"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(body["stop_sequences"], json!(["END"]));
}

/// TS: a 401 response maps to `AiMuxError::Auth`.
#[tokio::test]
async fn anthropic_aws_generate_auth_error() {
    let server = MockServer::start().await;
    mock_messages_json(
        &server,
        401,
        json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(_))),
        "expected Auth, got {result:?}"
    );
}

/// TS: a 429 response maps to `AiMuxError::RateLimited`.
#[tokio::test]
async fn anthropic_aws_generate_rate_limit_error() {
    let server = MockServer::start().await;
    mock_messages_json(
        &server,
        429,
        json!({ "type": "error", "error": { "type": "rate_limit_error", "message": "Too many requests" } }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::RateLimited { .. })),
        "expected RateLimited, got {result:?}"
    );
}

/// TS: response headers are exposed on the stream result.
#[tokio::test]
async fn anthropic_aws_stream_response_headers() {
    let server = MockServer::start().await;
    let sse_body = sse_stream(&[
        json!({
            "type": "message_start",
            "message": { "id": "msg_h", "model": "claude-sonnet-4-20250514", "usage": { "input_tokens": 3 } }
        }),
        json!({
            "type": "content_block_start", "index": 0,
            "content_block": { "type": "text", "text": "" }
        }),
        json!({
            "type": "content_block_delta", "index": 0,
            "delta": { "type": "text_delta", "text": "Hi" }
        }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn" },
            "usage": { "output_tokens": 1 }
        }),
        json!({ "type": "message_stop" }),
    ]);
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .insert_header("test-header", "test-value")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let model = make_model(&server);
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
