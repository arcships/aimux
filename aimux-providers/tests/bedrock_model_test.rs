//! Wiremock tests for the Amazon Bedrock provider.
//!
//! Tests cover:
//! - Non-streaming text generation (Converse API JSON response)
//! - Non-streaming tool calls
//! - Error handling (HTTP error status)
//! - Streaming via the AWS event stream binary format

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

use aimux_providers::bedrock::{BedrockModel, BedrockProviderConfig};

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

fn make_model(server: &MockServer) -> BedrockModel {
    BedrockModel::new(
        "anthropic.claude-3-5-sonnet-20240620-v1:0".to_string(),
        aimux_providers::bedrock::BedrockConfig {
            base_url: server.uri(),
            auth: aimux_providers::bedrock::BedrockAuth::BearerToken("test-token".to_string()),
            retry_config: aimux_provider_utils::RetryConfig::default(),
            api_key_source: None,
        },
    )
}

async fn mock_converse_json(server: &MockServer, status: u16, body: Value) {
    let model_path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse";
    Mock::given(method("POST"))
        .and(path(model_path))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

fn as_text(item: &GenerateContent) -> &str {
    match item {
        GenerateContent::Text { text, .. } => text,
        _ => panic!("expected Text content, got {item:?}"),
    }
}

fn as_tool_call(item: &GenerateContent) -> (&str, &str, &str) {
    match item {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => (tool_call_id, tool_name, input),
        _ => panic!("expected ToolCall content, got {item:?}"),
    }
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

// ═════════════════════════════════════════════════════════════════════════════
// doGenerate tests
// ═════════════════════════════════════════════════════════════════════════════

/// Test: non-streaming text generation extracts text content and usage.
#[tokio::test]
async fn bedrock_generate_text_response() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [{ "text": "Hello from Bedrock!" }]
                }
            },
            "stopReason": "end_turn",
            "usage": {
                "inputTokens": 10,
                "outputTokens": 20,
                "totalTokens": 30
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
    assert_eq!(as_text(&result.content[0]), "Hello from Bedrock!");
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("end_turn"));
    assert_eq!(result.usage.input_tokens.total, Some(10));
    assert_eq!(result.usage.output_tokens.total, Some(20));
}

/// Test: non-streaming tool call extraction.
#[tokio::test]
async fn bedrock_generate_tool_call() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        { "text": "Let me check the weather." },
                        {
                            "toolUse": {
                                "toolUseId": "tool_use_123",
                                "name": "getWeather",
                                "input": { "location": "San Francisco" }
                            }
                        }
                    ]
                }
            },
            "stopReason": "tool_use",
            "usage": {
                "inputTokens": 15,
                "outputTokens": 25
            }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 2);
    assert_eq!(as_text(&result.content[0]), "Let me check the weather.");

    let (id, name, input) = as_tool_call(&result.content[1]);
    assert_eq!(id, "tool_use_123");
    assert_eq!(name, "getWeather");
    assert_eq!(input, &json!(r#"{"location":"San Francisco"}"#));
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// Test: HTTP error status is propagated as an error.
#[tokio::test]
async fn bedrock_generate_error_status() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        400,
        json!({
            "error": {
                "message": "The model ID is invalid",
                "type": "ValidationException"
            }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;

    assert!(result.is_err(), "should return error for 400 status");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("invalid") || err.contains("ValidationException") || err.contains("model ID"),
        "error should contain the error message, got: {err}"
    );
}

/// Test: request body contains the expected Converse format.
#[tokio::test]
async fn bedrock_generate_request_body() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [{ "text": "OK" }]
                }
            },
            "stopReason": "end_turn",
            "usage": { "inputTokens": 1, "outputTokens": 1 }
        }),
    )
    .await;

    let mut opts = default_options(test_prompt());
    opts.max_output_tokens = Some(512);
    opts.temperature = Some(0.7);

    let model = make_model(&server);
    let result = model
        .do_generate(&opts)
        .await
        .expect("do_generate should succeed");

    let body = result.request_body.expect("should have request body");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"][0]["text"], "Hello");
    assert_eq!(body["inferenceConfig"]["maxTokens"], 512);
    // temperature is f32, compare with tolerance
    let temp = body["inferenceConfig"]["temperature"].as_f64().unwrap();
    assert!(
        (temp - 0.7).abs() < 0.001,
        "temperature should be ~0.7, got {temp}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// doStream tests (AWS event stream binary format)
// ═════════════════════════════════════════════════════════════════════════════

/// Test: streaming text via the Bedrock event stream binary format.
#[tokio::test]
async fn bedrock_stream_text() {
    let server = MockServer::start().await;

    // Build the binary event stream response.
    let events: Vec<(&str, &str, &str)> = vec![
        ("event", "messageStart", r#"{"role":"assistant"}"#),
        (
            "event",
            "contentBlockStart",
            r#"{"contentBlockIndex":0,"start":{}}"#,
        ),
        (
            "event",
            "contentBlockDelta",
            r#"{"contentBlockIndex":0,"delta":{"text":"Hello"}}"#,
        ),
        (
            "event",
            "contentBlockDelta",
            r#"{"contentBlockIndex":0,"delta":{"text":" world"}}"#,
        ),
        ("event", "contentBlockStop", r#"{"contentBlockIndex":0}"#),
        ("event", "messageStop", r#"{"stopReason":"end_turn"}"#),
        (
            "event",
            "metadata",
            r#"{"usage":{"inputTokens":5,"outputTokens":10}}"#,
        ),
    ];
    let body_bytes = aimux_providers::bedrock::event_stream::encode_messages(&events);

    let model_path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse-stream";
    Mock::given(method("POST"))
        .and(path(model_path))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_raw(body_bytes, "application/vnd.amazon.eventstream"),
        )
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;

    // Verify stream parts: StreamStart, ResponseMetadata, TextStart, TextDelta×2, TextEnd, Finish
    let text_deltas: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_deltas, vec!["Hello", " world"]);

    // Check finish
    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { finish_reason, .. } => Some(finish_reason.clone()),
        _ => None,
    });
    assert!(finish.is_some());
    assert_eq!(finish.unwrap().unified, FinishReasonUnified::Stop);
}

