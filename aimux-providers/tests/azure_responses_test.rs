//! Wiremock tests for the Azure OpenAI Responses API provider.
//!
//! These cover the Azure-specific differences from the OpenAI Responses
//! provider:
//! - deployment-based URL construction with an `api-version` query parameter
//! - v1 URL form on a non-Azure gateway (no `api-version`)
//! - `api-key` header authentication (API key)
//! - `Authorization: Bearer <token>` authentication (Azure AD token provider)
//! - token provider invoked per request (tokens differ per call)
//! - custom provider/request headers + user-agent suffix
//! - Azure `assistant-` file ID prefix passthrough (file_id vs base64)
//! - doGenerate text extraction, usage, response metadata, provider metadata
//! - doStream text content streaming
//! - error status codes
//! - env-var config (serial)
//!
//! Each test spins up a `wiremock` mock server, points an `AzureProvider` at
//! it via `base_url`, and asserts on the issued request and/or the parsed
//! result.
//!
//! Translated from the TS test suite
//! `reference/ai/packages/azure/src/azure-openai-provider.test.ts`
//! (responses-specific cases). Not all 44 TS cases are translated — the focus
//! is on Azure-specific capabilities.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
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
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;

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

/// The default deployment-based path used by most tests.
const DEPLOYMENT_PATH: &str = "/deployments/test-deployment/responses";

/// Standard mock for a JSON responses-api response on the deployment path.
async fn mock_json_response(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path(DEPLOYMENT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mock for a JSON responses-api response with custom headers.
async fn mock_json_response_with_headers(
    server: &MockServer,
    body: Value,
    headers: Vec<(&str, &str)>,
) {
    let mut template = ResponseTemplate::new(200).set_body_json(body);
    for (k, v) in headers {
        template = template.insert_header(k, v);
    }
    Mock::given(method("POST"))
        .and(path(DEPLOYMENT_PATH))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Standard mock for an SSE streaming response on the deployment path.
async fn mock_sse_response(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path(DEPLOYMENT_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

/// A simple text responses-api response body (single message output item).
fn text_response_body() -> Value {
    json!({
        "id": "resp_67c97c0203188190a025beb4a75242bc",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "error": null,
        "incomplete_details": null,
        "model": "gpt-4o",
        "output": [
            {
                "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": "answer text",
                        "annotations": []
                    }
                ]
            }
        ],
        "usage": {
            "input_tokens": 345,
            "input_tokens_details": {
                "cached_tokens": 234
            },
            "output_tokens": 538,
            "output_tokens_details": {
                "reasoning_tokens": 123
            }
        },
        "reasoning": { "effort": null, "summary": null, "context": "current_turn" }
    })
}

/// A responses-api response body with a function_call output item.
fn tool_call_response_body() -> Value {
    json!({
        "id": "resp_tool_call_123",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "error": null,
        "incomplete_details": null,
        "model": "gpt-4o",
        "output": [
            {
                "id": "fc_001",
                "type": "function_call",
                "status": "completed",
                "call_id": "call_abc123",
                "name": "getWeather",
                "arguments": "{\"location\": \"San Francisco\"}"
            }
        ],
        "usage": {
            "input_tokens": 50,
            "output_tokens": 20,
            "total_tokens": 70
        }
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

/// Build an SSE event string from a JSON string.
fn sse_event(json_str: &str) -> String {
    format!("data: {}\n\n", json_str)
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

/// Extract the first recorded request body as a JSON value.
async fn first_request_body(server: &MockServer) -> Value {
    let requests = server
        .received_requests()
        .await
        .expect("no requests received");
    serde_json::from_slice(&requests[0].body).expect("invalid JSON body")
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
// URL construction & api-version
// ════════════════════════════════════════════════════════════════════════════

/// The default deployment-based URL form places the deployment in the path and
/// appends `?api-version=2024-10-21` (the Rust AzureConfig default).
#[tokio::test]
async fn should_build_deployment_url_with_default_api_version() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), DEPLOYMENT_PATH);
    assert_eq!(requests[0].url.query(), Some("api-version=2024-10-21"));
}

/// A custom `api_version` is reflected in the `api-version` query parameter.
#[tokio::test]
async fn should_use_custom_api_version() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key")
        .with_api_version("2025-04-01-preview");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");
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

/// The v1 form on a non-Azure gateway baseURL omits `api-version`
/// (the gateway owns its own versioning).
#[tokio::test]
async fn should_omit_api_version_on_gateway_v1_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body()))
        .mount(&server)
        .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key")
        .use_v1_urls();
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].url.path(), "/v1/responses");
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
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");
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
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_token_provider(Arc::new(StaticToken("test-azure-ad-token".to_string())));
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");
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
    // Mount two responses (one per request).
    for _ in 0..2 {
        mock_json_response(&server, text_response_body()).await;
    }

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_token_provider(Arc::new(CountingToken {
            count: AtomicU32::new(0),
        }));
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("first do_generate");
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("second do_generate");

    let requests = server.received_requests().await.unwrap();
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

// ════════════════════════════════════════════════════════════════════════════
// Headers
// ════════════════════════════════════════════════════════════════════════════

/// Provider-level and per-request headers are merged; user-agent suffix is set.
#[tokio::test]
async fn should_pass_custom_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(DEPLOYMENT_PATH))
        .and(header("api-key", "test-api-key"))
        .and(header("custom-provider-header", "provider-header-value"))
        .and(header("custom-request-header", "request-header-value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body()))
        .mount(&server)
        .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key")
        .with_header("Custom-Provider-Header", "provider-header-value");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let options = CallOptions {
        headers: Some({
            let mut m = std::collections::HashMap::new();
            m.insert(
                "Custom-Request-Header".to_string(),
                "request-header-value".to_string(),
            );
            m
        }),
        ..default_options(test_prompt())
    };

    let _ = model
        .do_generate(&options)
        .await
        .expect("do_generate should succeed");

    // If the mock matched, the headers were all present.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let ua = requests[0]
        .headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ua.contains("ai-sdk/azure"),
        "user-agent should contain ai-sdk/azure, got: {}",
        ua
    );
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — response parsing
// ════════════════════════════════════════════════════════════════════════════

/// Text content is extracted from the `output` array.
#[tokio::test]
async fn should_extract_text_content() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let text: Vec<&str> = result
        .content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, vec!["answer text"]);
}

