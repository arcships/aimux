//! Wiremock tests for the Cohere provider.
//!
//! Translated from `packages/cohere/src/cohere-chat-language-model.test.ts`,
//! focusing on the cases that the Rust data model can express:
//! - doGenerate: text extraction, MAX_TOKENS finish reason, tool calls, request body
//! - doStream: text streaming, tool call streaming, reasoning streaming

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

use aimux_providers::{CohereConfig, CohereProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a system message + a user text message.
fn test_prompt() -> LanguageModelPrompt {
    vec![
        LanguageModelPromptMessage {
            role: Role::System,
            content: vec![ContentPart::text("you are a friendly bot!")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hello")],
            ..Default::default()
        },
    ]
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
        .and(path("/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_sse_response(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

/// Build a Cohere SSE event from a JSON value (with `event:` field).
fn cohere_event(json_str: &str) -> String {
    // Parse to get the type, then format as `event: type\ndata: json\n\n`.
    if let Ok(val) = serde_json::from_str::<Value>(json_str) {
        let event_type = val
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("message");
        format!("event: {event_type}\ndata: {json_str}\n\n")
    } else {
        format!("event: unknown\ndata: {json_str}\n\n")
    }
}

/// Concatenate Cohere SSE events.
fn cohere_sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(&cohere_event(event));
    }
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

/// TS: "should extract text response"
#[tokio::test]
async fn should_extract_text_response() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "e7592632-1e3d-424f-b129-bd5f9f980f7b",
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "The capital of France is Paris." }
                ]
            },
            "finish_reason": "COMPLETE",
            "usage": {
                "billed_units": { "input_tokens": 12, "output_tokens": 7 },
                "tokens": { "input_tokens": 507, "output_tokens": 10 }
            }
        }),
    )
    .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "The capital of France is Paris.");
        }
        other => panic!("expected Text, got {other:?}"),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("COMPLETE"));
}

/// TS: "should map MAX_TOKENS finish reason to length"
#[tokio::test]
async fn should_map_max_tokens_finish_reason() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "max-tokens-id",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "truncated" }]
            },
            "finish_reason": "MAX_TOKENS",
            "usage": {
                "billed_units": { "input_tokens": 12, "output_tokens": 7 },
                "tokens": { "input_tokens": 507, "output_tokens": 10 }
            }
        }),
    )
    .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Length);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("MAX_TOKENS"));
}

/// TS: "should extract tool calls"
#[tokio::test]
async fn should_extract_tool_calls() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "f201af17-e24a-4396-8f6a-98e8bf9c3432",
            "message": {
                "role": "assistant",
                "tool_plan": "I will use the weather tool.",
                "tool_calls": [
                    {
                        "id": "weather_dqgshstja6p9",
                        "type": "function",
                        "function": {
                            "name": "weather",
                            "arguments": "{\"location\":\"San Francisco\"}"
                        }
                    },
                    {
                        "id": "cityAttractions_dcxfx4myvx68",
                        "type": "function",
                        "function": {
                            "name": "cityAttractions",
                            "arguments": "{\"city\":\"San Francisco\"}"
                        }
                    }
                ]
            },
            "finish_reason": "TOOL_CALL",
            "usage": {
                "billed_units": { "input_tokens": 119, "output_tokens": 52 },
                "tokens": { "input_tokens": 1549, "output_tokens": 103 }
            }
        }),
    )
    .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let options = CallOptions {
        tools: Some(vec![test_tool()]),
        ..default_options(test_prompt())
    };

    let result = model.do_generate(&options).await.expect("should succeed");

    assert_eq!(result.content.len(), 2);
    match &result.content[0] {
        GenerateContent::ToolCall {
            tool_name, input, ..
        } => {
            assert_eq!(tool_name, "weather");
            assert_eq!(
                input,
                &Value::String(r#"{"location":"San Francisco"}"#.into())
            );
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    match &result.content[1] {
        GenerateContent::ToolCall {
            tool_name, input, ..
        } => {
            assert_eq!(tool_name, "cityAttractions");
            assert_eq!(input, &Value::String(r#"{"city":"San Francisco"}"#.into()));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// TS: "should handle string 'null' tool call arguments"
#[tokio::test]
async fn should_handle_null_tool_call_arguments() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "null-args-id",
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "currentTime_abc",
                    "type": "function",
                    "function": {
                        "name": "currentTime",
                        "arguments": "null"
                    }
                }]
            },
            "finish_reason": "TOOL_CALL",
            "usage": {
                "billed_units": { "input_tokens": 10, "output_tokens": 5 },
                "tokens": { "input_tokens": 20, "output_tokens": 10 }
            }
        }),
    )
    .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::ToolCall { input, .. } => {
            // "null" should be replaced with "{}".
            assert_eq!(input, &Value::String("{}".into()));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

/// TS: "should extract usage"
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "Hello" }]
            },
            "finish_reason": "COMPLETE",
            "usage": {
                "billed_units": { "input_tokens": 12, "output_tokens": 7 },
                "tokens": { "input_tokens": 507, "output_tokens": 10 }
            }
        }),
    )
    .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // Cohere uses `tokens` (not `billed_units`) for usage.
    assert_eq!(result.usage.input_tokens.total, Some(507));
    assert_eq!(result.usage.input_tokens.no_cache, Some(507));
    assert_eq!(result.usage.output_tokens.total, Some(10));
}

