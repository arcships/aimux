//! Rust port of the Anthropic provider `doGenerate` response-parsing tests and
//! `doStream` streaming tests.
//!
//! Translated from the Vercel AI SDK TypeScript test suite:
//! - `packages/anthropic/src/anthropic-language-model.test.ts`
//!   `describe('doGenerate')` response-parsing subset (text / usage /
//!   stop_reason / tool_calls / request body / headers / error status codes).
//! - `packages/anthropic/src/anthropic-language-model.test.ts`
//!   `describe('doStream')` streaming subset (text deltas, tool-call streaming,
//!   usage, finish-reason mapping, response metadata, ping, in-stream errors,
//!   pre-stream HTTP errors).
//!
//! HTTP is mocked with `wiremock` (a real loopback HTTP server), replacing the
//! TS MSW-based `createTestServer`. Each test starts its own `MockServer` so
//! parallel `#[tokio::test]` runs do not collide.
//!
//! Tests that depend on features absent from the Rust data model (reasoning /
//! thinking config, refusal `stop_details`, citations, provider-executed tools,
//! MCP servers, code execution, JSON output-format tool wrapping, `raw` usage
//! field, `providerMetadata` on the finish part) are intentionally omitted and
//! documented in the task summary.

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

use aimux_providers::anthropic::AnthropicConfig;
use aimux_providers::anthropic::model::AnthropicModel;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// Build `CallOptions` with everything unset except `prompt`.
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build an `AnthropicModel` whose base URL points at the wiremock server.
fn make_model(server: &MockServer) -> AnthropicModel {
    AnthropicModel::new(
        "claude-3-haiku-20240307".to_string(),
        AnthropicConfig::new("test-api-key").with_base_url(server.uri()),
    )
}

/// Mount a JSON response on `/v1/messages`.
async fn mock_json(server: &MockServer, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE stream response on `/v1/messages`.
async fn mock_sse(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

/// Format a single Anthropic SSE event as `data: {json}\n\n`.
fn sse(data: &Value) -> String {
    format!("data: {}\n\n", data)
}

/// Concatenate an ordered list of SSE events into one response body.
fn sse_stream(events: &[Value]) -> String {
    events.iter().map(sse).collect()
}

/// Drain a `StreamResult` into a `Vec<StreamPart>`, panicking on any error.
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

/// A minimal text response body (mirrors the TS `anthropic-text` shape).
fn text_response(text: &str) -> Value {
    json!({
        "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "text", "text": text }],
        "model": "claude-3-haiku-20240307",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": { "input_tokens": 4, "output_tokens": 30 },
    })
}

/// Helper to extract the text from a `GenerateContent::Text` item.
fn as_text(item: &GenerateContent) -> &str {
    match item {
        GenerateContent::Text { text, .. } => text,
        _ => panic!("expected Text content, got {:?}", item),
    }
}

/// Helper to destructure a `GenerateContent::ToolCall`.
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

/// A function tool with the common TS test schema `{ value: string }`.
fn value_tool() -> Tool {
    Tool::Function(FunctionTool::new(
        "test-tool".to_string(),
        json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"],
            "additionalProperties": false,
        }),
    ))
}

// ═════════════════════════════════════════════════════════════════════════════
// doGenerate — response parsing
// (anthropic-language-model.test.ts → describe('doGenerate'))
// ═════════════════════════════════════════════════════════════════════════════

mod do_generate {
    use super::*;

    // ── content extraction ──────────────────────────────────────────────────