/// Function-call output items are extracted as ToolCall content.
#[tokio::test]
async fn should_extract_tool_call_content() {
    let server = MockServer::start().await;
    mock_json_response(&server, tool_call_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let tool_calls: Vec<_> = result
        .content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
            } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].0, "call_abc123");
    assert_eq!(tool_calls[0].1, "getWeather");
    assert_eq!(tool_calls[0].2["location"], "San Francisco");
}

/// Usage is extracted from the `usage` field.
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(345));
    assert_eq!(result.usage.output_tokens.total, Some(538));
}

/// Response metadata (id, model, timestamp) is extracted.
#[tokio::test]
async fn should_extract_response_metadata() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(
        result.response.id.as_deref(),
        Some("resp_67c97c0203188190a025beb4a75242bc")
    );
    assert_eq!(result.response.model_id.as_deref(), Some("gpt-4o"));
    assert!(result.response.timestamp.is_some());
}

/// Provider metadata uses the `azure` namespace key.
#[tokio::test]
async fn should_use_azure_provider_metadata_namespace() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let pm = result.provider_metadata.expect("provider_metadata");
    assert!(pm.get("azure").is_some(), "should have 'azure' key");
    assert_eq!(
        pm["azure"]["responseId"],
        "resp_67c97c0203188190a025beb4a75242bc"
    );
    // The provider() method returns "azure.responses".
    assert_eq!(model.provider(), "azure.responses");
}

/// Finish reason is `stop` for a completed text response.
#[tokio::test]
async fn should_map_finish_reason_stop() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
}