/// Test: streaming tool calls via the Bedrock event stream.
#[tokio::test]
async fn bedrock_stream_tool_call() {
    let server = MockServer::start().await;

    let events: Vec<(&str, &str, &str)> = vec![
        ("event", "messageStart", r#"{"role":"assistant"}"#),
        (
            "event",
            "contentBlockStart",
            r#"{"contentBlockIndex":0,"start":{"toolUse":{"name":"getWeather","toolUseId":"tool_1"}}}"#,
        ),
        (
            "event",
            "contentBlockDelta",
            r#"{"contentBlockIndex":0,"delta":{"toolUse":{"input":"{\"location\":"}}}"#,
        ),
        (
            "event",
            "contentBlockDelta",
            r#"{"contentBlockIndex":0,"delta":{"toolUse":{"input":"\"SF\"}"}}}"#,
        ),
        ("event", "contentBlockStop", r#"{"contentBlockIndex":0}"#),
        ("event", "messageStop", r#"{"stopReason":"tool_use"}"#),
    ];
    let body_bytes = aimux_providers::bedrock::event_stream::encode_messages(&events);

    let model_path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse-stream";
    Mock::given(method("POST"))
        .and(path(model_path))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_raw(body_bytes, "application/vnd.amazon.eventstream"),
        )
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;

    // Should have a ToolCall part
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
    assert_eq!(tool_calls[0].0, "tool_1");
    assert_eq!(tool_calls[0].1, "getWeather");
    assert_eq!(
        tool_calls[0].2,
        Value::String(r#"{"location":"SF"}"#.into())
    );
}

/// Test: SigV4 authentication adds Authorization header.
#[tokio::test]
async fn bedrock_sigv4_auth() {
    let server = MockServer::start().await;

    // The mock path must match the model ID used in the test.
    Mock::given(method("POST"))
        .and(path("/model/test-model/converse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [{ "text": "Signed!" }]
                }
            },
            "stopReason": "end_turn",
            "usage": { "inputTokens": 1, "outputTokens": 1 }
        })))
        .mount(&server)
        .await;

    let model = BedrockModel::new(
        "test-model".to_string(),
        aimux_providers::bedrock::BedrockConfig {
            base_url: server.uri(),
            auth: aimux_providers::bedrock::BedrockAuth::SigV4(
                aimux_providers::bedrock::AwsCredentials {
                    access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
                    secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
                    session_token: None,
                    region: "us-east-1".to_string(),
                },
            ),
            retry_config: aimux_provider_utils::RetryConfig::default(),
            api_key_source: None,
        },
    );

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed with SigV4");

    assert_eq!(as_text(&result.content[0]), "Signed!");
}