/// TS: "should pass model and messages"
#[tokio::test]
async fn should_send_correct_request_body() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "Hi" }]
            },
            "finish_reason": "COMPLETE",
            "usage": {
                "billed_units": { "input_tokens": 12, "output_tokens": 7 },
                "tokens": { "input_tokens": 12, "output_tokens": 7 }
            }
        }),
    )
    .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let body = result.request_body.expect("body");
    assert_eq!(body["model"], "command-r-plus");
    assert_eq!(
        body["messages"],
        json!([
            { "role": "system", "content": "you are a friendly bot!" },
            { "role": "user", "content": "Hello" }
        ])
    );
    // No extra keys when options are unset.
    assert!(body.get("max_tokens").is_none());
    assert!(body.get("temperature").is_none());
    assert!(body.get("tools").is_none());
    assert!(body.get("documents").is_none());
}

/// TS: "should pass tools" with tool_choice none → "NONE"
#[tokio::test]
async fn should_pass_tools_with_tool_choice_none() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "ok" }]
            },
            "finish_reason": "COMPLETE",
            "usage": {
                "billed_units": { "input_tokens": 12, "output_tokens": 7 },
                "tokens": { "input_tokens": 12, "output_tokens": 7 }
            }
        }),
    )
    .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let options = CallOptions {
        tools: Some(vec![test_tool()]),
        tool_choice: ToolChoice::None,
        ..default_options(test_prompt())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["tool_choice"], json!("NONE"));
    assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
}

