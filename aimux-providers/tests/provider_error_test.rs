//! Provider error-handling integration tests (wiremock).
//!
//! Translates the error scenarios from:
//! - `packages/openai/src/chat/openai-chat-language-model.test.ts`
//!   (the three `doStream` error cases: first-chunk error, numeric status
//!   preservation, mid-stream error forwarding).
//! - `packages/anthropic/src/anthropic-language-model.test.ts`
//!   (the 529 overloaded `doGenerate` error + the three `doStream` error
//!   cases: 529 status, first-chunk overloaded, mid-stream overloaded).
//! - `packages/provider-utils/src/response-handler.test.ts`
//!   (status-code → error-variant mapping exercised end-to-end through the
//!   providers' `do_generate` / `do_stream`, which select explicit
//!   failed-response handlers).
//! - `packages/openai/src/openai-error.test.ts` &
//!   `packages/anthropic/src/anthropic-error.test.ts` (provider-specific
//!   failed-response message extraction for both JSON shapes).
//!
//! Each test stands up a `wiremock` server returning an error status code +
//! error JSON body (or, for the stream cases, an SSE body whose first/mid
//! chunk is an error JSON) and asserts the resulting `AiMuxError` variant.
//!
//! Status → error mapping: every status
//! yields `AiMuxError::ApiCall` with the observed code in `status_code`;
//! 408/409/429/5xx are stored retryable (`is_retryable`), other 4xx are not.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::Resolvable;
use aimux_providers::anthropic::{AnthropicProviderSettings, create_anthropic};
use aimux_providers::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A native OpenAI provider against `base_url`. The key is an explicit value,
/// so the environment is never consulted.
fn provider_with(api_key: &str, base_url: impl Into<String>) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::Value(api_key.to_string())),
        base_url: Some(base_url.into()),
        ..Default::default()
    })
    .expect("settings are valid")
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

/// Build `CallOptions` with everything unset except `prompt`.
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn options() -> CallOptions {
    default_options(test_prompt())
}

/// Collect every `StreamPart` from a `StreamResult` into a `Vec`.
async fn collect_stream(stream: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut s = stream.stream;
    while let Some(part) = s.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(e) => parts.push(StreamPart::Error { error: e }),
        }
    }
    parts
}

// ════════════════════════════════════════════════════════════════════════════
// OpenAI — do_generate error mapping
// (response-handler.test.ts status mapping + openai-error.test.ts structure)
// ════════════════════════════════════════════════════════════════════════════

mod openai_generate_errors {
    use super::*;

    fn model(server: &MockServer) -> impl LanguageModel {
        provider_with("test-api-key", server.uri()).chat("gpt-4o")
    }

    /// TS response-handler: 401 → AuthenticationError.
    /// OpenAI body shape: `{"error":{"message":"...","type":"..."}}`.
    #[tokio::test]
    async fn status_401_maps_to_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {
                    "message": "Incorrect API key provided",
                    "type": "invalid_request_error",
                    "param": null,
                    "code": "invalid_api_key"
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.status_code == Some(401) && m.message == "Incorrect API key provided"),
            "expected Auth error, got {result:?}"
        );
    }

    /// TS response-handler: 429 → RateLimitError.
    #[tokio::test]
    async fn status_429_maps_to_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "error": {
                    "message": "Rate limit reached for requests",
                    "type": "requests",
                    "param": null,
                    "code": "rate_limit_exceeded"
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(ref e) if e.status_code() == Some(429)),
            "expected RateLimited error, got {result:?}"
        );
    }

    /// 404 → ModelNotFound (OpenAI returns this for unknown model IDs).
    #[tokio::test]
    async fn status_404_maps_to_model_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": {
                    "message": "The model 'gpt-4o' does not exist",
                    "type": "invalid_request_error",
                    "param": "model",
                    "code": "model_not_found"
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m))
                if m.status_code == Some(404) && m.message == "The model 'gpt-4o' does not exist"),
            "expected a 404 provider error, got {result:?}"
        );
    }

    /// TS response-handler: 500 → APICallError (Provider).
    #[tokio::test]
    async fn status_500_maps_to_provider() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "error": {
                    "message": "The server had an error processing your request.",
                    "type": "server_error",
                    "param": null,
                    "code": null
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.to_string().contains("The server had an error processing your request.")),
            "expected ApiCall error for 5xx, got {result:?}"
        );
    }

    /// TS openai-error.test.ts: OpenRouter nests a stringified JSON error
    /// inside `error.message`. The OpenAI failed-response handler must keep that string
    /// verbatim (it is the message, not a nested structure to drill into).
    #[tokio::test]
    async fn openrouter_resource_exhausted_message_kept_verbatim() {
        let server = MockServer::start().await;
        let nested = "{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n";
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "error": {
                    "message": nested,
                    "code": 429
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        // The provider message is retained inside the ApiCall detail.
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref e)) if e.status_code == Some(429) && e.message == nested),
            "expected ApiCall for OpenRouter 429, got {result:?}"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// OpenAI — do_stream error scenarios
