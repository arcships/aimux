//! Wiremock tests for the Azure OpenAI provider.
//!
//! These cover the Azure-specific differences from the OpenAI provider:
//! - deployment-based URL construction with an `api-version` query parameter
//! - `api-key` header authentication (API key)
//! - `Authorization: Bearer <token>` authentication (Azure AD token provider)
//! - request/response handling (reusing the OpenAI conversion logic)
//! - streaming
//! - error status codes
//! - config validation (missing resource/base URL, missing auth)
//!
//! Each test spins up a `wiremock` mock server, points an `AzureProvider` at
//! it via `base_url`, and asserts on the issued request and/or the parsed
//! result.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};

use aimux_providers::{AzureConfig, AzureProvider, TokenProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

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

/// Standard mock for a JSON chat completion response on a given path.
async fn mock_json_response(server: &MockServer, request_path: &str, body: Value) {
    Mock::given(method("POST"))
        .and(path(request_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mock for a JSON error response on a given path with a status code.
async fn mock_json_error(server: &MockServer, request_path: &str, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path(request_path))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// Standard mock for an SSE streaming response on a given path.
async fn mock_sse_response(server: &MockServer, request_path: &str, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path(request_path))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

/// A simple text chat-completion response body.
fn text_completion_response(text: &str) -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}

/// Concatenate SSE events and append `[DONE]`.
fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Collect all `StreamPart`s from a `StreamResult` into a `Vec`.
async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
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

/// Extract text deltas from a list of stream parts.
fn text_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

/// A token provider that returns a fixed token.
struct StaticToken(String);
#[async_trait]
impl TokenProvider for StaticToken {
    async fn get_token(&self) -> Result<String, AiMuxError> {
        Ok(self.0.clone())
    }
}

/// A token provider that returns a different token on each call
/// (`token-1`, `token-2`, …) so tests can assert it is invoked per request.
struct CountingToken {
    count: AtomicU32,
}
#[async_trait]
impl TokenProvider for CountingToken {
    async fn get_token(&self) -> Result<String, AiMuxError> {
        let n = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(format!("token-{}", n))
    }
}

// ════════════════════════════════════════════════════════════════════════════
// URL construction
// ════════════════════════════════════════════════════════════════════════════

/// The default deployment-based URL form places the deployment in the path and
/// appends `?api-version=2024-10-21` (the default api version).
#[tokio::test]
async fn should_build_deployment_based_url_with_default_api_version() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("hi"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let url = &requests[0].url;
    assert_eq!(url.path(), "/deployments/gpt-4o/chat/completions");
    assert_eq!(url.query(), Some("api-version=2024-10-21"));
}

/// A custom `api_version` is reflected in the `api-version` query parameter.
#[tokio::test]
async fn should_use_custom_api_version() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("hi"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key")
        .with_api_version("2025-04-01-preview");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].url.query(),
        Some("api-version=2025-04-01-preview")
    );
}

/// The v1 URL form uses `/v1/chat/completions` (no deployment in the path).
/// With a non-Azure gateway `base_url`, the gateway owns versioning and
/// `api-version` is omitted — mirroring the TS `useAzureOpenAIEndpoint` gate.
/// (The real-Azure v1 case — `api-version` appended — is covered by the unit
/// tests in `azure/model.rs`, which construct the URL from a `resource_name`
/// without needing a live HTTP endpoint.)
#[tokio::test]
async fn should_use_v1_url_form_on_gateway() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/v1/chat/completions",
        text_completion_response("hi"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key")
        .use_v1_urls();
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].url.path(), "/v1/chat/completions");
    // Non-Azure gateway owns its own versioning — no api-version query param.
    assert_eq!(requests[0].url.query(), None);
}

// ════════════════════════════════════════════════════════════════════════════
// Authentication
// ════════════════════════════════════════════════════════════════════════════

/// API-key auth sends the `api-key` header (not a Bearer token).
#[tokio::test]
async fn should_send_api_key_header() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("hi"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("api-key").and_then(|v| v.to_str().ok()),
        Some("test-api-key")
    );
    // No Authorization header when using api-key auth.
    assert!(headers.get("authorization").is_none());
}

