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

/// TS: a 429 response maps to `AiMuxError::ApiCall` (429 in `status_code`).
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
        matches!(result, Err(ref e) if e.status_code() == Some(429)),
        "expected RateLimited, got {result:?}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// doStream — provider tool results & metadata (mirrors the google provider's
// do_stream tests in google_provider_tools_test.rs; issue #141).
// ═════════════════════════════════════════════════════════════════════════════

/// A single SSE chunk carrying `content.parts: [{ text }]`, optional
/// `finishReason`, and optional `groundingMetadata` / `urlContextMetadata`.
fn stream_chunk(
    text: &str,
    finish_reason: Option<&str>,
    grounding: Option<Value>,
    url_context: Option<Value>,
) -> Value {
    let mut candidate = json!({
        "content": { "parts": [{ "text": text }], "role": "model" }
    });
    if let Some(r) = finish_reason {
        candidate["finishReason"] = json!(r);
    }
    if let Some(g) = grounding {
        candidate["groundingMetadata"] = g;
    }
    if let Some(u) = url_context {
        candidate["urlContextMetadata"] = u;
    }
    json!({ "candidates": [candidate] })
}

/// Extract the `Finish` part's provider metadata from a collected stream.
fn finish_provider_metadata(parts: &[StreamPart]) -> Option<Value> {
    parts.iter().find_map(|p| match p {
        StreamPart::Finish {
            provider_metadata, ..
        } => provider_metadata.clone(),
        _ => None,
    })
}

/// Collect `(tool_call_id, tool_name, input)` from streamed `ToolCall` parts.
fn stream_tool_calls(parts: &[StreamPart]) -> Vec<(String, String, Value)> {
    parts
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
        .collect()
}

/// Collect `(tool_call_id, result)` from streamed `ToolResult` parts.
fn stream_tool_results(parts: &[StreamPart]) -> Vec<(String, Value)> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolResult {
                tool_call_id,
                result,
                ..
            } => Some((tool_call_id.clone(), result.clone())),
            _ => None,
        })
        .collect()
}

/// Collect `(id, source_type, url, title)` from streamed `Source` parts.
fn stream_sources(parts: &[StreamPart]) -> Vec<(String, String, Option<String>, Option<String>)> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::Source {
                id,
                source_type,
                url,
                title,
                ..
            } => Some((id.clone(), source_type.clone(), url.clone(), title.clone())),
            _ => None,
        })
        .collect()
}

/// TS: "should stream code execution tool calls and results" — the Vertex
/// stream must not silently drop provider-executed code results (#141).
#[tokio::test]
async fn vertex_stream_code_execution_tool_calls_and_results() {
    let server = MockServer::start().await;
    mock_stream_content(
        &server,
        &sse_stream(&[
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "executableCode": { "language": "PYTHON", "code": "print(\"hello\")" } }]
                    }
                }]
            }),
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "hello\n" } }]
                    },
                    "finishReason": "STOP"
                }]
            }),
        ]),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    let calls = stream_tool_calls(&parts);
    let has_call = calls.iter().any(|(_, name, input)| {
        name == "code_execution"
            && *input == json!({ "language": "PYTHON", "code": "print(\"hello\")" })
    });
    assert!(
        has_call,
        "expected a code_execution tool-call, got {:?}",
        calls
    );

    // The ToolResult must reference the preceding executableCode call id.
    let results = stream_tool_results(&parts);
    let call_id = calls
        .iter()
        .find(|(_, name, _)| name == "code_execution")
        .map(|(id, _, _)| id.clone())
        .expect("code_execution call id");
    let has_result = results.iter().any(|(id, output)| {
        *id == call_id && *output == json!({ "outcome": "OUTCOME_OK", "output": "hello\n" })
    });
    assert!(
        has_result,
        "expected a code_execution tool-result, got {:?}",
        results
    );

    // Provider-executed tool → Stop, not ToolCalls.
    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { finish_reason, .. } => Some(finish_reason.clone()),
        _ => None,
    });
    assert_eq!(
        finish.expect("finish part").unified,
        FinishReasonUnified::Stop
    );
}