// ════════════════════════════════════════════════════════════════════════════
// doStream
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should stream text deltas"
#[tokio::test]
async fn should_stream_text_deltas() {
    let server = MockServer::start().await;
    let sse = cohere_sse_body(&[
        r#"{"id":"321d178c-2c12-44d3-ae42-2f5510f6b1cc","type":"message-start","delta":{"message":{"role":"assistant","content":[],"tool_plan":"","tool_calls":[],"citations":[]}}}"#,
        r#"{"type":"content-start","index":0,"delta":{"message":{"content":{"type":"text","text":""}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":"The"}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" capital"}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" of"}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" France"}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" is"}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":" Paris"}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":"."}}}}"#,
        r#"{"type":"content-end","index":0}"#,
        r#"{"type":"message-end","delta":{"finish_reason":"COMPLETE","usage":{"billed_units":{"input_tokens":12,"output_tokens":7},"tokens":{"input_tokens":507,"output_tokens":10},"cached_tokens":448}}}"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;

    // StreamStart, ResponseMetadata, TextStart, TextDelta x7, TextEnd, Finish.
    assert!(matches!(
        parts.first(),
        Some(StreamPart::StreamStart { .. })
    ));

    let deltas = text_deltas(&parts);
    assert_eq!(
        deltas,
        vec!["The", " capital", " of", " France", " is", " Paris", "."]
    );

    // Check finish.
    let finish = parts.last().expect("should have finish");
    match finish {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.input_tokens.total, Some(507));
            assert_eq!(usage.output_tokens.total, Some(10));
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// TS: "should stream tool deltas"
#[tokio::test]
async fn should_stream_tool_call_deltas() {
    let server = MockServer::start().await;
    let sse = cohere_sse_body(&[
        r#"{"id":"2941521a","type":"message-start","delta":{"message":{"role":"assistant","content":[],"tool_plan":"","tool_calls":[],"citations":[]}}}"#,
        r#"{"type":"tool-call-start","index":0,"delta":{"message":{"tool_calls":{"id":"weather_e8p4pn45zt0t","type":"function","function":{"name":"weather","arguments":""}}}}}"#,
        r#"{"type":"tool-call-delta","index":0,"delta":{"message":{"tool_calls":{"function":{"arguments":"{"}}}}}"#,
        r#"{"type":"tool-call-delta","index":0,"delta":{"message":{"tool_calls":{"function":{"arguments":"\"location\": \"San Francisco\"}"}}}}}"#,
        r#"{"type":"tool-call-end","index":0}"#,
        r#"{"type":"message-end","delta":{"finish_reason":"TOOL_CALL","usage":{"billed_units":{"input_tokens":119,"output_tokens":44},"tokens":{"input_tokens":1549,"output_tokens":95},"cached_tokens":1504}}}"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let options = CallOptions {
        tools: Some(vec![test_tool()]),
        ..default_options(test_prompt())
    };

    let result = model.do_stream(&options).await.expect("should succeed");
    let parts = collect_stream(result).await;

    // Should have ToolInputStart, ToolInputDelta(s), ToolInputEnd, ToolCall.
    let tool_call = parts.iter().find_map(|p| match p {
        StreamPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => Some((tool_call_id, tool_name, input)),
        _ => None,
    });
    let (id, name, input) = tool_call.expect("should have ToolCall");
    assert_eq!(id, "weather_e8p4pn45zt0t");
    assert_eq!(name, "weather");
    // The flush parses and re-serializes compactly, so interior whitespace
    // from the deltas is dropped.
    assert_eq!(
        input,
        &Value::String(r#"{"location":"San Francisco"}"#.into())
    );

    // Verify the accumulated deltas.
    let deltas: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolInputDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, vec!["{", "\"location\": \"San Francisco\"}"]);

    // Finish reason should be ToolCalls.
    let finish = parts.last().expect("should have finish");
    match finish {
        StreamPart::Finish { finish_reason, .. } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// TS: "should stream reasoning deltas"
#[tokio::test]
async fn should_stream_reasoning_deltas() {
    let server = MockServer::start().await;
    let sse = cohere_sse_body(&[
        r#"{"id":"c9117d7f","type":"message-start","delta":{"message":{"role":"assistant","content":[],"tool_plan":"","tool_calls":[],"citations":[]}}}"#,
        r#"{"type":"content-start","index":0,"delta":{"message":{"content":{"type":"thinking","thinking":""}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"thinking":"The user is asking"}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"thinking":" for 2+2."}}}}"#,
        r#"{"type":"content-end","index":0}"#,
        r#"{"type":"content-start","index":1,"delta":{"message":{"content":{"type":"text","text":""}}}}"#,
        r#"{"type":"content-delta","index":1,"delta":{"message":{"content":{"text":"2 + 2 = 4"}}}}"#,
        r#"{"type":"content-end","index":1}"#,
        r#"{"type":"message-end","delta":{"finish_reason":"COMPLETE","usage":{"billed_units":{"input_tokens":8,"output_tokens":50},"tokens":{"input_tokens":1394,"output_tokens":54},"cached_tokens":1360}}}"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    // Should have ReasoningStart, ReasoningDelta(s), ReasoningEnd.
    let reasoning_deltas: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ReasoningDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning_deltas, vec!["The user is asking", " for 2+2."]);

    // Should also have text deltas.
    let text_deltas: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_deltas, vec!["2 + 2 = 4"]);

    // Should have ReasoningStart and ReasoningEnd.
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

/// TS: "should pass the messages and the model" (streaming request body)
#[tokio::test]
async fn should_send_streaming_request_body() {
    let server = MockServer::start().await;
    let sse = cohere_sse_body(&[
        r#"{"id":"test","type":"message-start","delta":{"message":{"role":"assistant","content":[]}}}"#,
        r#"{"type":"content-start","index":0,"delta":{"message":{"content":{"type":"text","text":""}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":"Hi"}}}"#,
        r#"{"type":"content-end","index":0}"#,
        r#"{"type":"message-end","delta":{"finish_reason":"COMPLETE","usage":{"billed_units":{"input_tokens":1,"output_tokens":1},"tokens":{"input_tokens":1,"output_tokens":1}}}}"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let body = result.request_body.expect("body");
    assert_eq!(body["model"], "command-r-plus");
    assert_eq!(body["stream"], json!(true));
    assert_eq!(
        body["messages"],
        json!([
            { "role": "system", "content": "you are a friendly bot!" },
            { "role": "user", "content": "Hello" }
        ])
    );
}

/// TS: "should handle unparsable stream parts"
#[tokio::test]
async fn should_handle_unparsable_stream_parts() {
    let server = MockServer::start().await;
    let sse = "event: foo-message\ndata: {unparsable}\n\n";
    mock_sse_response(&server, sse).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;

    // Should have StreamStart, Error, Finish.
    assert!(matches!(
        parts.first(),
        Some(StreamPart::StreamStart { .. })
    ));
    assert!(parts.iter().any(|p| matches!(p, StreamPart::Error { .. })));
    let finish = parts.last().expect("should have finish");
    assert!(matches!(finish, StreamPart::Finish { .. }));
}

/// TS: "should handle 401 auth error"
#[tokio::test]
async fn should_handle_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({ "message": "Invalid API key" })),
        )
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model.do_generate(&default_options(test_prompt())).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.status_code(), Some(401), "got {err:?}");
    assert!(err.to_string().contains("Invalid API key"));
}

// ════════════════════════════════════════════════════════════════════════════
// Additional doGenerate cases — headers, response format, response metadata,
// tool_choice variants, file→documents, full request body, errors.
// ════════════════════════════════════════════════════════════════════════════

/// A minimal Cohere "ok" response reused by request-body assertions.
fn ok_cohere_body() -> Value {
    json!({
        "id": "test-id",
        "generation_id": "gen-123",
        "message": {
            "role": "assistant",
            "content": [{ "type": "text", "text": "ok" }]
        },
        "finish_reason": "COMPLETE",
        "usage": {
            "billed_units": { "input_tokens": 12, "output_tokens": 7 },
            "tokens": { "input_tokens": 12, "output_tokens": 7 }
        }
    })
}

/// TS: "should pass headers" — request-level custom headers reach the server.
#[tokio::test]
async fn should_pass_request_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .and(wiremock::matchers::header(
            "custom-request-header",
            "request-header-value",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_cohere_body()))
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

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
        .and(path("/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(ok_cohere_body()),
        )
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

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

/// TS: "should send additional response information" — generation_id → response.id
#[tokio::test]
async fn should_send_additional_response_information() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.response.id.as_deref(), Some("gen-123"));
}

/// TS: "should pass response format" — JSON schema → Cohere response_format.
#[tokio::test]
async fn should_pass_response_format() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let schema = json!({
        "type": "object",
        "properties": { "text": { "type": "string" } },
        "required": ["text"]
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
    assert_eq!(
        body["response_format"],
        json!({ "type": "json_object", "json_schema": schema })
    );
}

/// TS: "should pass tools" with tool_choice required → "REQUIRED"
#[tokio::test]
async fn should_pass_tools_with_tool_choice_required() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let options = CallOptions {
        prompt: test_prompt(),
        tools: Some(vec![test_tool()]),
        tool_choice: ToolChoice::Required,
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["tool_choice"], json!("REQUIRED"));
    assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
}

/// TS: "should pass tools" with tool_choice tool → "REQUIRED" + filtered tools
#[tokio::test]
async fn should_pass_tools_with_tool_choice_tool() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

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
    assert_eq!(body["tool_choice"], json!("REQUIRED"));
    assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
}

/// TS: "should extract text documents and send to API" — non-image File parts
/// become Cohere RAG `documents`. The Rust `ContentPart::File` carries no
/// filename, so `title` is omitted (unlike TS).
#[tokio::test]
async fn should_extract_text_documents_from_file_parts() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("What does this say?"),
            ContentPart::file(
                b"This is a test document.".to_vec(),
                "text/plain".to_string(),
            ),
        ],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(
        body["documents"],
        json!([{ "data": { "text": "This is a test document." } }])
    );
    assert_eq!(body["messages"][0]["role"], json!("user"));
    assert_eq!(body["messages"][0]["content"], json!("What does this say?"));
}

// ════════════════════════════════════════════════════════════════════════════
// citations — translated from the TS `describe('citations', ...)` block.
//
// TS has 7 citations cases; `should_extract_text_documents_from_file_parts`
// above already covers "should extract text documents and send to API". The
// 6 tests below cover the remaining cases: response-side citation extraction,
// multiple/JSON/PDF/markdown file→document conversion, and the no-files path.
// ════════════════════════════════════════════════════════════════════════════

/// The Cohere `cohere-citations.json` fixture: an assistant message with three
/// TEXT_CONTENT citations, all sourced from `doc:0` (title `benefits.txt`).
fn cohere_citations_response() -> Value {
    let citation = |start: u64, end: u64, text: &str| {
        json!({
            "start": start,
            "end": end,
            "text": text,
            "sources": [{
                "type": "document",
                "id": "doc:0",
                "document": {
                    "id": "doc:0",
                    "text": "AI provides: 1. Automation of tasks 2. Better decision-making 3. Cost reduction",
                    "title": "benefits.txt"
                }
            }],
            "type": "TEXT_CONTENT"
        })
    };
    json!({
        "id": "68475c80-574b-4c65-98a4-e81cebab5dce",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "The key benefits mentioned in this document are:\n1. Automation of tasks\n2. Better decision-making\n3. Cost reduction"
            }],
            "citations": [
                citation(52, 71, "Automation of tasks"),
                citation(75, 97, "Better decision-making"),
                citation(101, 115, "Cost reduction")
            ]
        },
        "finish_reason": "COMPLETE",
        "usage": {
            "billed_units": { "input_tokens": 39, "output_tokens": 27 },
            "tokens": { "input_tokens": 1683, "output_tokens": 62 },
            "cached_tokens": 992
        }
    })
}