/// A token provider supplies a Bearer token via the `Authorization` header.
#[tokio::test]
async fn should_send_bearer_token_from_token_provider() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("hi"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_token_provider(Arc::new(StaticToken("test-azure-ad-token".to_string())));
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some("Bearer test-azure-ad-token")
    );
    // No api-key header when using token auth.
    assert!(headers.get("api-key").is_none());
}

/// The token provider is invoked on every request (tokens differ per call).
#[tokio::test]
async fn should_call_token_provider_per_request() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("hi"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_token_provider(Arc::new(CountingToken {
            count: AtomicU32::new(0),
        }));
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("first call");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("second call");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer token-1")
    );
    assert_eq!(
        requests[1]
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer token-2")
    );
}

/// Provider-level and per-request headers are merged onto the auth headers.
#[tokio::test]
async fn should_merge_provider_and_request_headers() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("hi"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key")
        .with_header("Custom-Provider-Header", "provider-header-value");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let mut options = default_options(test_prompt());
    let mut req_headers = std::collections::HashMap::new();
    req_headers.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );
    options.headers = Some(req_headers);

    let _ = model.do_generate(&options).await.expect("do_generate");

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("api-key").and_then(|v| v.to_str().ok()),
        Some("test-api-key")
    );
    assert_eq!(
        headers
            .get("custom-provider-header")
            .and_then(|v| v.to_str().ok()),
        Some("provider-header-value")
    );
    assert_eq!(
        headers
            .get("custom-request-header")
            .and_then(|v| v.to_str().ok()),
        Some("request-header-value")
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Response parsing (reuses OpenAI conversion logic)
// ════════════════════════════════════════════════════════════════════════════

/// `do_generate` extracts the assistant text content.
#[tokio::test]
async fn should_extract_text_response() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("Hello, World!"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text } => assert_eq!(text, "Hello, World!"),
        other => panic!("expected Text, got {:?}", other),
    }
    // Finish reason + response metadata.
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.response.id, Some("chatcmpl-test".to_string()));
    assert_eq!(result.response.model_id, Some("gpt-4o".to_string()));
}

/// `do_generate` maps Azure/OpenAI usage tokens.
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response(""),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(30));
}

/// The request body carries the deployment as `model` plus the user message.
#[tokio::test]
async fn should_send_deployment_and_messages_in_request_body() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("hi"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "gpt-4o");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "Hello");
}

// ════════════════════════════════════════════════════════════════════════════
// Streaming
// ════════════════════════════════════════════════════════════════════════════

/// `do_stream` emits text deltas followed by a Finish part.
#[tokio::test]
async fn should_stream_text_deltas() {
    let server = MockServer::start().await;
    let sse = sse_body(&[
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\", World!\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":10,\"total_tokens\":14}}\n\n",
    ]);
    mock_sse_response(&server, "/deployments/gpt-4o/chat/completions", &sse).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;
    let deltas = text_deltas(&parts);
    assert_eq!(deltas, vec!["Hello".to_string(), ", World!".to_string()]);

    // The stream must end with a Finish part carrying the stop reason + usage.
    let finish = parts
        .iter()
        .rev()
        .find(|p| matches!(p, StreamPart::Finish { .. }))
        .expect("stream should end with Finish");
    match finish {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.output_tokens.total, Some(10));
        }
        _ => unreachable!(),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Errors & config validation
// ════════════════════════════════════════════════════════════════════════════