/// TS: "should stream code execution result with missing output field".
#[tokio::test]
async fn vertex_stream_code_execution_result_missing_output() {
    let server = MockServer::start().await;
    mock_stream_content(
        &server,
        &sse_stream(&[
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "executableCode": {
                            "language": "PYTHON",
                            "code": "img = PIL.Image.open('input.png')\nimg.save('output.png')\n"
                        } }]
                    }
                }]
            }),
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "codeExecutionResult": { "outcome": "OUTCOME_OK" } }]
                    },
                    "finishReason": "STOP"
                }]
            }),
        ]),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    assert!(
        stream_tool_calls(&parts)
            .iter()
            .any(|(_, name, _)| name == "code_execution"),
        "expected a code_execution tool-call"
    );
    // Missing output defaults to "".
    let results = stream_tool_results(&parts);
    let has_empty = results
        .iter()
        .any(|(_, output)| *output == json!({ "outcome": "OUTCOME_OK", "output": "" }));
    assert!(
        has_empty,
        "expected a tool-result with empty output, got {:?}",
        results
    );
}

/// TS: "should stream server-side toolCall and toolResponse parts (tool
/// combination)" — server tools are provider-executed and must be streamed.
#[tokio::test]
async fn vertex_stream_server_tool_call_and_response() {
    let server = MockServer::start().await;
    mock_stream_content(
        &server,
        &sse_stream(&[
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "toolCall": {
                            "toolType": "GOOGLE_SEARCH_WEB",
                            "args": { "query": "San Francisco weather" },
                            "id": "server-call-1"
                        }, "thoughtSignature": "sig-abc" }]
                    }
                }]
            }),
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "toolResponse": {
                                "toolType": "GOOGLE_SEARCH_WEB",
                                "response": { "results": [{ "title": "Weather in SF" }] },
                                "id": "server-call-1"
                            }, "thoughtSignature": "sig-def" },
                            { "text": "The weather in San Francisco is sunny." }
                        ]
                    },
                    "finishReason": "STOP"
                }]
            }),
        ]),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    let calls = stream_tool_calls(&parts);
    assert!(
        calls
            .iter()
            .any(|(_, name, _)| name == "server:GOOGLE_SEARCH_WEB"),
        "expected a server-side tool-call in the stream, got {:?}",
        calls
    );

    let results = stream_tool_results(&parts);
    assert!(
        results.iter().any(|(id, output)| *id == "server-call-1"
            && *output == json!({ "results": [{ "title": "Weather in SF" }] })),
        "expected a server tool-response keyed to server-call-1, got {:?}",
        results
    );

    // Provider-executed server tools → stop, not tool-calls.
    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { finish_reason, .. } => Some(finish_reason.clone()),
        _ => None,
    });
    assert_eq!(
        finish.expect("finish part").unified,
        FinishReasonUnified::Stop
    );
}