    /// TS: "should extract text response"
    #[tokio::test]
    async fn should_extract_text_response() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            200,
            json!({
                "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "text", "text": "Hello, World!" }],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            }),
        )
        .await;

        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.content.len(), 1);
        assert_eq!(as_text(&result.content[0]), "Hello, World!");
    }

    /// TS: "should extract usage"
    #[tokio::test]
    async fn should_extract_usage() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            200,
            json!({
                "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "text", "text": "" }],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 20, "output_tokens": 5 },
            }),
        )
        .await;

        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.usage.input_tokens.total, Some(20));
        assert_eq!(result.usage.output_tokens.total, Some(5));
    }

    /// TS: "should send additional response information"
    #[tokio::test]
    async fn should_send_additional_response_information() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            200,
            json!({
                "id": "test-id",
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "text", "text": "" }],
                "model": "test-model",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            }),
        )
        .await;

        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.response.id.as_deref(), Some("test-id"));
        assert_eq!(result.response.model_id.as_deref(), Some("test-model"));
        assert!(result.response.timestamp.is_none());
    }

    /// TS: "should extract tool calls" — text + tool_use, finish_reason tool-calls.
    #[tokio::test]
    async fn should_extract_tool_calls() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            200,
            json!({
                "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "Some text\n\n" },
                    {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "test-tool",
                        "input": { "value": "example value" },
                    },
                ],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "tool_use",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            }),
        )
        .await;

        let model = make_model(&server);
        let opts = CallOptions {
            tools: Some(vec![value_tool()]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("do_generate ok");

        assert_eq!(result.content.len(), 2);
        assert_eq!(as_text(&result.content[0]), "Some text\n\n");
        let (id, name, input) = as_tool_call(&result.content[1]);
        assert_eq!(id, "toolu_1");
        assert_eq!(name, "test-tool");
        assert_eq!(input, &json!({ "value": "example value" }));

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("tool_use"));
    }

    // ── stop_reason mapping ─────────────────────────────────────────────────

    /// `end_turn` → `Stop`.
    #[tokio::test]
    async fn should_map_end_turn_to_stop() {
        let server = MockServer::start().await;
        mock_json(&server, 200, text_response("hi")).await;
        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("end_turn"));
    }

    /// `max_tokens` → `Length`.
    #[tokio::test]
    async fn should_map_max_tokens_to_length() {
        let server = MockServer::start().await;
        let mut body = text_response("hi");
        body["stop_reason"] = json!("max_tokens");
        mock_json(&server, 200, body).await;
        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Length);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("max_tokens"));
    }

    /// `tool_use` → `ToolCalls`.
    #[tokio::test]
    async fn should_map_tool_use_to_tool_calls() {
        let server = MockServer::start().await;
        let mut body = text_response("hi");
        body["stop_reason"] = json!("tool_use");
        mock_json(&server, 200, body).await;
        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("tool_use"));
    }

    /// Unknown stop_reason → `Other`.
    #[tokio::test]
    async fn should_map_unknown_stop_reason_to_other() {
        let server = MockServer::start().await;
        let mut body = text_response("hi");
        body["stop_reason"] = json!("something_new");
        mock_json(&server, 200, body).await;
        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("something_new"));
    }

    /// Missing stop_reason → `Other` with `raw: None` (current Rust default).
    #[tokio::test]
    async fn should_default_finish_reason_to_other_when_missing() {
        let server = MockServer::start().await;
        let mut body = text_response("hi");
        body["stop_reason"] = Value::Null;
        mock_json(&server, 200, body).await;
        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
        assert!(result.finish_reason.raw.is_none());
    }

    // ── multi-block content ─────────────────────────────────────────────────

    /// Multiple text blocks are preserved in order.
    #[tokio::test]
    async fn should_handle_multiple_text_blocks() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            200,
            json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "First" },
                    { "type": "text", "text": "Second" },
                ],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            }),
        )
        .await;
        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert_eq!(result.content.len(), 2);
        assert_eq!(as_text(&result.content[0]), "First");
        assert_eq!(as_text(&result.content[1]), "Second");
    }

    /// Multiple tool_use blocks (parallel tool calls) are preserved in order.
    #[tokio::test]
    async fn should_handle_multiple_tool_use_blocks() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            200,
            json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "toolu_a",
                        "name": "tool-a",
                        "input": { "x": 1 },
                    },
                    {
                        "type": "tool_use",
                        "id": "toolu_b",
                        "name": "tool-b",
                        "input": { "y": 2 },
                    },
                ],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "tool_use",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            }),
        )
        .await;
        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert_eq!(result.content.len(), 2);
        let (id_a, name_a, input_a) = as_tool_call(&result.content[0]);
        assert_eq!(id_a, "toolu_a");
        assert_eq!(name_a, "tool-a");
        assert_eq!(input_a, &json!({ "x": 1 }));
        let (id_b, name_b, input_b) = as_tool_call(&result.content[1]);
        assert_eq!(id_b, "toolu_b");
        assert_eq!(name_b, "tool-b");
        assert_eq!(input_b, &json!({ "y": 2 }));
    }

    /// Tool input may be a nested object.
    #[tokio::test]
    async fn should_handle_tool_use_with_complex_nested_input() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            200,
            json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "test-tool",
                    "input": {
                        "nested": { "arr": [1, 2, 3], "flag": true },
                        "items": ["a", "b"],
                    },
                }],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "tool_use",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            }),
        )
        .await;
        let model = make_model(&server);
        let opts = CallOptions {
            tools: Some(vec![value_tool()]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&opts).await.unwrap();
        let (_, _, input) = as_tool_call(&result.content[0]);
        assert_eq!(
            input,
            &json!({
                "nested": { "arr": [1, 2, 3], "flag": true },
                "items": ["a", "b"],
            })
        );
    }

    /// Empty content array → no content items, still a valid result.
    #[tokio::test]
    async fn should_handle_empty_content_array() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            200,
            json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            }),
        )
        .await;
        let model = make_model(&server);
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert!(result.content.is_empty());
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    }

    // ── request body & headers ──────────────────────────────────────────────

    /// The request body carries the model id and the single user message.
    #[tokio::test]
    async fn should_send_model_id_in_request_body() {
        let server = MockServer::start().await;
        mock_json(&server, 200, text_response("hi")).await;
        let model = make_model(&server);
        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "claude-3-haiku-20240307");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["text"], "Hello");
        // Non-streaming request sets stream: false.
        assert_eq!(body["stream"], false);
    }

    /// Anthropic system message is a top-level `system` field, not in messages.
    #[tokio::test]
    async fn should_send_system_message_as_top_level_field() {
        let server = MockServer::start().await;
        mock_json(&server, 200, text_response("hi")).await;
        let model = make_model(&server);

        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::System,
                content: vec![ContentPart::text("You are a helpful assistant.")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
        ];
        let _ = model.do_generate(&default_options(prompt)).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        // system is a top-level array of text blocks.
        assert_eq!(
            body["system"],
            json!([{ "type": "text", "text": "You are a helpful assistant." }])
        );
        // messages array must not contain a system role.
        for msg in body["messages"].as_array().unwrap() {
            assert_ne!(msg["role"], "system", "system leaked into messages");
        }
    }

    /// The x-api-key and anthropic-version headers are sent on every request.
    #[tokio::test]
    async fn should_send_api_key_and_version_headers() {
        let server = MockServer::start().await;
        mock_json(&server, 200, text_response("hi")).await;
        let model = make_model(&server);
        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let headers = &requests[0].headers;
        assert_eq!(
            headers.get("x-api-key").and_then(|v| v.to_str().ok()),
            Some("test-api-key")
        );
        assert_eq!(
            headers
                .get("anthropic-version")
                .and_then(|v| v.to_str().ok()),
            Some("2023-06-01")
        );
    }

    // ── error status codes ──────────────────────────────────────────────────

    /// 401 → `AiMuxError::ApiCall` (401 in `status_code`).
    #[tokio::test]
    async fn should_return_auth_error_on_401() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            401,
            json!({ "error": { "message": "invalid api key", "type": "authentication_error" } }),
        )
        .await;
        let model = make_model(&server);
        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("401 should error");
        assert!(matches!(err, ref e if e.status_code() == Some(401)));
    }

    /// 429 → `AiMuxError::ApiCall` (429 in `status_code`).
    #[tokio::test]
    async fn should_return_rate_limited_on_429() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            429,
            json!({ "error": { "message": "rate limited", "type": "rate_limit_error" } }),
        )
        .await;
        let model = make_model(&server);
        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("429 should error");
        assert!(matches!(err, ref e if e.status_code() == Some(429)));
    }

    /// 404 → `AiMuxError::ApiCall` (404 in `status_code`).
    #[tokio::test]
    async fn should_return_model_not_found_on_404() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            404,
            json!({ "error": { "message": "model not found", "type": "not_found_error" } }),
        )
        .await;
        let model = make_model(&server);
        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("404 should error");
        assert!(matches!(err, ref e if e.status_code() == Some(404)));
    }

    /// 500 → `AiMuxError::ApiCall`.
    #[tokio::test]
    async fn should_return_provider_error_on_500() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            500,
            json!({ "error": { "message": "internal error", "type": "api_error" } }),
        )
        .await;
        let model = make_model(&server);
        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("500 should error");
        assert!(matches!(err, ref e if e.status_code().is_some_and(|s| s >= 500)));
    }

    // ── request body options ────────────────────────────────────────────────

    /// Run `do_generate` with the given options against a fresh mock server and
    /// return the serialized request body.
    async fn gen_body(options: CallOptions) -> Value {
        let server = MockServer::start().await;
        mock_json(&server, 200, text_response("hi")).await;
        let model = make_model(&server);
        model.do_generate(&options).await.unwrap();
        let requests = server.received_requests().await.expect("requests recorded");
        serde_json::from_slice(&requests[0].body).unwrap()
    }

    #[tokio::test]
    async fn should_send_max_output_tokens() {
        let mut opts = default_options(test_prompt());
        opts.max_output_tokens = Some(1234);
        let body = gen_body(opts).await;
        assert_eq!(body["max_tokens"], 1234);
    }

    #[tokio::test]
    async fn should_default_max_tokens_to_4096_when_unset() {
        let body = gen_body(default_options(test_prompt())).await;
        assert_eq!(body["max_tokens"], 4096);
    }

    #[tokio::test]
    async fn should_send_temperature() {
        let mut opts = default_options(test_prompt());
        opts.temperature = Some(0.5);
        let body = gen_body(opts).await;
        assert_eq!(body["temperature"], 0.5);
    }

    #[tokio::test]
    async fn should_send_top_p() {
        let mut opts = default_options(test_prompt());
        opts.top_p = Some(0.25);
        let body = gen_body(opts).await;
        assert_eq!(body["top_p"], 0.25);
    }

    #[tokio::test]
    async fn should_send_stop_sequences() {
        let mut opts = default_options(test_prompt());
        opts.stop_sequences = Some(vec!["STOP".to_string(), "END".to_string()]);
        let body = gen_body(opts).await;
        assert_eq!(body["stop_sequences"], json!(["STOP", "END"]));
    }

    #[tokio::test]
    async fn should_send_tools_and_tool_choice_auto() {
        let mut opts = default_options(test_prompt());
        opts.tools = Some(vec![value_tool()]);
        opts.tool_choice = ToolChoice::Auto;
        let body = gen_body(opts).await;
        assert_eq!(body["tools"][0]["name"], "test-tool");
        assert_eq!(body["tool_choice"], json!({ "type": "auto" }));
    }

    #[tokio::test]
    async fn should_send_tool_choice_required_as_any() {
        let mut opts = default_options(test_prompt());
        opts.tools = Some(vec![value_tool()]);
        opts.tool_choice = ToolChoice::Required;
        let body = gen_body(opts).await;
        assert_eq!(body["tool_choice"], json!({ "type": "any" }));
    }

    #[tokio::test]
    async fn should_send_tool_choice_tool() {
        let mut opts = default_options(test_prompt());
        opts.tools = Some(vec![value_tool()]);
        opts.tool_choice = ToolChoice::Tool {
            tool_name: "test-tool".to_string(),
        };
        let body = gen_body(opts).await;
        assert_eq!(
            body["tool_choice"],
            json!({ "type": "tool", "name": "test-tool" })
        );
    }

    #[tokio::test]
    async fn should_omit_tools_and_tool_choice_when_no_tools() {
        let body = gen_body(default_options(test_prompt())).await;
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// doStream — streaming
// (anthropic-language-model.test.ts → describe('doStream'))
// ═════════════════════════════════════════════════════════════════════════════

mod do_stream {
    use super::*;

    /// The canonical Anthropic text-only stream (mirrors the `anthropic-text`
    /// fixture shape), with two text deltas.
    fn text_stream() -> String {
        sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 17, "output_tokens": 1 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":227}}),
            json!({"type":"message_stop"}),
        ])
    }

    // ── text streaming ──────────────────────────────────────────────────────

    /// TS: "raw chunks" text-only stream — stream-start, response-metadata,
    /// text-start, two text-deltas, text-end, finish.
    #[tokio::test]
    async fn should_stream_text_deltas() {
        let server = MockServer::start().await;
        mock_sse(&server, &text_stream()).await;
        let model = make_model(&server);
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Expected sequence: StreamStart, ResponseMetadata, TextStart,
        // TextDelta("Hello"), TextDelta("!"), TextEnd, Finish.
        assert_eq!(parts.len(), 7, "parts = {:?}", parts);

        assert!(matches!(
            &parts[0],
            StreamPart::StreamStart { warnings } if warnings.is_empty()
        ));
        match &parts[1] {
            StreamPart::ResponseMetadata { id, model_id, .. } => {
                assert_eq!(id.as_deref(), Some("msg_1"));
                assert_eq!(model_id.as_deref(), Some("claude-3-haiku-20240307"));
            }
            other => panic!("expected ResponseMetadata, got {:?}", other),
        }
        match &parts[2] {
            StreamPart::TextStart { id, .. } => assert_eq!(id, "0"),
            other => panic!("expected TextStart, got {:?}", other),
        }
        match &parts[3] {
            StreamPart::TextDelta { id, delta, .. } => {
                assert_eq!(id, "0");
                assert_eq!(delta, "Hello");
            }
            other => panic!("expected TextDelta Hello, got {:?}", other),
        }
        match &parts[4] {
            StreamPart::TextDelta { id, delta, .. } => {
                assert_eq!(id, "0");
                assert_eq!(delta, "!");
            }
            other => panic!("expected TextDelta !, got {:?}", other),
        }
        match &parts[5] {
            StreamPart::TextEnd { id, .. } => assert_eq!(id, "0"),
            other => panic!("expected TextEnd, got {:?}", other),
        }
        match &parts[6] {
            StreamPart::Finish { finish_reason, .. } => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
                assert_eq!(finish_reason.raw.as_deref(), Some("end_turn"));
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    /// Response metadata is emitted exactly once, right after message_start.
    #[tokio::test]
    async fn should_emit_response_metadata_after_message_start() {
        let server = MockServer::start().await;
        mock_sse(&server, &text_stream()).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;

        let meta_count = parts
            .iter()
            .filter(|p| matches!(p, StreamPart::ResponseMetadata { .. }))
            .count();
        assert_eq!(meta_count, 1);

        // The metadata part must come right after StreamStart.
        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));
        assert!(matches!(parts[1], StreamPart::ResponseMetadata { .. }));
    }

    /// Usage: input_tokens from message_start, output_tokens from message_delta.
    #[tokio::test]
    async fn should_extract_usage_from_message_start_and_message_delta() {
        let server = MockServer::start().await;
        mock_sse(&server, &text_stream()).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;

        let finish = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::Finish { usage, .. } => Some(usage),
                _ => None,
            })
            .expect("a Finish part");
        assert_eq!(finish.input_tokens.total, Some(17));
        // message_delta carries output_tokens: 227 (overrides message_start's 1).
        assert_eq!(finish.output_tokens.total, Some(227));
    }

    /// `end_turn` stop_reason in message_delta → `Stop`.
    #[tokio::test]
    async fn should_map_end_turn_stop_reason_in_stream() {
        let server = MockServer::start().await;
        mock_sse(&server, &text_stream()).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;
        let finish_reason = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
                _ => None,
            })
            .unwrap();
        assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
    }

    /// `tool_use` stop_reason in message_delta → `ToolCalls`.
    #[tokio::test]
    async fn should_map_tool_use_stop_reason_in_stream() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 10 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
            json!({"type":"message_stop"}),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;
        let finish_reason = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
                _ => None,
            })
            .unwrap();
        assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
    }

    /// When no message_delta carries a stop_reason, finish defaults to `Stop`.
    #[tokio::test]
    async fn should_default_finish_reason_to_stop_when_missing() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 10 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}),
            json!({"type":"content_block_stop","index":0}),
            // message_delta with no stop_reason.
            json!({"type":"message_delta","delta":{}}),
            json!({"type":"message_stop"}),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;
        let finish_reason = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
                _ => None,
            })
            .unwrap();
        assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
        assert!(finish_reason.raw.is_none());
    }

    // ── tool-call streaming ─────────────────────────────────────────────────

    /// TS: "should stream tool deltas" — text block then a tool_use block with
    /// input_json_delta fragments. Verifies ToolInputStart, ToolInputDelta*,
    /// ToolInputEnd, and the final ToolCall with the parsed JSON.
    #[tokio::test]
    async fn should_stream_tool_input_deltas() {
        let server = MockServer::start().await;

        // Five partial_json fragments that reconstruct
        // `{"value":"Sparkle Day"}`. Defined as plain Rust strings so the
        // escaping is unambiguous.
        let frags: [&str; 5] = ["{\"value", "\":", "\"Spark", "le", " Day\"}"];
        assert_eq!(frags.concat(), r#"{"value":"Sparkle Day"}"#);

        let mut events: Vec<Value> = vec![
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_01GouTqNCGXzrj5LQ5jEkw67",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 441, "output_tokens": 2 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"ping"}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Okay"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01DBsB4vvYLnBDzZ5rBSxSLs","name":"test-tool","input":{}}}),
            // Leading empty partial_json — must NOT produce a tool-input-delta.
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}),
        ];
        for frag in &frags {
            events.push(json!({
                "type": "content_block_delta",
                "index": 1,
                "delta": { "type": "input_json_delta", "partial_json": frag }
            }));
        }
        events.push(json!({"type":"content_block_stop","index":1}));
        events.push(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":65}}));
        events.push(json!({"type":"message_stop"}));

        let sse_body = sse_stream(&events);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let opts = CallOptions {
            tools: Some(vec![value_tool()]),
            ..default_options(test_prompt())
        };
        let parts = collect_stream(model.do_stream(&opts).await.unwrap()).await;

        // Collect the tool-input-delta fragments (excluding the empty one).
        let tool_deltas: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolInputDelta { id, delta, .. } => {
                    assert_eq!(id, "toolu_01DBsB4vvYLnBDzZ5rBSxSLs");
                    Some(delta.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            tool_deltas,
            frags.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );

        // ToolInputStart.
        assert!(parts.iter().any(|p| matches!(
            p,
            StreamPart::ToolInputStart { id, tool_name, .. }
                if id == "toolu_01DBsB4vvYLnBDzZ5rBSxSLs" && tool_name == "test-tool"
        )));

        // ToolInputEnd.
        assert!(parts.iter().any(|p| matches!(
            p,
            StreamPart::ToolInputEnd { id, .. }
                if id == "toolu_01DBsB4vvYLnBDzZ5rBSxSLs"
        )));

        // Final ToolCall carries the parsed accumulated JSON object.
        let tool_call = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    ..
                } => Some((tool_call_id, tool_name, input)),
                _ => None,
            })
            .expect("a ToolCall part");
        assert_eq!(tool_call.0, "toolu_01DBsB4vvYLnBDzZ5rBSxSLs");
        assert_eq!(tool_call.1, "test-tool");
        assert_eq!(tool_call.2, &json!({ "value": "Sparkle Day" }));

        // Finish reason reflects tool_use.
        let finish = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::Finish {
                    finish_reason,
                    usage,
                    ..
                } => Some((finish_reason, usage)),
                _ => None,
            })
            .unwrap();
        assert_eq!(finish.0.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(finish.1.input_tokens.total, Some(441));
        assert_eq!(finish.1.output_tokens.total, Some(65));
    }

    /// TS: "should support tools with empty parameters in streaming" — a
    /// tool_use block with only the leading empty `input_json_delta` produces
    /// a ToolCall whose input is an empty JSON object.
    #[tokio::test]
    async fn should_stream_tool_with_empty_input() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 565, "output_tokens": 7 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01QE","name":"updateIssueList","input":{}}}),
            // Only the leading empty partial_json — no real fragments.
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":48}}),
            json!({"type":"message_stop"}),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let opts = CallOptions {
            tools: Some(vec![value_tool()]),
            ..default_options(test_prompt())
        };
        let parts = collect_stream(model.do_stream(&opts).await.unwrap()).await;

        // No tool-input-delta should be emitted (the only fragment is empty).
        assert!(
            !parts
                .iter()
                .any(|p| matches!(p, StreamPart::ToolInputDelta { .. })),
            "no tool-input-delta expected, parts = {:?}",
            parts
        );

        let tool_call = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::ToolCall { input, .. } => Some(input.clone()),
                _ => None,
            })
            .expect("a ToolCall part");
        assert_eq!(tool_call, json!({}));
    }

    // ── misc SSE scenarios ──────────────────────────────────────────────────

    /// `ping` events are silently ignored (no stream part emitted).
    #[tokio::test]
    async fn should_handle_ping_events() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 10 },
                },
            }),
            json!({"type":"ping"}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"ping"}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}),
            json!({"type":"message_stop"}),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;

        // ping must not surface as any stream part; only the expected
        // text-stream sequence remains.
        let text_deltas: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(text_deltas, vec!["hi".to_string()]);
    }

    /// Two consecutive text blocks (index 0 then index 1) each get their own
    /// TextStart/TextDelta/TextEnd with the correct index-keyed id.
    #[tokio::test]
    async fn should_handle_multiple_text_blocks_in_sequence() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 10 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"A"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"B"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}),
            json!({"type":"message_stop"}),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;

        // Two TextStart ids "0" and "1", two TextEnd ids, deltas keyed by id.
        let starts: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextStart { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(starts, vec!["0".to_string(), "1".to_string()]);
        let ends: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextEnd { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ends, vec!["0".to_string(), "1".to_string()]);
    }

    /// A text block followed by a tool_use block (the canonical mixed response).
    #[tokio::test]
    async fn should_handle_text_then_tool_use() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 10 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Calling tool"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"test-tool","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"value\":"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"x\"}"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":12}}),
            json!({"type":"message_stop"}),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let opts = CallOptions {
            tools: Some(vec![value_tool()]),
            ..default_options(test_prompt())
        };
        let parts = collect_stream(model.do_stream(&opts).await.unwrap()).await;

        // Text block: TextStart(0) / TextDelta / TextEnd(0).
        assert!(parts.iter().any(|p| matches!(
            p,
            StreamPart::TextStart { id, .. } if id == "0"
        )));
        assert!(parts.iter().any(|p| matches!(
            p,
            StreamPart::TextEnd { id, .. } if id == "0"
        )));
        // Tool block: ToolInputStart / ToolInputDelta* / ToolInputEnd / ToolCall.
        let tool_call = parts
            .iter()
            .find_map(|p| match p {
                StreamPart::ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    ..
                } => Some((tool_call_id, tool_name, input)),
                _ => None,
            })
            .unwrap();
        assert_eq!(tool_call.0, "toolu_1");
        assert_eq!(tool_call.1, "test-tool");
        assert_eq!(tool_call.2, &json!({ "value": "x" }));
    }

    /// Two parallel tool_use blocks (index 0 and index 1) each produce their
    /// own ToolInputStart/Delta/End/ToolCall sequence.
    #[tokio::test]
    async fn should_stream_multiple_tool_calls_parallel() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 10 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"test-tool","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"a\":"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"1}"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_b","name":"test-tool","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"b\":"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"2}"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":20}}),
            json!({"type":"message_stop"}),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let opts = CallOptions {
            tools: Some(vec![value_tool()]),
            ..default_options(test_prompt())
        };
        let parts = collect_stream(model.do_stream(&opts).await.unwrap()).await;

        let tool_calls: Vec<(String, Value)> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolCall {
                    tool_call_id,
                    input,
                    ..
                } => Some((tool_call_id.clone(), input.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(tool_calls.len(), 2);
        assert_eq!(tool_calls[0].0, "toolu_a");
        assert_eq!(tool_calls[0].1, json!({ "a": 1 }));
        assert_eq!(tool_calls[1].0, "toolu_b");
        assert_eq!(tool_calls[1].1, json!({ "b": 2 }));
    }

    // ── pre-stream HTTP errors ──────────────────────────────────────────────

    /// 401 before streaming → `AiMuxError::ApiCall` (401 in `status_code`) (no stream is produced).
    #[tokio::test]
    async fn should_return_auth_error_on_401_before_streaming() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            401,
            json!({ "error": { "message": "invalid api key", "type": "authentication_error" } }),
        )
        .await;
        let model = make_model(&server);
        let err = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect_err("401 should error before streaming");
        assert!(matches!(err, ref e if e.status_code() == Some(401)));
    }

    /// 500 before streaming → `Provider` with a 5xx status.
    #[tokio::test]
    async fn should_return_provider_error_on_500_before_streaming() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            500,
            json!({ "error": { "message": "internal error", "type": "api_error" } }),
        )
        .await;
        let model = make_model(&server);
        let err = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect_err("500 should error before streaming");
        assert!(matches!(err, ref e if e.status_code().is_some_and(|s| s >= 500)));
    }

    // ── in-stream error event ───────────────────────────────────────────────

    /// An Anthropic mid-stream `error` event (e.g. `overloaded_error`) is
    /// surfaced as a `StreamPart::Error` and the stream terminates without a
    /// `Finish`. Mirrors the TS "first stream chunk is an overloaded error" /
    /// "forward overloaded error" cases.
    #[tokio::test]
    async fn should_forward_in_stream_error_event() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 10 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}),
            // Mid-stream error event.
            json!({
                "type": "error",
                "error": {
                    "type": "overloaded_error",
                    "message": "Overloaded",
                },
            }),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;

        // The stream must contain a StreamPart::Error carrying the provider
        // message, followed by an error-finish (Finish is the final-chunk
        // contract, matching openai/google — see #112).
        let error_idx = parts
            .iter()
            .position(|p| matches!(p, StreamPart::Error { .. }));
        let error_idx = error_idx.expect("an Error stream part");
        match &parts[error_idx] {
            StreamPart::Error { error } => {
                let msg = error.to_string();
                assert!(msg.contains("Overloaded"), "msg = {}", msg);
            }
            _ => unreachable!(),
        }
        // Error is followed by exactly one Finish with unified=Error.
        assert_eq!(parts.len(), error_idx + 2);
        assert!(matches!(
            &parts[error_idx + 1],
            StreamPart::Finish { finish_reason, .. }
                if matches!(finish_reason.unified, FinishReasonUnified::Error)
        ));
    }

    /// Extended-thinking streams: `signature_delta` pieces accumulate on the
    /// thinking block and are attached to the concluding ReasoningEnd as
    /// provider_metadata (`{"anthropic": {"signature": ..}}`), matching the
    /// non-streaming path (#113).
    #[tokio::test]
    async fn should_attach_thinking_signature_to_reasoning_end() {
        let server = MockServer::start().await;
        let sse_body = sse_stream(&[
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": "claude-3-haiku-20240307",
                    "usage": { "input_tokens": 10 },
                },
            }),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think."}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-part-1"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-part-2"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}),
            json!({"type":"message_stop"}),
        ]);
        mock_sse(&server, &sse_body).await;
        let model = make_model(&server);
        let parts = collect_stream(
            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap(),
        )
        .await;

        assert!(
            parts.iter().any(|p| matches!(p,
                StreamPart::ReasoningDelta { delta, .. } if delta == "Let me think.")),
            "expected the thinking text delta, got {parts:?}"
        );
        let meta = parts.iter().find_map(|p| match p {
            StreamPart::ReasoningEnd {
                provider_metadata, ..
            } => provider_metadata.clone(),
            _ => None,
        });
        let meta = meta.expect("ReasoningEnd with provider_metadata");
        // Incremental signature pieces are concatenated.
        assert_eq!(
            meta["anthropic"]["signature"].as_str(),
            Some("sig-part-1sig-part-2")
        );
    }
}
