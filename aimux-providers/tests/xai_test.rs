//! Rust translations of the AI SDK xAI provider tests that are not specific
//! to one model file.
//!
//! Sources (TS → Rust):
//! - `packages/xai/src/supports-reasoning-effort.test.ts` → `supports_reasoning_effort` mod
//! - `packages/xai/src/xai-error.test.ts` → `error_handling` mod
//! - `packages/xai/src/xai-provider.test.ts` → `provider` mod
//!
//! The package serves the Responses API only (see `xai_responses_test.rs` for
//! the model tests), so the error and provider cases run against
//! `provider.responses(id)`. Each test uses `wiremock` to spin up a mock HTTP
//! server and asserts on the request or the result.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;

use aimux_providers::xai::convert::supports_reasoning_effort;
use aimux_providers::{XAIProvider, XAIProviderSettings, create_xai};

fn test_provider(api_key: &str, base_url: impl Into<String>) -> XAIProvider {
    create_xai(XAIProviderSettings {
        api_key: Some(api_key.to_string().into()),
        base_url: Some(base_url.into()),
        ..Default::default()
    })
    .expect("valid settings")
}

// ── shared helpers ───────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> Vec<LanguageModelPromptMessage> {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// `CallOptions` with only `prompt` set.
fn default_options(prompt: Vec<LanguageModelPromptMessage>) -> CallOptions {
    CallOptions::new(prompt)
}

/// A standard non-streaming Responses API body returning "hello world".
fn text_response_body() -> Value {
    json!({
        "id": "resp_123",
        "object": "response",
        "created_at": 1700000000,
        "status": "completed",
        "model": "grok-4-fast-non-reasoning",
        "output": [{
            "type": "message",
            "id": "msg_123",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": "hello world",
                "annotations": []
            }]
        }],
        "usage": { "input_tokens": 10, "output_tokens": 5, "total_tokens": 15 }
    })
}

fn make_provider(server: &MockServer) -> XAIProvider {
    test_provider("test-api-key", server.uri())
}

// ════════════════════════════════════════════════════════════════════════════
// supportsReasoningEffort — direct function tests
// (supports-reasoning-effort.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod supports_reasoning_effort_tests {
    use super::*;

    /// TS: should return true for grok-4.3
    #[test]
    fn true_for_grok_4_3() {
        assert!(supports_reasoning_effort("grok-4.3"));
    }

    /// TS: should return true for grok-latest
    #[test]
    fn true_for_grok_latest() {
        assert!(supports_reasoning_effort("grok-latest"));
    }

    /// TS: should return true for grok-4.20-multi-agent
    #[test]
    fn true_for_grok_4_20_multi_agent() {
        assert!(supports_reasoning_effort("grok-4.20-multi-agent"));
    }

    /// TS: should return true for grok-4.20-multi-agent-0309
    #[test]
    fn true_for_grok_4_20_multi_agent_0309() {
        assert!(supports_reasoning_effort("grok-4.20-multi-agent-0309"));
    }

    /// TS: should return true for grok-3-mini
    #[test]
    fn true_for_grok_3_mini() {
        assert!(supports_reasoning_effort("grok-3-mini"));
    }

    /// TS: should return false for grok-4.20-reasoning
    #[test]
    fn false_for_grok_4_20_reasoning() {
        assert!(!supports_reasoning_effort("grok-4.20-reasoning"));
    }

    /// TS: should return false for grok-4.20-non-reasoning
    #[test]
    fn false_for_grok_4_20_non_reasoning() {
        assert!(!supports_reasoning_effort("grok-4.20-non-reasoning"));
    }

    /// TS: should return false for grok-4.20-0309-reasoning
    #[test]
    fn false_for_grok_4_20_0309_reasoning() {
        assert!(!supports_reasoning_effort("grok-4.20-0309-reasoning"));
    }

    /// TS: should return false for grok-4.20-0309-non-reasoning
    #[test]
    fn false_for_grok_4_20_0309_non_reasoning() {
        assert!(!supports_reasoning_effort("grok-4.20-0309-non-reasoning"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convertXaiChatUsage — tested through do_generate
// (convert-xai-chat-usage.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod error_handling {
    use super::*;

    /// TS: extracts message from chat completions error shape (400)
    #[tokio::test]
    async fn chat_completions_error_400() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": {
                    "message": "Invalid value: temperature must be between 0 and 2",
                    "type": "invalid_request_error",
                    "code": "invalid_value"
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-3");
        let result = model.do_generate(&default_options(test_prompt())).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Invalid value: temperature must be between 0 and 2"),
            "got: {msg}"
        );
    }

    /// TS: extracts message and code from responses api error shape
    #[tokio::test]
    async fn responses_error_shape() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "code": "Client specified an invalid argument",
                "error": "Invalid request content: Each message must have at least one content element."
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-3");
        let result = model.do_generate(&default_options(test_prompt())).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Client specified an invalid argument"),
            "got: {msg}"
        );
        assert!(msg.contains("Invalid request content"), "got: {msg}");
    }

    /// TS: should throw APICallError when xai returns error with 200 status (doGenerate)
    #[tokio::test]
    async fn error_with_200_status_do_generate() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": "The service is currently unavailable",
                "error": "Timed out waiting for first token"
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-3");
        let result = model.do_generate(&default_options(test_prompt())).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Timed out waiting for first token"),
            "got: {msg}"
        );
    }

    /// TS: should throw APICallError when xai returns error with 200 status (doStream)
    #[tokio::test]
    async fn error_with_200_status_do_stream() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": "The service is currently unavailable",
                "error": "Timed out waiting for first token"
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-3");
        let result = model.do_stream(&default_options(test_prompt())).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Timed out waiting for first token"),
            "got: {msg}"
        );
    }

    /// TS: 401 maps to `ApiCall` (401)
    #[tokio::test]
    async fn status_401_maps_to_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {
                    "message": "Incorrect API key provided",
                    "type": "invalid_request_error"
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-3");
        let result = model.do_generate(&default_options(test_prompt())).await;

        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.status_code == Some(401) && m.message == "Incorrect API key provided")
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Provider configuration — base URL, headers, auth
// (xai-provider.test.ts → chat-related tests)
// ════════════════════════════════════════════════════════════════════════════

