//! Tests for the Codex provider (RFC-0018).
//!
//! Path A (API-key mode): non-streaming, streaming, and tool-call paths over
//! the standard Responses endpoint, asserting delegation to the shared
//! OpenAI Responses channel.
//!
//! Path B (subscription mode): forced `stream: true` / `store: false`,
//! channel headers (`Originator` / `ChatGPT-Account-Id`), 401 →//! `AiMuxError::TokenExpired` mapping, and the stateless `codex_refresh`
//! OAuth helper.

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, Tool, ToolChoice};
use aimux_core::result::StreamResult;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::{CODEX_OAUTH_TOKEN_URL, CodexConfig, CodexProvider, codex_refresh_at};

// helpers

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

fn weather_tool() -> FunctionTool {
    FunctionTool {
        name: "weather".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false,
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(&sse_event(event));
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Concatenate string SSE events (for events built at runtime).
fn sse_body_strings(events: &[String]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

async fn mock_json_response(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_sse_response(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
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

async fn first_request(server: &MockServer) -> (Value, wiremock::http::HeaderMap) {
    let requests = server
        .received_requests()
        .await
        .expect("no requests received");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("invalid JSON body");
    (body, requests[0].headers.clone())
}

/// A basic text response body (single message output item).
fn text_response_body() -> Value {
    json!({
        "id": "resp_codex_1",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "error": null,
        "incomplete_details": null,
        "model": "gpt-5.2-codex",
        "output": [
            {
                "id": "msg_1",
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [
                    { "type": "output_text", "text": "answer text", "annotations": [] }
                ]
            }
        ],
        "usage": {
            "input_tokens": 345,
            "input_tokens_details": { "cached_tokens": 234 },
            "output_tokens": 538,
            "output_tokens_details": { "reasoning_tokens": 123 }
        },
        "reasoning": { "effort": null, "summary": null, "context": "current_turn" }
    })
}

/// The text streaming event sequence (RFC-0018 acceptance: streaming path).
const TEXT_STREAM_EVENTS: &[&str] = &[
    r#"{"type":"response.created","response":{"id":"resp_1","created_at":1741269019,"model":"gpt-5.2-codex"}}"#,
    r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
    r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"Hello,"}"#,
    r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":" World!"}"#,
    r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[]}}"#,
    r#"{"type":"response.completed","response":{"id":"resp_1","created_at":1741269019,"model":"gpt-5.2-codex","incomplete_details":null,"usage":{"input_tokens":543,"input_tokens_details":{"cached_tokens":234},"output_tokens":478,"output_tokens_details":{"reasoning_tokens":123}}}}"#,
];

/// The tool-call streaming event sequence.
const TOOL_STREAM_EVENTS: &[&str] = &[
    r#"{"type":"response.created","response":{"id":"resp_tc","created_at":1741362087,"model":"gpt-5.2-codex"}}"#,
    r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_added","name":"weather","arguments":"","status":"completed"}}"#,
    r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"{\"location\":"}"#,
    r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"\"Rome\"}"}"#,
    r#"{"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":0,"arguments":"{\"location\":\"Rome\"}"}"#,
    r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_done","name":"weather","arguments":"{\"location\":\"Rome\"}","status":"completed"}}"#,
    r#"{"type":"response.completed","response":{"id":"resp_tc","created_at":1741362087,"model":"gpt-5.2-codex","incomplete_details":null,"usage":{"input_tokens":0,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0}}}}"#,
];

/// A `response.completed` event whose nested response object carries the full
/// output —what the subscription channel emits at the end of a stream.
fn subscription_completed_event() -> String {
    sse_event(
        &json!({ "type": "response.completed", "response": text_response_body() }).to_string(),
    )
}

// Path A: API-key mode

#[tokio::test]
async fn api_key_generates_text() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = CodexConfig::new("test-key").with_base_url(server.uri());
    let model = CodexProvider::new(config).model("gpt-5.2-codex");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    match &result.content[0] {
        aimux_core::result::GenerateContent::Text { text, .. } => {
            assert_eq!(text, "answer text")
        }
        other => panic!("expected Text, got {other:?}"),
    }
    assert_eq!(model.provider(), "codex");
    assert_eq!(model.model_id(), "gpt-5.2-codex");

    // Request shape: Responses endpoint, bearer auth, non-streaming body
    // (the `stream` key is absent for non-streaming requests).
    let (body, headers) = first_request(&server).await;
    assert_eq!(body["model"], "gpt-5.2-codex");
    assert_ne!(body["stream"], json!(true));
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(
        headers.get("authorization").map(|v| v.to_str().unwrap()),
        Some("Bearer test-key")
    );
}