/// TS: "should stream source events" + "should deduplicate sources across
/// chunks" — groundingMetadata is processed, not dropped.
#[tokio::test]
async fn vertex_stream_grounding_metadata_sources() {
    let server = MockServer::start().await;
    mock_stream_content(
        &server,
        &sse_stream(&[
            stream_chunk(
                "first chunk",
                None,
                Some(json!({
                    "groundingChunks": [
                        { "web": { "uri": "https://example.com", "title": "Example" } },
                        { "web": { "uri": "https://unique.com", "title": "Unique" } }
                    ]
                })),
                None,
            ),
            stream_chunk(
                "second chunk",
                None,
                Some(json!({
                    "groundingChunks": [
                        { "web": { "uri": "https://example.com", "title": "Example Duplicate" } },
                        { "web": { "uri": "https://another.com", "title": "Another" } }
                    ]
                })),
                None,
            ),
            stream_chunk("final chunk", Some("STOP"), None, None),
        ]),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    let sources = stream_sources(&parts);
    // The duplicate https://example.com appears in two chunks but should be
    // emitted only once (keeping the first title).
    let example: Vec<_> = sources
        .iter()
        .filter(|(_, _, url, _)| url.as_deref() == Some("https://example.com"))
        .collect();
    assert_eq!(
        example.len(),
        1,
        "duplicate source should be emitted once, got {:?}",
        sources
    );
    assert_eq!(example[0].1, "url");
    assert_eq!(example[0].3.as_deref(), Some("Example"));
    assert_eq!(
        sources.len(),
        3,
        "expected 3 deduplicated sources, got {:?}",
        sources
    );
}

/// TS: "should preserve grounding/url context metadata when it arrives before
/// the finishReason chunk" — Finish provider_metadata is non-empty with all
/// six fields under the `googleVertex` key.
#[tokio::test]
async fn vertex_stream_finish_provider_metadata() {
    let server = MockServer::start().await;
    mock_stream_content(
        &server,
        &sse_stream(&[
            {
                let mut c = stream_chunk(
                    "hello",
                    None,
                    Some(json!({
                        "webSearchQueries": ["super bowl 2026 halftime show"],
                        "groundingChunks": [{
                            "web": { "uri": "https://example.com/superbowl", "title": "Super Bowl 2026" }
                        }]
                    })),
                Some(json!({
                    "urlMetadata": [{
                        "retrievedUrl": "https://example.com/page",
                        "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS"
                    }]
                })),
                );
                // Carry a valued promptFeedback so the assertion below locks
                // the captured value, not just key presence.
                c["promptFeedback"] = json!({ "promptTokenCount": 12 });
                c
            },
            {
                let mut c = stream_chunk(" world", Some("STOP"), None, None);
                c["candidates"][0]["safetyRatings"] = json!([
                    { "category": "HARM_CATEGORY_HARASSMENT", "probability": "NEGLIGIBLE" }
                ]);
                c["candidates"][0]["finishMessage"] = json!("natural stop");
                c["usageMetadata"] = json!({
                    "promptTokenCount": 38,
                    "candidatesTokenCount": 1335,
                    "totalTokenCount": 1890
                });
                c
            },
        ]),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    let pm = finish_provider_metadata(&parts).expect("finish part");
    let vertex = &pm["googleVertex"];
    assert!(
        !vertex.is_null() && vertex.as_object().map(|o| !o.is_empty()).unwrap_or(false),
        "googleVertex provider metadata should be non-empty, got {}",
        pm
    );
    assert_eq!(
        vertex["groundingMetadata"],
        json!({
            "webSearchQueries": ["super bowl 2026 halftime show"],
            "groundingChunks": [{
                "web": { "uri": "https://example.com/superbowl", "title": "Super Bowl 2026" }
            }]
        })
    );
    assert_eq!(
        vertex["urlContextMetadata"],
        json!({
            "urlMetadata": [{
                "retrievedUrl": "https://example.com/page",
                "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS"
            }]
        })
    );
    // usageMetadata is the serialized GoogleUsageMetadata (includes null
    // optional fields, mirroring the google provider).
    assert_eq!(vertex["usageMetadata"]["promptTokenCount"], json!(38));
    assert_eq!(vertex["usageMetadata"]["candidatesTokenCount"], json!(1335));
    assert_eq!(vertex["usageMetadata"]["totalTokenCount"], json!(1890));
    // All six keys are always present (null when absent), matching google.
    for key in [
        "promptFeedback",
        "groundingMetadata",
        "urlContextMetadata",
        "safetyRatings",
        "usageMetadata",
        "finishMessage",
    ] {
        assert!(
            vertex.get(key).is_some(),
            "expected key `{key}` in googleVertex metadata, got {}",
            pm
        );
    }
    // …and the three snapshot fields carry their captured values, not just
    // key presence (locks the capture code, not the key scaffold).
    assert_eq!(vertex["promptFeedback"], json!({ "promptTokenCount": 12 }));
    assert_eq!(
        vertex["safetyRatings"],
        json!([{ "category": "HARM_CATEGORY_HARASSMENT", "probability": "NEGLIGIBLE" }])
    );
    assert_eq!(vertex["finishMessage"], json!("natural stop"));

    // Usage also lands on the Finish part itself.
    let finish_usage = parts.iter().find_map(|p| match p {
        StreamPart::Finish { usage, .. } => Some(usage.clone()),
        _ => None,
    });
    assert_eq!(
        finish_usage.expect("finish usage").input_tokens.total,
        Some(38)
    );
}