/// TS: "should extract citations from response" — each citation becomes a
/// `Source` content item. The Rust `GenerateContent::Source` variant carries no
/// `mediaType`/`providerMetadata`, so only `source_type`/`title` are asserted
/// (the rich start/end/text/sources/citationType metadata is a documented gap).
/// The TS id comes from a mocked `generateId`; Rust has no such hook, so ids are
/// positional (`citation-{i}`).
#[tokio::test]
async fn should_extract_citations_from_response() {
    let server = MockServer::start().await;
    mock_json_response(&server, cohere_citations_response()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("What are AI benefits?"),
            ContentPart::File {
                data: b"AI provides automation and efficiency.".to_vec(),
                media_type: "text/plain".to_string(),
                filename: Some("ai-benefits.txt".to_string()),
                provider_options: None,
            },
        ],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");

    // 1 text + 3 citation sources.
    assert_eq!(result.content.len(), 4);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(
                text,
                "The key benefits mentioned in this document are:\n\
                 1. Automation of tasks\n\
                 2. Better decision-making\n\
                 3. Cost reduction"
            );
        }
        other => panic!("expected Text, got {other:?}"),
    }
    for (i, c) in result.content[1..].iter().enumerate() {
        match c {
            GenerateContent::Source {
                id,
                source_type,
                url,
                title,
                ..
            } => {
                assert_eq!(id, &format!("citation-{i}"));
                assert_eq!(source_type, "document");
                assert_eq!(url, &None);
                assert_eq!(title, &Some("benefits.txt".to_string()));
            }
            other => panic!("expected Source, got {other:?}"),
        }
    }

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("COMPLETE"));
    assert_eq!(result.usage.input_tokens.total, Some(1683));
    assert_eq!(result.usage.output_tokens.total, Some(62));
}