/// Finish reason is `tool-calls` when the output contains a function_call.
#[tokio::test]
async fn should_map_finish_reason_tool_calls() {
    let server = MockServer::start().await;
    mock_json_response(&server, tool_call_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

/// Response headers are captured.
#[tokio::test]
async fn should_extract_response_headers() {
    let server = MockServer::start().await;
    mock_json_response_with_headers(
        &server,
        text_response_body(),
        vec![("test-header", "test-value")],
    )
    .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let headers = result.response_headers.expect("response_headers");
    assert_eq!(
        headers.get("test-header").map(|s| s.as_str()),
        Some("test-value")
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Azure file ID prefix (assistant-)
// ════════════════════════════════════════════════════════════════════════════

/// When a FileBase64 part's data starts with `assistant-`, it is passed
/// through as a `file_id` instead of being base64-encoded.
#[tokio::test]
async fn should_pass_through_assistant_file_id_for_image() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("Analyze this image"),
            ContentPart::FileBase64 {
                data: "assistant-abc123".to_string(),
                media_type: "image/jpeg".to_string(),
                filename: None,
                provider_options: None,
            },
        ],
        ..Default::default()
    }];

    let _ = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");

    let body = first_request_body(&server).await;
    assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
    assert_eq!(
        body["input"][0]["content"][1]["file_id"],
        "assistant-abc123"
    );
    // image_url should NOT be present.
    assert!(body["input"][0]["content"][1].get("image_url").is_none());
}

/// When a FileBase64 part's data starts with `assistant-` for a PDF,
/// it is passed through as a `file_id` with `input_file` type.
#[tokio::test]
async fn should_pass_through_assistant_file_id_for_pdf() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("Analyze this PDF"),
            ContentPart::FileBase64 {
                data: "assistant-pdf123".to_string(),
                media_type: "application/pdf".to_string(),
                filename: None,
                provider_options: None,
            },
        ],
        ..Default::default()
    }];

    let _ = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");

    let body = first_request_body(&server).await;
    assert_eq!(body["input"][0]["content"][1]["type"], "input_file");
    assert_eq!(
        body["input"][0]["content"][1]["file_id"],
        "assistant-pdf123"
    );
    // file_data and filename should NOT be present.
    assert!(body["input"][0]["content"][1].get("file_data").is_none());
    assert!(body["input"][0]["content"][1].get("filename").is_none());
}

/// Non-`assistant-` file IDs fall back to base64 encoding (the default
/// OpenAI behavior). The Rust convert produces `input_file` for `FileBase64`
/// regardless of media type — the data is not treated as a file ID.
#[tokio::test]
async fn should_fall_back_to_base64_for_non_assistant_file_ids() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("Analyze this image"),
            ContentPart::FileBase64 {
                data: "file-abc123".to_string(),
                media_type: "image/jpeg".to_string(),
                filename: None,
                provider_options: None,
            },
        ],
        ..Default::default()
    }];

    let _ = model
        .do_generate(&default_options(prompt))
        .await
        .expect("should succeed");

    let body = first_request_body(&server).await;
    // Rust convert produces input_file for FileBase64 (not input_image).
    assert_eq!(body["input"][0]["content"][1]["type"], "input_file");
    assert_eq!(
        body["input"][0]["content"][1]["file_data"],
        "data:image/jpeg;base64,file-abc123"
    );
    // file_id should NOT be present (data doesn't start with assistant-).
    assert!(body["input"][0]["content"][1].get("file_id").is_none());
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — text content
// ════════════════════════════════════════════════════════════════════════════

/// Streaming text content: response.created → output_item.added →
/// output_text.delta → output_item.done → response.completed.
#[tokio::test]
async fn should_stream_text_content() {
    let server = MockServer::start().await;
    let sse = sse_body(&[
        &sse_event(
            r#"{"type":"response.created","response":{"id":"resp_123","object":"response","created_at":1741257730,"status":"in_progress","model":"gpt-4o","output":[]}}"#,
        ),
        &sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
        ),
        &sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello"}"#,
        ),
        &sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":" world"}"#,
        ),
        &sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Hello world","annotations":[]}]}}"#,
        ),
        &sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_123","object":"response","created_at":1741257730,"status":"completed","model":"gpt-4o","output":[{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Hello world","annotations":[]}]}],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#,
        ),
    ]);
    mock_sse_response(&server, &sse).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");

    let parts = collect_stream(result).await;

    // StreamStart
    assert!(matches!(&parts[0], StreamPart::StreamStart { .. }));

    // ResponseMetadata
    assert!(matches!(
        &parts[1],
        StreamPart::ResponseMetadata { id, .. } if id.as_deref() == Some("resp_123")
    ));

    // TextStart
    assert!(matches!(
        &parts[2],
        StreamPart::TextStart { id } if id == "msg_1"
    ));

    // TextDelta "Hello" + " world"
    let deltas = text_deltas(&parts);
    assert_eq!(deltas, vec!["Hello", " world"]);

    // TextEnd
    assert!(matches!(
        parts.iter().find(|p| matches!(p, StreamPart::TextEnd { .. })),
        Some(StreamPart::TextEnd { id }) if id == "msg_1"
    ));

    // Finish
    let finish = parts
        .iter()
        .find(|p| matches!(p, StreamPart::Finish { .. }));
    assert!(finish.is_some());
    if let Some(StreamPart::Finish {
        finish_reason,
        usage,
        ..
    }) = finish
    {
        assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(usage.input_tokens.total, Some(10));
        assert_eq!(usage.output_tokens.total, Some(5));
    } else {
        panic!("expected Finish part");
    }
}

