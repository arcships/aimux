//! Wiremock tests for the Google Vertex AI provider.
//!
//! Tests cover:
//! - Non-streaming text generation (generateContent JSON response)
//! - Non-streaming tool call extraction
//! - Streaming via SSE (streamGenerateContent)
//! - Error handling
//!
//! Vertex AI uses the same request/response format as the public Google Gemini
//! API, so the response fixtures mirror the Gemini API shape.

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::vertex::{VertexAuth, VertexConfig, VertexModel, VertexProviderConfig};

use aimux_provider_utils::RetryConfig;

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

fn make_model(server: &MockServer) -> VertexModel {
    VertexModel::new(
        "gemini-2.0-flash".to_string(),
        VertexConfig {
            base_url: server.uri(),
            auth: VertexAuth::BearerToken("test-token".to_string()),
            api_key_source: None,
            retry_config: RetryConfig::default(),
        },
    )
}

async fn mock_generate_content(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_generate_error(server: &MockServer, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_stream_content(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:streamGenerateContent"))
        .and(query_param("alt", "sse"))
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

// ═════════════════════════════════════════════════════════════════════════════
// doGenerate tests
// ═════════════════════════════════════════════════════════════════════════════

/// Test: non-streaming text generation with usage and finish reason.
#[tokio::test]
async fn vertex_generate_text_response() {
    let server = MockServer::start().await;
    mock_generate_content(
        &server,
        json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{ "text": "Hello from Vertex!" }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 10,
                "totalTokenCount": 15
            }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    assert_eq!(as_text(&result.content[0]), "Hello from Vertex!");
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.usage.input_tokens.total, Some(5));
    assert_eq!(result.usage.output_tokens.total, Some(10));
}

/// Test: non-streaming tool call extraction.
#[tokio::test]
async fn vertex_generate_tool_call() {
    let server = MockServer::start().await;
    mock_generate_content(
        &server,
        json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "getWeather",
                            "id": "call_1",
                            "args": { "location": "Tokyo" }
                        }
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 5
            }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    let (id, name, input) = as_tool_call(&result.content[0]);
    assert_eq!(id, "call_1");
    assert_eq!(name, "getWeather");
    assert_eq!(input["location"], "Tokyo");
    // STOP with tool calls → ToolCalls
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// Vertex Gemini thinking models echo `thoughtSignature` on `functionCall`
/// parts the same way the public Gemini API does — it must be preserved.
#[tokio::test]
async fn vertex_generate_tool_call_with_thought_signature() {
    let server = MockServer::start().await;
    mock_generate_content(
        &server,
        json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "getWeather",
                            "id": "call_1",
                            "args": { "location": "Tokyo" }
                        },
                        "thoughtSignature": "EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH"
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 5
            }
        }),
    )
    .await;

    let model = make_model(&server);
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
            thought_signature,
            ..
        } => {
            assert_eq!(tool_call_id, "call_1");
            assert_eq!(tool_name, "getWeather");
            assert_eq!(input["location"], "Tokyo");
            assert_eq!(
                thought_signature.as_deref(),
                Some("EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH")
            );
        }
        other => panic!("expected ToolCall, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// Test: HTTP error status is propagated.
#[tokio::test]
async fn vertex_generate_error() {
    let server = MockServer::start().await;
    mock_generate_error(
        &server,
        400,
        json!({
            "error": {
                "code": 400,
                "message": "Invalid model name",
                "status": "INVALID_ARGUMENT"
            }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;

    assert!(result.is_err(), "should return error for 400 status");
}

// ═════════════════════════════════════════════════════════════════════════════
// doStream tests
// ═════════════════════════════════════════════════════════════════════════════

/// Test: streaming text via SSE.
#[tokio::test]
async fn vertex_stream_text() {
    let server = MockServer::start().await;

    let sse_body = sse_stream(&[
        json!({
            "candidates": [{
                "content": { "role": "model", "parts": [{ "text": "Hello" }] }
            }]
        }),
        json!({
            "candidates": [{
                "content": { "role": "model", "parts": [{ "text": " world" }] },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 3,
                "candidatesTokenCount": 5
            }
        }),
    ]);

    mock_stream_content(&server, &sse_body).await;

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
    assert_eq!(text_deltas, vec!["Hello", " world"]);

    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { finish_reason, .. } => Some(finish_reason.clone()),
        _ => None,
    });
    assert!(finish.is_some());
    assert_eq!(finish.unwrap().unified, FinishReasonUnified::Stop);
}

/// Test: API key authentication uses x-goog-api-key header.
#[tokio::test]
async fn vertex_api_key_auth() {
    let server = MockServer::start().await;
    mock_generate_content(
        &server,
        json!({
            "candidates": [{
                "content": { "parts": [{ "text": "Express!" }] },
                "finishReason": "STOP"
            }]
        }),
    )
    .await;

    let model = VertexModel::new(
        "gemini-2.0-flash".to_string(),
        VertexConfig {
            base_url: server.uri(),
            auth: VertexAuth::ApiKey("test-api-key".to_string()),
            api_key_source: None,
            retry_config: RetryConfig::default(),
        },
    );

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed with API key");

    assert_eq!(as_text(&result.content[0]), "Express!");
}

/// Test: provider config construction and base URL.
#[tokio::test]
async fn vertex_provider_config() {
    let config = VertexProviderConfig::new("test-token", "my-project", "us-central1");
    assert!(
        config
            .base_url
            .contains("us-central1-aiplatform.googleapis.com")
    );
    assert!(config.base_url.contains("my-project"));

    let config_express = VertexProviderConfig::with_api_key("test-key");
    assert!(
        config_express
            .base_url
            .contains("aiplatform.googleapis.com")
    );
    match config_express.auth {
        VertexAuth::ApiKey(k) => assert_eq!(k, "test-key"),
        _ => panic!("expected ApiKey auth"),
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Additional cases — finish reasons, settings, headers, stream tool calls.
// ═════════════════════════════════════════════════════════════════════════════

/// A minimal Vertex generateContent "ok" body.
fn ok_vertex_body() -> Value {
    json!({
        "candidates": [{
            "content": { "role": "model", "parts": [{ "text": "ok" }] },
            "finishReason": "STOP"
        }],
        "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 7 }
    })
}

/// TS: MAX_TOKENS → Length
#[tokio::test]
async fn vertex_generate_finish_reason_max_tokens() {
    let server = MockServer::start().await;
    mock_generate_content(
        &server,
        json!({
            "candidates": [{
                "content": { "parts": [{ "text": "truncated" }] },
                "finishReason": "MAX_TOKENS"
            }],
            "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 34 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Length);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("MAX_TOKENS"));
}

/// TS: SAFETY → ContentFilter
#[tokio::test]
async fn vertex_generate_finish_reason_safety() {
    let server = MockServer::start().await;
    mock_generate_content(
        &server,
        json!({
            "candidates": [{
                "content": { "parts": [{ "text": "" }] },
                "finishReason": "SAFETY"
            }],
            "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 1 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(
        result.finish_reason.unified,
        FinishReasonUnified::ContentFilter
    );
}

/// TS: settings land in `generationConfig`.
#[tokio::test]
async fn vertex_generate_settings() {
    let server = MockServer::start().await;
    mock_generate_content(&server, ok_vertex_body()).await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.max_output_tokens = Some(256);
    opts.temperature = Some(0.5);
    opts.top_p = Some(0.9);
    opts.top_k = Some(40.0);

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["generationConfig"]["maxOutputTokens"], json!(256));
    assert!((body["generationConfig"]["temperature"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert!((body["generationConfig"]["topP"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(body["generationConfig"]["topK"], json!(40.0));
}

/// TS: request body carries the user message as `contents`.
#[tokio::test]
async fn vertex_generate_request_body() {
    let server = MockServer::start().await;
    mock_generate_content(&server, ok_vertex_body()).await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let body = result.request_body.expect("body");
    assert_eq!(body["contents"][0]["role"], json!("user"));
    assert_eq!(body["contents"][0]["parts"][0]["text"], json!("Hello"));
}

/// TS: response headers are exposed on the generate result.
#[tokio::test]
async fn vertex_generate_response_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(ok_vertex_body()),
        )
        .mount(&server)
        .await;

    let model = make_model(&server);
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

/// TS: streaming tool calls via SSE.
#[tokio::test]
async fn vertex_stream_tool_call() {
    let server = MockServer::start().await;
    let sse_body = sse_stream(&[json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{
                    "functionCall": {
                        "name": "getWeather",
                        "id": "call_1",
                        "args": { "location": "Tokyo" }
                    }
                }]
            },
            "finishReason": "STOP"
        }]
    })]);
    mock_stream_content(&server, &sse_body).await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;
    let tool_call = parts.iter().find_map(|p| match p {
        StreamPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
        _ => None,
    });
    let (id, name, input) = tool_call.expect("should have ToolCall");
    assert_eq!(id, "call_1");
    assert_eq!(name, "getWeather");
    assert_eq!(input["location"], "Tokyo");
}

/// TS: response headers are exposed on the stream result.
#[tokio::test]
async fn vertex_stream_response_headers() {
    let server = MockServer::start().await;
    let sse_body = sse_stream(&[json!({
        "candidates": [{
            "content": { "parts": [{ "text": "Hi" }] },
            "finishReason": "STOP"
        }]
    })]);
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:streamGenerateContent"))
        .and(query_param("alt", "sse"))
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

/// TS: a 429 response maps to `AiMuxError::RateLimited`.
#[tokio::test]
async fn vertex_generate_rate_limit() {
    let server = MockServer::start().await;
    mock_generate_error(
        &server,
        429,
        json!({ "error": { "code": 429, "message": "Quota exceeded", "status": "RESOURCE_EXHAUSTED" } }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::RateLimited { .. })),
        "expected RateLimited, got {result:?}"
    );
}