/// TS: "should extract multiple text documents" — two file parts become two
/// RAG documents, each carrying its filename as `title`.
#[tokio::test]
async fn should_extract_multiple_text_documents() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("What do these documents say?"),
            ContentPart::File {
                data: b"First document content".to_vec(),
                media_type: "text/plain".to_string(),
                filename: Some("doc1.txt".to_string()),
                provider_options: None,
            },
            ContentPart::File {
                data: b"Second document content".to_vec(),
                media_type: "text/plain".to_string(),
                filename: Some("doc2.txt".to_string()),
                provider_options: None,
            },
        ],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["documents"],
        json!([
            { "data": { "text": "First document content", "title": "doc1.txt" } },
            { "data": { "text": "Second document content", "title": "doc2.txt" } }
        ])
    );
    assert_eq!(body["messages"][0]["role"], json!("user"));
    assert_eq!(
        body["messages"][0]["content"],
        json!("What do these documents say?")
    );
    assert_eq!(body["model"], json!("command-r-plus"));
}

/// TS: "should support JSON files" — an `application/json` file part becomes a
/// RAG document whose `text` is the raw JSON string and `title` is the filename.
#[tokio::test]
async fn should_support_json_files() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("What is in this JSON?"),
            ContentPart::File {
                data: b"{\"key\": \"value\"}".to_vec(),
                media_type: "application/json".to_string(),
                filename: Some("data.json".to_string()),
                provider_options: None,
            },
        ],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["documents"],
        json!([{ "data": { "text": "{\"key\": \"value\"}", "title": "data.json" } }])
    );
    assert_eq!(
        body["messages"][0]["content"],
        json!("What is in this JSON?")
    );
    assert_eq!(body["model"], json!("command-r-plus"));
}