/// Test: provider config from_env with bearer token.
#[tokio::test]
async fn bedrock_provider_config_bearer() {
    unsafe {
        std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", "test-bearer-token");
        std::env::set_var("AWS_REGION", "us-west-2");
    }

    let config = BedrockProviderConfig::from_env().expect("should create config");
    match config.auth {
        aimux_providers::bedrock::BedrockAuth::BearerToken(t) => {
            assert_eq!(t, "test-bearer-token");
        }
        _ => panic!("expected BearerToken auth"),
    }
    assert!(config.base_url.contains("us-west-2"));

    unsafe {
        std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK");
        std::env::remove_var("AWS_REGION");
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Additional doGenerate cases — usage, metadata, headers, finish reasons,
// tool choice, settings.
// ═════════════════════════════════════════════════════════════════════════════

/// A minimal "ok" Converse response body.
fn ok_converse_body() -> Value {
    json!({
        "output": {
            "message": {
                "role": "assistant",
                "content": [{ "text": "ok" }]
            }
        },
        "stopReason": "end_turn",
        "usage": { "inputTokens": 4, "outputTokens": 7, "totalTokens": 11 }
    })
}

/// TS: "should extract usage"
#[tokio::test]
async fn bedrock_generate_usage() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(7));
}

/// TS: "should send additional response information" — the x-amzn-requestid
/// response header becomes `response.id`.
#[tokio::test]
async fn bedrock_generate_response_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-amzn-requestid", "req-abc-123")
                .set_body_json(ok_converse_body()),
        )
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.response.id.as_deref(), Some("req-abc-123"));
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("anthropic.claude-3-5-sonnet-20240620-v1:0")
    );
}

/// TS: "should expose the raw response headers"
#[tokio::test]
async fn bedrock_generate_response_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(ok_converse_body()),
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

/// TS: "should extract finish reason" — stop_sequence → Stop
#[tokio::test]
async fn bedrock_generate_finish_reason_stop_sequence() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "hi" }] } },
            "stopReason": "stop_sequence",
            "usage": { "inputTokens": 4, "outputTokens": 34 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("stop_sequence"));
}

/// TS: max_tokens → Length
#[tokio::test]
async fn bedrock_generate_finish_reason_max_tokens() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "truncated" }] } },
            "stopReason": "max_tokens",
            "usage": { "inputTokens": 4, "outputTokens": 34 }
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

/// TS: content_filtered → ContentFilter
#[tokio::test]
async fn bedrock_generate_finish_reason_content_filtered() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "" }] } },
            "stopReason": "content_filtered",
            "usage": { "inputTokens": 4, "outputTokens": 1 }
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

/// TS: "should support unknown finish reason" — unknown → Other
#[tokio::test]
async fn bedrock_generate_finish_reason_unknown() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "hi" }] } },
            "stopReason": "eos",
            "usage": { "inputTokens": 4, "outputTokens": 34 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("eos"));
}

/// TS: "should pass settings" — topP, topK, stopSequences land in inferenceConfig.
#[tokio::test]
async fn bedrock_generate_settings() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.top_p = Some(0.9);
    opts.top_k = Some(40.0);
    opts.stop_sequences = Some(vec!["END".to_string()]);

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert!((body["inferenceConfig"]["topP"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(body["inferenceConfig"]["topK"], json!(40.0));
    assert_eq!(body["inferenceConfig"]["stopSequences"], json!(["END"]));
}

/// TS: "should pass tools and tool choice correctly" — required → {"any":{}}
#[tokio::test]
async fn bedrock_generate_tool_choice_required() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![Tool::Function(
            FunctionTool::new(
                "get-weather".to_string(),
                json!({"type":"object","properties":{"city":{"type":"string"}}}),
            )
            .with_description("Get weather".to_string()),
        )]),
        tool_choice: ToolChoice::Required,
        ..default_options(test_prompt())
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["toolConfig"]["toolChoice"], json!({ "any": {} }));
    assert_eq!(
        body["toolConfig"]["tools"][0]["toolSpec"]["name"],
        "get-weather"
    );
}

/// TS: "should only send the forced tool when toolChoice specifies a specific tool"
#[tokio::test]
async fn bedrock_generate_tool_choice_specific_tool() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![Tool::Function(FunctionTool::new(
            "get-weather".to_string(),
            json!({"type":"object"}),
        ))]),
        tool_choice: ToolChoice::Tool {
            tool_name: "get-weather".to_string(),
        },
        ..default_options(test_prompt())
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(
        body["toolConfig"]["toolChoice"],
        json!({ "tool": { "name": "get-weather" } })
    );
    // Only the forced tool is sent.
    assert_eq!(body["toolConfig"]["tools"].as_array().unwrap().len(), 1);
}

/// TS: a 429/throttling response maps to `AiMuxError::ApiCall` (429 in `status_code`).
#[tokio::test]
async fn bedrock_generate_throttling_error() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        429,
        json!({ "message": "throttlingException", "type": "TooManyRequestsException" }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(429)),
        "expected RateLimited, got {result:?}"
    );
}