#[tokio::test]
async fn api_key_streams_text() {
    let server = MockServer::start().await;
    mock_sse_response(&server, &sse_body(TEXT_STREAM_EVENTS)).await;

    let config = CodexConfig::new("test-key").with_base_url(server.uri());
    let model = CodexProvider::new(config).model("gpt-5.2-codex");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");
    let parts = collect_stream(result).await;

    assert_eq!(parts.len(), 7);
    assert!(matches!(
        &parts[0],
        StreamPart::StreamStart { warnings } if warnings.is_empty()
    ));
    match &parts[2] {
        StreamPart::TextStart { id, .. } => assert_eq!(id, "msg_1"),
        other => panic!("expected TextStart, got {other:?}"),
    }
    match &parts[3] {
        StreamPart::TextDelta { delta, .. } => assert_eq!(delta, "Hello,"),
        other => panic!("expected TextDelta, got {other:?}"),
    }
    match &parts[6] {
        StreamPart::Finish { usage, .. } => assert_eq!(usage.input_tokens.total, Some(543)),
        other => panic!("expected Finish, got {other:?}"),
    }
}

#[tokio::test]
async fn api_key_streams_tool_calls() {
    let server = MockServer::start().await;
    mock_sse_response(&server, &sse_body(TOOL_STREAM_EVENTS)).await;

    let config = CodexConfig::new("test-key").with_base_url(server.uri());
    let model = CodexProvider::new(config).model("gpt-5.2-codex");

    let options = CallOptions {
        tools: Some(vec![Tool::from(weather_tool())]),
        tool_choice: ToolChoice::Auto,
        ..CallOptions::new(test_prompt())
    };

    let result = model.do_stream(&options).await.expect("should succeed");
    let parts = collect_stream(result).await;

    match &parts[6] {
        StreamPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            assert_eq!(tool_call_id, "call_done");
            assert_eq!(tool_name, "weather");
            assert_eq!(input, &Value::String(r#"{"location":"Rome"}"#.into()));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    match &parts[7] {
        StreamPart::Finish { finish_reason, .. } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

// Path B: subscription mode

#[tokio::test]
async fn subscription_generate_forces_stream_and_assembles_result() {
    let server = MockServer::start().await;
    // The subscription endpoint only streams: text deltas + a final
    // response.completed carrying the full response object.
    let events: Vec<String> = TEXT_STREAM_EVENTS[..5]
        .iter()
        .map(|e| sse_event(e))
        .chain(std::iter::once(subscription_completed_event()))
        .collect();
    mock_sse_response(&server, &sse_body_strings(&events)).await;

    let config = CodexConfig::subscription("account-token")
        .with_base_url(server.uri())
        .with_chatgpt_account_id("acct_123")
        .with_originator("aimux-test");
    let model = CodexProvider::new(config).model("gpt-5.2-codex");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("subscription generate should succeed");

    match &result.content[0] {
        aimux_core::result::GenerateContent::Text { text, .. } => {
            assert_eq!(text, "answer text")
        }
        other => panic!("expected Text, got {other:?}"),
    }

    // The request must be a *streaming* request with store disabled.
    let (body, headers) = first_request(&server).await;
    assert_eq!(
        body["stream"], true,
        "subscription channel must always stream"
    );
    assert_eq!(
        body["store"], false,
        "subscription channel must never store"
    );

    // Channel headers: bearer account token + originator + account id.
    assert_eq!(
        headers.get("authorization").map(|v| v.to_str().unwrap()),
        Some("Bearer account-token")
    );
    assert_eq!(
        headers.get("originator").map(|v| v.to_str().unwrap()),
        Some("aimux-test")
    );
    assert_eq!(
        headers
            .get("chatgpt-account-id")
            .map(|v| v.to_str().unwrap()),
        Some("acct_123")
    );
}

#[tokio::test]
async fn subscription_stream_forces_store_false() {
    let server = MockServer::start().await;
    mock_sse_response(&server, &sse_body(TEXT_STREAM_EVENTS)).await;

    let config = CodexConfig::subscription("account-token").with_base_url(server.uri());
    let model = CodexProvider::new(config).model("gpt-5.2-codex");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");
    let parts = collect_stream(result).await;
    assert_eq!(parts.len(), 7);

    let (body, _) = first_request(&server).await;
    assert_eq!(body["stream"], true);
    assert_eq!(body["store"], false);
}

#[tokio::test]
async fn subscription_user_headers_win_over_defaults() {
    let server = MockServer::start().await;
    mock_sse_response(&server, &sse_body(TEXT_STREAM_EVENTS)).await;

    let config = CodexConfig::subscription("account-token")
        .with_base_url(server.uri())
        .with_originator("library-default");
    let model = CodexProvider::new(config).model("gpt-5.2-codex");

    let options = CallOptions {
        headers: Some(
            [("Originator".to_string(), "user-value".to_string())]
                .into_iter()
                .collect(),
        ),
        ..CallOptions::new(test_prompt())
    };

    model.do_stream(&options).await.expect("should succeed");
    let (_, headers) = first_request(&server).await;
    assert_eq!(
        headers.get("originator").map(|v| v.to_str().unwrap()),
        Some("user-value"),
        "explicit user headers must win over subscription defaults"
    );
}

#[tokio::test]
async fn subscription_401_maps_to_token_expired() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({ "error": { "message": "invalid token" } })),
        )
        .mount(&server)
        .await;

    let config = CodexConfig::subscription("expired-token").with_base_url(server.uri());
    let model = CodexProvider::new(config).model("gpt-5.2-codex");
    let options = default_options(test_prompt());

    let gen_err = model
        .do_generate(&options)
        .await
        .expect_err("401 must fail");
    assert!(
        matches!(gen_err, AiMuxError::TokenExpired(_)),
        "do_generate: expected TokenExpired, got {gen_err:?}"
    );
    assert!(!gen_err.is_retryable());

    let stream_err = model.do_stream(&options).await.expect_err("401 must fail");
    assert!(
        matches!(stream_err, AiMuxError::TokenExpired(_)),
        "do_stream: expected TokenExpired, got {stream_err:?}"
    );
}

// codex_refresh (stateless OAuth helper)

#[tokio::test]
async fn codex_refresh_exchanges_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "new-access",
            "refresh_token": "new-refresh",
            "expires_in": 3600,
            "token_type": "Bearer",
        })))
        .mount(&server)
        .await;

    let tokens = codex_refresh_at(
        "old-refresh",
        "test-client",
        &format!("{}/oauth/token", server.uri()),
    )
    .await
    .expect("refresh should succeed");

    assert_eq!(tokens.access_token, "new-access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("new-refresh"));
    assert_eq!(tokens.expires_in_secs, Some(3600));

    // Wire format: the refresh grant must carry the old refresh token.
    let requests = server.received_requests().await.expect("request received");
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["grant_type"], "refresh_token");
    assert_eq!(sent["refresh_token"], "old-refresh");
    assert_eq!(sent["client_id"], "test-client");

    // The production helper targets the official endpoint.
    assert_eq!(CODEX_OAUTH_TOKEN_URL, "https://auth.openai.com/oauth/token");
}

#[tokio::test]
async fn codex_refresh_never_retries() {
    // Refresh tokens rotate on first use: a retry of a lost response would
    // burn the rotation (`refresh_token_reused`). Exactly one request even on
    // a retryable-looking status.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;

    let err = codex_refresh_at(
        "old-refresh",
        "test-client",
        &format!("{}/oauth/token", server.uri()),
    )
    .await
    .expect_err("429 must fail");
    assert!(matches!(err, ref e if e.status_code() == Some(429)));
}

#[tokio::test]
async fn codex_refresh_rejects_bad_grant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant",
            "error_description": "refresh token is invalid or expired",
        })))
        .mount(&server)
        .await;

    let err = codex_refresh_at(
        "stale-refresh",
        "test-client",
        &format!("{}/oauth/token", server.uri()),
    )
    .await
    .expect_err("400 must fail");
    // parse_provider_error maps non-401/403 4xx to Provider (not retryable).
    assert!(matches!(err, AiMuxError::ApiCall(_)));
    assert!(!err.is_retryable());
}