/// TS: "should not include mediaType in the outgoing payload (category D)" — a
/// non-text/non-image file (PDF) still becomes a RAG document, and neither the
/// media type nor the `mediaType` key leaks into the serialized request body.
#[tokio::test]
async fn should_not_include_mediatype_in_outgoing_payload() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("What is this?"),
            ContentPart::File {
                data: b"Some file content".to_vec(),
                media_type: "application/pdf".to_string(),
                filename: Some("document.pdf".to_string()),
                provider_options: None,
            },
        ],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["documents"],
        json!([{ "data": { "text": "Some file content", "title": "document.pdf" } }])
    );
    let serialized = body.to_string();
    assert!(
        !serialized.contains("application/pdf"),
        "mediaType must not leak into payload: {serialized}"
    );
    assert!(
        !serialized.contains("mediaType"),
        "mediaType key must not leak into payload: {serialized}"
    );
}

/// TS: "should successfully process supported text media types" — `text/plain`
/// and `text/markdown` files both become RAG documents with their content and
/// filename-derived title.
#[tokio::test]
async fn should_process_supported_text_media_types() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("What is this?"),
            ContentPart::File {
                data: b"This is plain text content".to_vec(),
                media_type: "text/plain".to_string(),
                filename: Some("text.txt".to_string()),
                provider_options: None,
            },
            ContentPart::File {
                data: b"# Markdown Header\nContent".to_vec(),
                media_type: "text/markdown".to_string(),
                filename: Some("doc.md".to_string()),
                provider_options: None,
            },
        ],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["documents"],
        json!([
            { "data": { "text": "This is plain text content", "title": "text.txt" } },
            { "data": { "text": "# Markdown Header\nContent", "title": "doc.md" } }
        ])
    );
}

/// TS: "should not include documents parameter when no files present" — a plain
/// text prompt produces a request body with no `documents` key.
#[tokio::test]
async fn should_not_include_documents_when_no_files_present() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");
    let body = result.request_body.expect("body");

    assert!(
        body.get("documents").is_none(),
        "documents must be absent when no files are present: {body}"
    );
}