// (openai-chat-language-model.test.ts doStream error cases)
// ════════════════════════════════════════════════════════════════════════════

mod openai_stream_errors {
    use super::*;

    fn model(server: &MockServer) -> impl LanguageModel {
        provider_with("test-api-key", server.uri()).chat("gpt-4o")
    }

    /// A non-success HTTP status on the stream endpoint makes `do_stream`
    /// itself return `Err` via the failed-response handler (the stream is never
    /// started). 500 → Provider.
    #[tokio::test]
    async fn http_500_status_rejects_do_stream() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "error": {
                    "message": "The server had an error processing your request.",
                    "type": "server_error",
                    "param": null,
                    "code": null
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_stream(&options()).await;
        assert!(
            matches!(result, Err(ref e) if e.status_code().is_some_and(|s| s >= 500)),
            "expected ApiCall error for 5xx, got {result:?}"
        );
    }

    /// TS: "should throw an api error when the first stream chunk is an error".
    ///
    /// The stream returns 200 + SSE whose first data chunk is an error JSON.
    /// Mirroring the TS SDK (which rejects the `doStream` promise when the
    /// very first chunk is an error), the OpenAI model peeks at the first SSE
    /// event and makes `do_stream` itself return `Err`.
    #[tokio::test]
    async fn first_stream_chunk_is_error_rejects_do_stream() {
        let server = MockServer::start().await;
        let body = concat!(
            "data: {\"error\":{\"message\":",
            "\"The server had an error processing your request. Sorry about that!\",",
            "\"type\":\"server_error\",\"param\":null,\"code\":null}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let result = model(&server).do_stream(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.to_string().contains("The server had an error processing your request.")),
            "expected Provider error from first-chunk stream error, got {result:?}"
        );
    }

