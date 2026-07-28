//! Wiremock tests for Anthropic partner models on Vertex AI (`rawPredict`).
//!
//! Claude models on Vertex AI are reached via the
//! `publishers/anthropic/models/{model}:rawPredict` (and `:streamRawPredict`)
//! endpoints. The request body is a standard Anthropic Messages request
//! wrapped in an `anthropic_version` envelope (model identity in the URL, not
//! the body); the response is a verbatim Anthropic Messages response. These
//! tests mock the Vertex AI endpoints and never touch the public network or
//! real credentials.

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
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

use aimux_providers::vertex::{
    VertexAnthropicModel, VertexAuth, VertexProvider, VertexProviderConfig,
};

// ── Constants ─────────────────────────────────────────────────────────────────

const MODEL_ID: &str = "claude-sonnet-4-20250514";
/// Endpoint path once the `/publishers/google` suffix has been swapped for
/// `/publishers/anthropic`.
const RAW_PREDICT_PATH: &str = "/publishers/anthropic/models/claude-sonnet-4-20250514:rawPredict";
const STREAM_RAW_PREDICT_PATH: &str =
    "/publishers/anthropic/models/claude-sonnet-4-20250514:streamRawPredict";

// ── Helpers ───────────────────────────────────────────────────────────────────

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

/// Build a `VertexProvider` whose base URL ends in `/publishers/google` and
/// points at the mock server, then return the Anthropic partner model. This
/// exercises the `/publishers/google` → `/publishers/anthropic` base-URL
/// rewrite performed by [`VertexProvider::anthropic_model`].
fn make_model(server: &MockServer) -> VertexAnthropicModel {
    let config = VertexProviderConfig {
        base_url: format!("{}/publishers/google", server.uri()),
        project: Some("test-project".to_string()),
        location: Some("us-central1".to_string()),
        auth: VertexAuth::BearerToken("test-token".to_string()),
    };
    VertexProvider::new(config)
        .anthropic_model(MODEL_ID)
        .expect("anthropic_model should succeed")
}

async fn mock_raw_predict_json(server: &MockServer, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path(RAW_PREDICT_PATH))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_stream_raw_predict_sse(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path(STREAM_RAW_PREDICT_PATH))
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
        GenerateContent::Text { text } => text,
        _ => panic!("expected Text content, got {:?}", item),
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

/// Test: non-streaming text generation via rawPredict extracts text, usage,
/// and finish reason from the standard Anthropic response body.
#[tokio::test]
async fn vertex_anthropic_generate_text_response() {
    let server = MockServer::start().await;
    mock_raw_predict_json(&server, 200, text_response("Hello from Claude on Vertex!")).await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    assert_eq!(as_text(&result.content[0]), "Hello from Claude on Vertex!");
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(30));
    assert_eq!(
        result.response.id.as_deref(),
        Some("msg_017TfcQ4AgGxKyBduUpqYPZn")
    );
}

/// Test: the request hits the `/publishers/anthropic/models/{model}:rawPredict`
/// path (i.e. the `/publishers/google` suffix is replaced) and carries the
/// Bearer token Authorization header.
#[tokio::test]
async fn vertex_anthropic_url_and_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(RAW_PREDICT_PATH))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_response("OK")))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(as_text(&result.content[0]), "OK");

    // The single recorded request hit the Anthropic publisher path.
    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), RAW_PREDICT_PATH);
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer test-token"
    );
}

/// Test: the rawPredict request body is wrapped in the `anthropic_version`
/// envelope and drops the `model` field (the model identity lives in the URL).
#[tokio::test]
async fn vertex_anthropic_request_body_envelope() {
    let server = MockServer::start().await;
    mock_raw_predict_json(&server, 200, text_response("OK")).await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    // The reported request body is the envelope actually sent.
    let reported = result.request_body.expect("should have request body");
    assert_eq!(reported["anthropic_version"], "vertex-2023-10-16");
    assert_eq!(reported["messages"][0]["role"], "user");
    assert_eq!(reported["messages"][0]["content"][0]["text"], "Hello");
    assert!(
        reported.get("model").is_none(),
        "model must not be in the body"
    );

    // The body sent over the wire matches (anthropic_version present, no model).
    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let wire: Value = serde_json::from_slice(&requests[0].body).expect("valid json body");
    assert_eq!(wire["anthropic_version"], "vertex-2023-10-16");
    assert_eq!(wire["messages"][0]["content"][0]["text"], "Hello");
    assert!(
        wire.get("model").is_none(),
        "model must not be in the wire body"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// doStream tests
// ═════════════════════════════════════════════════════════════════════════════

/// Test: streaming text via the Anthropic SSE events returned by
/// streamRawPredict.
#[tokio::test]
async fn vertex_anthropic_stream_text() {
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

    mock_stream_raw_predict_sse(&server, &sse_body).await;

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
    let finish = finish.expect("should have a Finish part");
    assert_eq!(finish.unified, FinishReasonUnified::Stop);

    // The stream request hit the streamRawPredict path.
    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), STREAM_RAW_PREDICT_PATH);
    let wire: Value = serde_json::from_slice(&requests[0].body).expect("valid json body");
    assert_eq!(wire["anthropic_version"], "vertex-2023-10-16");
    assert_eq!(wire["stream"], true);
}

// ═════════════════════════════════════════════════════════════════════════════
// Error handling
// ═════════════════════════════════════════════════════════════════════════════

/// Test: a 401 response maps to `AiMuxError::Auth`.
#[tokio::test]
async fn vertex_anthropic_generate_auth_error() {
    let server = MockServer::start().await;
    mock_raw_predict_json(
        &server,
        401,
        json!({
            "error": {
                "code": 401,
                "message": "Request had invalid authentication credentials.",
                "status": "UNAUTHENTICATED"
            }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(_))),
        "expected Auth, got {result:?}"
    );
}

/// Test: a generic HTTP error (500) maps to `AiMuxError::Provider`.
#[tokio::test]
async fn vertex_anthropic_generate_provider_error() {
    let server = MockServer::start().await;
    mock_raw_predict_json(
        &server,
        500,
        json!({
            "error": {
                "code": 500,
                "message": "Internal error.",
                "status": "INTERNAL"
            }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(AiMuxError::Provider(_))),
        "expected Provider, got {result:?}"
    );
}