/// Streaming: api-key header is sent and api-version query param is present.
#[tokio::test]
async fn should_send_api_key_header_on_stream() {
    let server = MockServer::start().await;
    let sse = sse_body(&[
        &sse_event(
            r#"{"type":"response.created","response":{"id":"resp_1","object":"response","created_at":1,"status":"in_progress","model":"gpt-4o","output":[]}}"#,
        ),
        &sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_1","object":"response","created_at":1,"status":"completed","model":"gpt-4o","output":[],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}"#,
        ),
    ]);
    mock_sse_response(&server, &sse).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let _ = collect_stream(result).await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.query(), Some("api-version=2024-10-21"));
    assert_eq!(
        requests[0]
            .headers
            .get("api-key")
            .and_then(|v| v.to_str().ok()),
        Some("test-api-key")
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Error handling
// ════════════════════════════════════════════════════════════════════════════

/// A non-2xx response is mapped to a provider error.
#[tokio::test]
async fn should_handle_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(DEPLOYMENT_PATH))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {
                "message": "Rate limit exceeded",
                "type": "rate_limit_exceeded",
                "code": "rate_limit_exceeded"
            }
        })))
        .mount(&server)
        .await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let result = model.do_generate(&default_options(test_prompt())).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("rate limit"),
        "error should mention rate limit, got: {}",
        err
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Environment variable configuration (serial)
// ════════════════════════════════════════════════════════════════════════════

/// `AzureConfig::from_env` reads `AZURE_API_KEY` and `AZURE_RESOURCE_NAME`.
#[tokio::test]
#[serial_test::serial]
async fn should_create_config_from_env() {
    // Set env vars.
    unsafe {
        std::env::set_var("AZURE_API_KEY", "env-api-key");
        std::env::set_var("AZURE_RESOURCE_NAME", "env-resource");
    }

    let config = AzureConfig::from_env().expect("from_env");
    assert_eq!(config.resource_name.as_deref(), Some("env-resource"));

    // Clean up.
    unsafe {
        std::env::remove_var("AZURE_API_KEY");
        std::env::remove_var("AZURE_RESOURCE_NAME");
    }

    // Verify the config produces the right URL.
    let model = AzureProvider::new(config)
        .expect("provider")
        .responses_model("my-deployment");
    assert_eq!(model.model_id(), "my-deployment");
    assert_eq!(model.provider(), "azure.responses");
}

/// `AzureConfig::from_env` fails when `AZURE_API_KEY` is not set.
#[tokio::test]
#[serial_test::serial]
async fn should_fail_from_env_without_api_key() {
    // Ensure the env var is not set.
    unsafe {
        std::env::remove_var("AZURE_API_KEY");
    }

    let result = AzureConfig::from_env();
    assert!(result.is_err());
}

// ════════════════════════════════════════════════════════════════════════════
// Request body construction
// ════════════════════════════════════════════════════════════════════════════

/// The request body includes the model (deployment) and input array.
#[tokio::test]
async fn should_send_model_and_input_in_request_body() {
    let server = MockServer::start().await;
    mock_json_response(&server, text_response_body()).await;

    let config = AzureConfig::new()
        .with_base_url(server.uri())
        .with_api_key("test-api-key");
    let provider = AzureProvider::new(config).expect("provider");
    let model = provider.responses_model("test-deployment");

    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let body = first_request_body(&server).await;
    assert_eq!(body["model"], "test-deployment");
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(body["input"][0]["content"][0]["text"], "Hello");
}