mod provider {
    use super::*;

    /// TS: should construct a chat model with correct configuration
    #[tokio::test]
    async fn chat_model_config() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-3");

        assert_eq!(model.model_id(), "grok-3");
        assert_eq!(model.provider(), "xai.responses");
    }

    /// TS: should use custom baseURL
    #[tokio::test]
    async fn custom_base_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body()))
            .mount(&server)
            .await;

        let provider = test_provider("test-api-key", server.uri());
        let model = provider.responses("grok-3");
        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/responses");
    }

    /// TS: should pass headers (provider + request)
    #[tokio::test]
    async fn pass_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-3");
        let mut headers = HashMap::new();
        headers.insert(
            "Custom-Request-Header".to_string(),
            "request-header-value".to_string(),
        );
        let options = CallOptions {
            headers: Some(headers),
            ..default_options(test_prompt())
        };
        let _ = model.do_generate(&options).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer test-api-key")
        );
        assert_eq!(
            requests[0]
                .headers
                .get("custom-request-header")
                .and_then(|v| v.to_str().ok()),
            Some("request-header-value")
        );
    }

    /// TS: request uses correct URL and auth header
    #[tokio::test]
    async fn request_url_and_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-2");
        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/responses");
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer test-api-key")
        );
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "grok-2");
    }

    /// TS: should warn for unsupported parameters (frequencyPenalty,
    /// presencePenalty, stopSequences)
    #[tokio::test]
    async fn warn_for_unsupported_params() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses("grok-3");
        let options = CallOptions {
            frequency_penalty: Some(0.5),
            presence_penalty: Some(0.3),
            stop_sequences: Some(vec!["stop".to_string()]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let features: Vec<_> = result
            .warnings
            .iter()
            .filter_map(|w| match w {
                aimux_core::types::Warning::Unsupported { feature, .. } => Some(feature.clone()),
                _ => None,
            })
            .collect();
        assert!(features.contains(&"frequencyPenalty".to_string()));
        assert!(features.contains(&"presencePenalty".to_string()));
        assert!(features.contains(&"stopSequences".to_string()));
    }
}