    /// TS: "should forward error stream parts after output has started".
    ///
    /// A normal text delta is emitted first, then an error chunk. The error
    /// must be forwarded as a `StreamPart::Error`.
    #[tokio::test]
    async fn mid_stream_error_is_forwarded() {
        let server = MockServer::start().await;
        let body = concat!(
            "data: {\"id\":\"chatcmpl-err\",\"object\":\"chat.completion.chunk\",",
            "\"created\":1702657020,\"model\":\"gpt-4o\",",
            "\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
            "data: {\"error\":{\"message\":\"stream failed after output\",\"type\":\"server_error\",\"param\":null,\"code\":null}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let stream = model(&server).do_stream(&options()).await.unwrap();
        let parts = collect_stream(stream).await;

        // The text delta must have been emitted before the error.
        let saw_text = parts.iter().any(|p| {
            matches!(p,
            StreamPart::TextDelta { delta, .. } if delta == "Hello")
        });
        assert!(
            saw_text,
            "expected a 'Hello' text delta before the error, got {parts:?}"
        );

        assert!(
            parts.iter().any(|p| matches!(p,
                StreamPart::Error { error: AiMuxError::ApiCall(m) }
                if m.message.contains("stream failed after output"))),
            "expected a StreamPart::Error carrying 'stream failed after output', got {parts:?}"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Anthropic — do_generate error mapping
// (anthropic-language-model.test.ts 529 + response-handler status mapping +
//  anthropic-error.test.ts structure)
// ════════════════════════════════════════════════════════════════════════════

mod anthropic_generate_errors {
    use super::*;

    fn model(server: &MockServer) -> impl LanguageModel {
        create_anthropic(AnthropicProviderSettings {
            api_key: Some("test-api-key".to_string().into()),
            base_url: Some(format!("{}/v1", server.uri())),
            ..Default::default()
        })
        .unwrap()
        .messages("claude-3-haiku-20240307")
    }

    /// Anthropic 401 → `ApiCall` (401). Anthropic body shape:
    /// `{"type":"error","error":{"type":"authentication_error","message":"..."}}`.
    #[tokio::test]
    async fn status_401_maps_to_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "authentication_error",
                    "message": "invalid x-api-key"
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.status_code == Some(401) && m.message == "invalid x-api-key"),
            "expected Auth error, got {result:?}"
        );
    }

    /// Anthropic 429 → RateLimited.
    #[tokio::test]
    async fn status_429_maps_to_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "rate_limit_error",
                    "message": "number of request tokens has exceeded your rate limit"
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(ref e) if e.status_code() == Some(429)),
            "expected RateLimited error, got {result:?}"
        );
    }

    /// Anthropic 404 → ModelNotFound.
    #[tokio::test]
    async fn status_404_maps_to_model_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "not_found_error",
                    "message": "model: claude-3-haiku-20240307"
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m))
                if m.status_code == Some(404) && m.message == "model: claude-3-haiku-20240307"),
            "expected a 404 provider error, got {result:?}"
        );
    }

    /// Anthropic 500 → Provider.
    #[tokio::test]
    async fn status_500_maps_to_provider() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "api_error",
                    "message": "Internal server error"
                }
            })))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.to_string().contains("Internal server error")),
            "expected ApiCall error for 5xx, got {result:?}"
        );
    }

    /// TS: "should throw an api error when the server is returning a 529
    /// overloaded error" (doGenerate). Anthropic's 529 maps to the generic
    /// `Provider` variant (the Rust error model has no dedicated "overloaded"
    /// variant); the `overloaded_error` message is preserved.
    #[tokio::test]
    async fn status_529_overloaded_maps_to_provider() {
        let server = MockServer::start().await;
        let body = r#"{"type":"error","error":{"details":null,"type":"overloaded_error","message":"Overloaded"}}"#;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(529).set_body_string(body))
            .mount(&server)
            .await;

        let result = model(&server).do_generate(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.to_string().contains("Overloaded")),
            "expected ApiCall error carrying 'Overloaded', got {result:?}"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Anthropic — do_stream error scenarios
// (anthropic-language-model.test.ts doStream error cases)
// ════════════════════════════════════════════════════════════════════════════

mod anthropic_stream_errors {
    use super::*;

    fn model(server: &MockServer) -> impl LanguageModel {
        create_anthropic(AnthropicProviderSettings {
            api_key: Some("test-api-key".to_string().into()),
            base_url: Some(format!("{}/v1", server.uri())),
            ..Default::default()
        })
        .unwrap()
        .messages("claude-3-haiku-20240307")
    }

    /// TS: "should throw an api error when the server is returning a 529
    /// overloaded error" (doStream). A non-success HTTP status makes
    /// `do_stream` return `Err` via the failed-response handler.
    #[tokio::test]
    async fn http_529_status_rejects_do_stream() {
        let server = MockServer::start().await;
        let body = r#"{"type":"error","error":{"details":null,"type":"overloaded_error","message":"Overloaded"}}"#;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(529).set_body_string(body))
            .mount(&server)
            .await;

        let result = model(&server).do_stream(&options()).await;
        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.to_string().contains("Overloaded")),
            "expected ApiCall error carrying 'Overloaded', got {result:?}"
        );
    }

    /// TS: "should throw an api error when the first stream chunk is an
    /// overloaded error" (doStream).
    ///
    /// The stream returns 200 + SSE whose first event is an `error` event.
    /// The provider peeks that event before returning the stream so the Core
    /// operation retry can treat it as an attempt failure.
    #[tokio::test]
    async fn first_stream_chunk_is_overloaded_error() {
        let server = MockServer::start().await;
        let body = concat!(
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"details\":null,\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n",
            "\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let error = model(&server)
            .do_stream(&options())
            .await
            .expect_err("first error event must reject do_stream");
        assert!(
            matches!(error, AiMuxError::ApiCall(ref detail)
                if detail.status_code == Some(529)
                    && detail.message == "Overloaded"
                    && detail.is_retryable),
            "expected retryable 529 ApiCall error, got {error:?}"
        );
    }
}