/// TS: "should send request body" — verify all standard optional fields.
#[tokio::test]
async fn should_send_full_request_body() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_cohere_body()).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let options = CallOptions {
        prompt: test_prompt(),
        max_output_tokens: Some(100),
        temperature: Some(0.5),
        top_p: Some(0.9),
        top_k: Some(40.0),
        seed: Some(7),
        stop_sequences: Some(vec!["END".to_string()]),
        frequency_penalty: Some(0.1),
        presence_penalty: Some(0.2),
        ..default_options(Vec::new())
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["model"], "command-r-plus");
    assert_eq!(body["max_tokens"], json!(100));
    assert!((body["temperature"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert!((body["p"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(body["k"], json!(40.0));
    assert_eq!(body["seed"], json!(7));
    assert_eq!(body["stop_sequences"], json!(["END"]));
    assert!((body["frequency_penalty"].as_f64().unwrap() - 0.1).abs() < 1e-6);
    assert!((body["presence_penalty"].as_f64().unwrap() - 0.2).abs() < 1e-6);
    assert!(body.get("tools").is_none());
    assert!(body.get("response_format").is_none());
}

/// TS: a 429 response maps to `AiMuxError::ApiCall` (429 in `status_code`).
#[tokio::test]
async fn should_handle_rate_limit_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(
            ResponseTemplate::new(429).set_body_json(json!({ "message": "Too many requests" })),
        )
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(429)),
        "expected a 429, got {result:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Additional doStream cases — headers, response headers, empty tool args.
// ════════════════════════════════════════════════════════════════════════════

/// A minimal Cohere streaming text body reused by streaming assertions.
fn cohere_text_sse() -> String {
    cohere_sse_body(&[
        r#"{"id":"test","type":"message-start","delta":{"message":{"role":"assistant","content":[]}}}"#,
        r#"{"type":"content-start","index":0,"delta":{"message":{"content":{"type":"text","text":""}}}}"#,
        r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":"Hi"}}}"#,
        r#"{"type":"content-end","index":0}"#,
        r#"{"type":"message-end","delta":{"finish_reason":"COMPLETE","usage":{"billed_units":{"input_tokens":1,"output_tokens":1},"tokens":{"input_tokens":1,"output_tokens":1}}}}"#,
    ])
}

/// TS: "should pass headers" (streaming) — request-level custom headers.
#[tokio::test]
async fn should_pass_request_headers_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .and(wiremock::matchers::header(
            "custom-stream-header",
            "stream-value",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(cohere_text_sse()),
        )
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

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

/// TS: "should expose the raw response headers" (streaming)
#[tokio::test]
async fn should_expose_raw_response_headers_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .insert_header("test-header", "test-value")
                .set_body_string(cohere_text_sse()),
        )
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

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

/// TS: "should handle empty tool call arguments" (streaming) — empty arguments
/// string parses to an empty object.
#[tokio::test]
async fn should_stream_empty_tool_call_arguments() {
    let server = MockServer::start().await;
    let sse = cohere_sse_body(&[
        r#"{"id":"empty-args","type":"message-start","delta":{"message":{"role":"assistant","content":[],"tool_plan":"","tool_calls":[],"citations":[]}}}"#,
        r#"{"type":"tool-call-start","index":0,"delta":{"message":{"tool_calls":{"id":"tc_empty","type":"function","function":{"name":"doThing","arguments":""}}}}}"#,
        r#"{"type":"tool-call-end","index":0}"#,
        r#"{"type":"message-end","delta":{"finish_reason":"TOOL_CALL","usage":{"billed_units":{"input_tokens":5,"output_tokens":2},"tokens":{"input_tokens":5,"output_tokens":2}}}}"#,
    ]);
    mock_sse_response(&server, &sse).await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let options = CallOptions {
        tools: Some(vec![test_tool()]),
        ..default_options(test_prompt())
    };

    let result = model.do_stream(&options).await.expect("should succeed");
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
    assert_eq!(id, "tc_empty");
    assert_eq!(name, "doThing");
    assert_eq!(input, Value::String("{}".into()));

    let finish = parts.last().expect("should have finish");
    match finish {
        StreamPart::Finish { finish_reason, .. } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// TS: a 429 HTTP response surfaces as `AiMuxError::ApiCall` (429 in `status_code`) from do_stream.
#[tokio::test]
async fn should_stream_rate_limit_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(
            ResponseTemplate::new(429).set_body_json(json!({ "message": "Too many requests" })),
        )
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.model("command-r-plus");

    let result = model.do_stream(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(429)),
        "expected a 429, got {result:?}"
    );
}
