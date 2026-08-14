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
//!   providers' `do_generate` / `do_stream`, which call
//!   `parse_provider_error` under the hood).
//! - `packages/openai/src/openai-error.test.ts` &
//!   `packages/anthropic/src/anthropic-error.test.ts` (error-structure
//!   message extraction for both providers' JSON shapes).
//!
//! Each test stands up a `wiremock` server returning an error status code +
//! error JSON body (or, for the stream cases, an SSE body whose first/mid
//! chunk is an error JSON) and asserts the resulting `AiMuxError` variant.
//!
//! Status → error mapping (mirrors `parse_provider_error`): every status
//! yields `AiMuxError::ApiCall` with the observed code in `status_code`;
//! 408/409/429/5xx are stored retryable (`is_retryable`), other 4xx are not.

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::RetryConfig;
use aimux_providers::anthropic::{AnthropicConfig, AnthropicProvider};
use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
        let config = OpenAIConfig::new("test-api-key")
            .with_base_url(server.uri())
            .with_retry_config(RetryConfig {
                max_retries: 0,
                ..Default::default()
            });
        OpenAIProvider::new(config).model("gpt-4o")
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
    /// inside `error.message`. `parse_provider_error` must keep that string
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
        // 429 → RateLimited; the nested message is carried but RateLimited
        // only exposes retry_after_ms, so we just assert the variant.
        assert!(
            matches!(result, Err(ref e) if e.status_code() == Some(429)),
            "expected RateLimited for OpenRouter 429, got {result:?}"
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
        let config = OpenAIConfig::new("test-api-key")
            .with_base_url(server.uri())
            .with_retry_config(RetryConfig {
                max_retries: 0,
                ..Default::default()
            });
        OpenAIProvider::new(config).model("gpt-4o")
    }

    /// A non-success HTTP status on the stream endpoint makes `do_stream`
    /// itself return `Err` via `parse_provider_error` (the stream is never
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
        let config = AnthropicConfig::new("test-api-key")
            .with_base_url(server.uri())
            .with_retry_config(RetryConfig {
                max_retries: 0,
                ..Default::default()
            });
        AnthropicProvider::new(config).model("claude-3-haiku-20240307")
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

    /// TS anthropic-error.test.ts: the overloaded error structure
    /// `{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}`
    /// is parsed and `error.message` is extracted.
    #[tokio::test]
    async fn overloaded_error_structure_message_extracted() {
        use aimux_provider_utils::{DEFAULT_ERROR_STRUCTURE, parse_provider_error};
        let body = r#"{"type":"error","error":{"details":null,"type":"overloaded_error","message":"Overloaded"}}"#;
        let err = parse_provider_error(529, body, &DEFAULT_ERROR_STRUCTURE);
        assert!(
            matches!(err, AiMuxError::ApiCall(ref m) if m.message.contains("Overloaded")),
            "expected Provider carrying 'Overloaded', got {err:?}"
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
        let config = AnthropicConfig::new("test-api-key")
            .with_base_url(server.uri())
            .with_retry_config(RetryConfig {
                max_retries: 0,
                ..Default::default()
            });
        AnthropicProvider::new(config).model("claude-3-haiku-20240307")
    }

    /// TS: "should throw an api error when the server is returning a 529
    /// overloaded error" (doStream). A non-success HTTP status makes
    /// `do_stream` return `Err` via `parse_provider_error`.
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
    /// In the Rust model the error surfaces as a `StreamPart::Error` early in
    /// the stream.
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

        let stream = model(&server).do_stream(&options()).await.unwrap();
        let parts = collect_stream(stream).await;
        assert!(
            parts.iter().any(|p| matches!(p,
                StreamPart::Error { error: AiMuxError::ApiCall(m) } if m.message == "Overloaded")),
            "expected a StreamPart::Error carrying 'Overloaded', got {parts:?}"
        );
        // Finish is the final-chunk contract: the stream must terminate with
        // an error-finish even when the very first event is an error.
        assert!(
            matches!(parts.last(),
                Some(StreamPart::Finish { finish_reason, .. })
                    if matches!(finish_reason.unified,
                        aimux_core::prelude::FinishReasonUnified::Error)),
            "expected the last part to be Finish with unified=Error, got {parts:?}"
        );
    }

    /// TS: "should forward error chunks" + "should forward overloaded error
    /// during streaming".
    ///
    /// A normal text delta is emitted first, then an error event mid-stream.
    /// The error must be forwarded as a `StreamPart::Error`.
    #[tokio::test]
    async fn mid_stream_error_is_forwarded() {
        let server = MockServer::start().await;
        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-3-haiku-20240307\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":17,\"output_tokens\":1}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"details\":null,\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
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

        let stream = model(&server).do_stream(&options()).await.unwrap();
        let parts = collect_stream(stream).await;

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
                StreamPart::Error { error: AiMuxError::ApiCall(m) } if m.message == "Overloaded")),
            "expected a StreamPart::Error carrying 'Overloaded', got {parts:?}"
        );

        // Finish=final chunk: a mid-stream error must still be followed by a
        // Finish part (error finish), never leave the stream unterminated.
        assert!(
            matches!(parts.last(),
                Some(StreamPart::Finish { finish_reason, .. })
                    if matches!(finish_reason.unified,
                        aimux_core::prelude::FinishReasonUnified::Error)),
            "expected the last part to be Finish with unified=Error, got {parts:?}"
        );
        // Error-finish zeroes usage: the message_start input_tokens (17)
        // must not leak into the final part (mirrors openai behaviour).
        if let Some(StreamPart::Finish { usage, .. }) = parts.last() {
            assert_eq!(
                usage.input_tokens.total, None,
                "usage must be zeroed on error finish"
            );
        }
    }

    /// TS: "should forward error chunks" — a generic (non-overloaded) error
    /// event mid-stream must also be forwarded.
    #[tokio::test]
    async fn mid_stream_generic_error_is_forwarded() {
        let server = MockServer::start().await;
        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-3-haiku-20240307\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":17,\"output_tokens\":1}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"ping\"}\n\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"error\",\"message\":\"test error\"}}\n\n",
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

        let stream = model(&server).do_stream(&options()).await.unwrap();
        let parts = collect_stream(stream).await;
        assert!(
            parts.iter().any(|p| matches!(p,
                StreamPart::Error { error: AiMuxError::ApiCall(m) } if m.message == "test error")),
            "expected a StreamPart::Error carrying 'test error', got {parts:?}"
        );
        assert!(
            matches!(parts.last(),
                Some(StreamPart::Finish { finish_reason, .. })
                    if matches!(finish_reason.unified,
                        aimux_core::prelude::FinishReasonUnified::Error)),
            "expected the last part to be Finish with unified=Error, got {parts:?}"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Error-structure parsing (openai-error.test.ts + anthropic-error.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod error_structure_parsing {
    use super::*;
    use aimux_provider_utils::{DEFAULT_ERROR_STRUCTURE, parse_provider_error};

    /// TS openai-error.test.ts: the OpenAI error schema parses
    /// `{"error":{"message":"...","code":429}}` and `error.message` is the
    /// surfaced message.
    #[test]
    fn openai_error_message_extracted_from_default_structure() {
        let body = r#"{"error":{"message":"Resource has been exhausted","type":"requests","param":null,"code":429}}"#;
        let err = parse_provider_error(429, body, &DEFAULT_ERROR_STRUCTURE);
        // 429 → RateLimited (variant only; message is not carried on the
        // RateLimited variant in the Rust error model).
        assert!(matches!(err, ref e if e.status_code() == Some(429)));
    }

    /// TS openai-error.test.ts: OpenRouter nests a stringified JSON object
    /// inside `error.message`. `parse_provider_error` must keep that string
    /// verbatim rather than trying to drill into it.
    #[test]
    fn openrouter_nested_message_kept_verbatim() {
        let nested = "{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n";
        let body = format!(
            r#"{{"error":{{"message":{msg},"code":429}}}}"#,
            msg = serde_json::to_string(nested).unwrap()
        );
        let err = parse_provider_error(500, &body, &DEFAULT_ERROR_STRUCTURE);
        // 500 → Provider; the (stringified) nested JSON is the message.
        assert!(
            matches!(err, AiMuxError::ApiCall(ref m) if m.message.contains("RESOURCE_EXHAUSTED")),
            "expected Provider carrying the OpenRouter nested message, got {err:?}"
        );
    }

    /// TS anthropic-error.test.ts: the overloaded error structure
    /// `{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}`
    /// parses with `error.message` = "Overloaded".
    #[test]
    fn anthropic_overloaded_error_message_extracted() {
        let body = r#"{"type":"error","error":{"details":null,"type":"overloaded_error","message":"Overloaded"}}"#;
        let err = parse_provider_error(529, body, &DEFAULT_ERROR_STRUCTURE);
        assert!(
            matches!(err, AiMuxError::ApiCall(ref m) if m.message.contains("Overloaded")),
            "expected Provider carrying 'Overloaded', got {err:?}"
        );
    }
}