/// A 401 response maps to `AiMuxError::Auth`.
#[tokio::test]
async fn should_return_auth_error_on_401() {
    let server = MockServer::start().await;
    mock_json_error(
        &server,
        "/deployments/gpt-4o/chat/completions",
        401,
        json!({ "error": { "message": "invalid api key", "type": "authentication_error" } }),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("bad-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let err = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect_err("401 should error");
    assert!(matches!(err, AiMuxError::Auth(_)));
}

/// Constructing a provider without `resource_name` or `base_url` is rejected.
#[tokio::test]
async fn should_reject_missing_resource_and_base_url() {
    let config = AzureConfig::new().with_api_key("test-api-key");
    assert!(matches!(
        AzureProvider::new(config),
        Err(AiMuxError::InvalidArgument(_))
    ));
}

/// Calling a model with no auth configured returns an `Auth` error at request
/// time.
#[tokio::test]
async fn should_error_when_no_auth_configured() {
    let server = MockServer::start().await;
    // No mock is mounted: the request must never be sent because auth fails
    // before the HTTP call.
    let config = AzureConfig::new().with_base_url(server.uri());
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let err = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect_err("no auth should error");
    assert!(matches!(err, AiMuxError::Auth(_)));

    // No request should have hit the server.
    let requests = server.received_requests().await.unwrap();
    assert!(requests.is_empty());
}

// ════════════════════════════════════════════════════════════════════════════
// Additional cases — tool calls, response format, reasoning, headers, errors.
// ════════════════════════════════════════════════════════════════════════════

/// A chat-completion response carrying a single tool call.
fn tool_call_completion_body() -> Value {
    json!({
        "id": "chatcmpl-tool",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_abc",
                    "type": "function",
                    "function": {
                        "name": "get-weather",
                        "arguments": "{\"city\":\"SF\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 10, "total_tokens": 20, "completion_tokens": 10 }
    })
}

/// TS: tool-call extraction — `do_generate` surfaces a `ToolCall` content.
#[tokio::test]
async fn should_extract_tool_call() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        tool_call_completion_body(),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

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
            assert_eq!(tool_call_id, "call_abc");
            assert_eq!(tool_name, "get-weather");
            assert_eq!(input, &json!({"city": "SF"}));
        }
        other => panic!("expected ToolCall, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// TS: "should stream tool call content" — streaming emits a ToolCall part.
#[tokio::test]
async fn should_stream_tool_call() {
    let server = MockServer::start().await;
    let sse = sse_body(&[
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{\"name\":\"get-weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\":\\\"SF\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":10,\"total_tokens\":20}}\n\n",
    ]);
    mock_sse_response(&server, "/deployments/gpt-4o/chat/completions", &sse).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

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
    assert_eq!(id, "call_abc");
    assert_eq!(name, "get-weather");
    assert_eq!(input, json!({"city": "SF"}));
}

/// TS: "should send a json_schema response format for structured output"
#[tokio::test]
async fn should_send_json_schema_response_format() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("ok"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let schema = json!({
        "type": "object",
        "properties": { "sentiment": { "type": "string" } },
        "required": ["sentiment"],
        "additionalProperties": false
    });
    let mut options = default_options(test_prompt());
    options.response_format = Some(aimux_core::options::ResponseFormat::Json {
        schema: Some(schema.clone()),
        name: None,
        description: None,
    });

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["response_format"]["type"], json!("json_schema"));
    assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
    assert_eq!(
        body["response_format"]["json_schema"]["strict"],
        json!(true)
    );
}

/// TS: "should map top-level reasoning to Azure DeepSeek reasoning effort" —
/// a custom `reasoning` value maps to `reasoning_effort` in the request body.
#[tokio::test]
async fn should_map_reasoning_effort() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        text_completion_response("ok"),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::High);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["reasoning_effort"], json!("high"));
}

/// TS: response headers are exposed on the generate result.
#[tokio::test]
async fn should_expose_response_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/deployments/gpt-4o/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(text_completion_response("hi")),
        )
        .mount(&server)
        .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

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

/// TS: response headers are exposed on the stream result.
#[tokio::test]
async fn should_expose_response_headers_stream() {
    let server = MockServer::start().await;
    let sse = sse_body(&[
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
    ]);
    Mock::given(method("POST"))
        .and(path("/deployments/gpt-4o/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .insert_header("test-header", "test-value")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

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

/// TS: a `length` finish reason maps to `FinishReasonUnified::Length`.
#[tokio::test]
async fn should_map_length_finish_reason() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        "/deployments/gpt-4o/chat/completions",
        json!({
            "id": "chatcmpl-len",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "truncated" },
                "finish_reason": "length"
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        }),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Length);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("length"));
}

/// TS: a 429 response maps to `AiMuxError::RateLimited`.
#[tokio::test]
async fn should_return_rate_limited_on_429() {
    let server = MockServer::start().await;
    mock_json_error(
        &server,
        "/deployments/gpt-4o/chat/completions",
        429,
        json!({ "error": { "message": "Too many requests", "type": "rate_limit" } }),
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.deployment("gpt-4o");

    let err = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect_err("429 should error");
    assert!(matches!(err, AiMuxError::RateLimited { .. }));
}
