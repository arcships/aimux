//! HTTP-level tests for the Google Gemini provider.
//!
//! These tests spin up a `wiremock` mock server, configure it with a JSON or
//! SSE response, point a `GoogleProvider` at the mock, and assert on the
//! `do_generate` / `do_stream` results and the request bodies.
//!
//! The response fixtures mirror the shape of the real Gemini API
//! (`candidates[].content.parts[]`, `usageMetadata`, `finishReason`).
//! Reference: `reference/ai/packages/google/src/__fixtures__/`.

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, Tool, ToolChoice};
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::google::convert::{
    build_request_body, convert_to_google_messages, prepare_tools,
};
use aimux_providers::{GoogleConfig, GoogleProvider};

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

/// Build `CallOptions` with tools.
fn options_with_tools(prompt: LanguageModelPrompt, tools: Vec<FunctionTool>) -> CallOptions {
    CallOptions {
        prompt,
        max_output_tokens: None,
        temperature: None,
        stop_sequences: None,
        top_p: None,
        top_k: None,
        presence_penalty: None,
        frequency_penalty: None,
        response_format: None,
        seed: None,
        tools: Some(tools.into_iter().map(Tool::from).collect()),
        tool_choice: ToolChoice::Auto,
        headers: None,
        provider_options: None,
        reasoning: None,
        body_overrides: None,
        max_retries: None,
        timeout: None,
        abort_signal: None,
        session_id: None,
        include_raw_chunks: None,
        call_id: None,
        recording_context: None,
    }
}

/// A simple function tool named `weather` with a `location` string parameter.
fn weather_tool() -> FunctionTool {
    FunctionTool::new(
        "weather".to_string(),
        json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false,
        }),
    )
    .with_description("Get the weather".to_string())
}

/// Mock a JSON `generateContent` response.
async fn mock_json_response(server: &MockServer, model: &str, body: Value) {
    Mock::given(method("POST"))
        .and(path(format!("/models/{model}:generateContent")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mock a JSON `generateContent` response with a custom status code.
async fn mock_json_error(server: &MockServer, model: &str, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path(format!("/models/{model}:generateContent")))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// Mock an SSE `streamGenerateContent` response.
async fn mock_sse_response(server: &MockServer, model: &str, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/models/{model}:streamGenerateContent")))
        .and(query_param("alt", "sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

/// Build an SSE event string from a JSON value.
fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

/// Concatenate SSE events (no `[DONE]` sentinel — Gemini doesn't use one).
fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body
}

/// Collect all `StreamPart`s from a `StreamResult` into a `Vec`.
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

// ════════════════════════════════════════════════════════════════════════════
// doGenerate
// ════════════════════════════════════════════════════════════════════════════

mod do_generate {
    use super::*;

    // ── should extract text response ──────────────────────────────────────────

    #[tokio::test]
    async fn should_extract_text_response() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "text": "Hello, World!" }],
                        "role": "model"
                    },
                    "finishReason": "STOP",
                    "index": 0
                }],
                "usageMetadata": {
                    "promptTokenCount": 4,
                    "candidatesTokenCount": 30,
                    "totalTokenCount": 34
                }
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "Hello, World!"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // ── should extract usage ──────────────────────────────────────────────────

    #[tokio::test]
    async fn should_extract_usage() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "text": "Hello, World!" }],
                        "role": "model"
                    },
                    "finishReason": "STOP",
                    "index": 0
                }],
                "usageMetadata": {
                    "promptTokenCount": 20,
                    "candidatesTokenCount": 5,
                    "totalTokenCount": 25,
                    "cachedContentTokenCount": 3,
                    "thoughtsTokenCount": 2
                }
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        // inputTokens.total = 20, noCache = 20 - 3 = 17, cacheRead = 3
        assert_eq!(result.usage.input_tokens.total, Some(20));
        assert_eq!(result.usage.input_tokens.no_cache, Some(17));
        assert_eq!(result.usage.input_tokens.cache_read, Some(3));
        // outputTokens.total = candidatesTokenCount(5) + thoughtsTokenCount(2) = 7
        assert_eq!(result.usage.output_tokens.total, Some(7));
    }

    // ── should map STOP finish reason ─────────────────────────────────────────

    #[tokio::test]
    async fn should_map_stop_finish_reason() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("STOP"));
    }

    // ── should map MAX_TOKENS finish reason ───────────────────────────────────

    #[tokio::test]
    async fn should_map_max_tokens_finish_reason() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "…" }], "role": "model" },
                    "finishReason": "MAX_TOKENS",
                    "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Length);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("MAX_TOKENS"));
    }

    // ── should map SAFETY finish reason ───────────────────────────────────────

    #[tokio::test]
    async fn should_map_safety_finish_reason() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [], "role": "model" },
                    "finishReason": "SAFETY",
                    "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(
            result.finish_reason.unified,
            FinishReasonUnified::ContentFilter
        );
    }

    // ── should map MALFORMED_FUNCTION_CALL finish reason ──────────────────────

    #[tokio::test]
    async fn should_map_malformed_function_call_finish_reason() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": {},
                    "finishReason": "MALFORMED_FUNCTION_CALL"
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Error);
        assert_eq!(
            result.finish_reason.raw.as_deref(),
            Some("MALFORMED_FUNCTION_CALL")
        );
    }

    // ── should extract a tool call (functionCall part) ────────────────────────

    #[tokio::test]
    async fn should_extract_tool_call() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{
                            "functionCall": {
                                "id": "call-1",
                                "name": "weather",
                                "args": { "location": "San Francisco" }
                            }
                        }],
                        "role": "model"
                    },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&options_with_tools(test_prompt(), vec![weather_tool()]))
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
                assert_eq!(tool_call_id, "call-1");
                assert_eq!(tool_name, "weather");
                assert_eq!(input, &json!(r#"{"location":"San Francisco"}"#));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        // STOP + has_tool_calls -> ToolCalls
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    }

    // ── should extract tool call with thought signature (thinking models) ────

    #[tokio::test]
    async fn should_extract_tool_call_with_thought_signature() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.5-pro",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{
                            "functionCall": {
                                "id": "call-1",
                                "name": "weather",
                                "args": { "location": "San Francisco" }
                            },
                            "thoughtSignature": "EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH"
                        }],
                        "role": "model"
                    },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.5-pro");

        let result = model
            .do_generate(&options_with_tools(test_prompt(), vec![weather_tool()]))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                thought_signature,
                ..
            } => {
                assert_eq!(tool_call_id, "call-1");
                assert_eq!(tool_name, "weather");
                assert_eq!(input, &json!(r#"{"location":"San Francisco"}"#));
                assert_eq!(
                    thought_signature.as_deref(),
                    Some("EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH")
                );
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    // ── should expose response id ─────────────────────────────────────────────

    #[tokio::test]
    async fn should_expose_response_id() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }],
                "responseId": "TestResponseId123"
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.response.id, Some("TestResponseId123".to_string()));
    }

    // ── should return the request body for debugging ──────────────────────────

    #[tokio::test]
    async fn should_return_request_body() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let body = result
            .request_body
            .as_ref()
            .expect("request body should be present");
        // contents should be [{ role: "user", parts: [{ text: "Hello" }] }]
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "Hello");
    }

    // ── should expose finishMessage in provider metadata ─────────────────────
    // TS: "should expose finishMessage in provider metadata"

    #[tokio::test]
    async fn should_expose_finish_message_in_provider_metadata() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": {},
                    "finishReason": "MALFORMED_FUNCTION_CALL",
                    "finishMessage": "Malformed function call: print(default_api.create(name='test'))"
                }],
                "usageMetadata": { "promptTokenCount": 130, "totalTokenCount": 130 }
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider metadata");
        assert_eq!(
            pm["google"]["finishMessage"],
            "Malformed function call: print(default_api.create(name='test'))"
        );
    }

    // ── should expose null finishMessage when not present ────────────────────

    #[tokio::test]
    async fn should_expose_null_finish_message_when_not_present() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "test response" }], "role": "model" },
                    "finishReason": "STOP", "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider metadata");
        assert!(pm["google"]["finishMessage"].is_null());
    }

    // ── should expose safety ratings in provider metadata ────────────────────
    // TS: "should expose safety ratings in provider metadata"

    #[tokio::test]
    async fn should_expose_safety_ratings_in_provider_metadata() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "test response" }], "role": "model" },
                    "finishReason": "STOP", "index": 0,
                    "safetyRatings": [{
                        "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                        "probability": "NEGLIGIBLE",
                        "probabilityScore": 0.1,
                        "severity": "LOW",
                        "severityScore": 0.2,
                        "blocked": false
                    }]
                }],
                "promptFeedback": { "safetyRatings": [
                    { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "probability": "NEGLIGIBLE" }
                ]}
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider metadata");
        let ratings = pm["google"]["safetyRatings"].as_array().expect("array");
        assert_eq!(ratings.len(), 1);
        assert_eq!(ratings[0]["category"], "HARM_CATEGORY_DANGEROUS_CONTENT");
        assert_eq!(ratings[0]["probability"], "NEGLIGIBLE");
        assert_eq!(ratings[0]["probabilityScore"], 0.1);
        assert_eq!(ratings[0]["blocked"], false);
    }

    // ── should expose PromptFeedback in provider metadata ────────────────────
    // TS: "should expose PromptFeedback in provider metadata"

    #[tokio::test]
    async fn should_expose_prompt_feedback_in_provider_metadata() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "No" }], "role": "model" },
                    "finishReason": "SAFETY", "index": 0
                }],
                "promptFeedback": {
                    "blockReason": "SAFETY",
                    "safetyRatings": [
                        { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "probability": "NEGLIGIBLE" },
                        { "category": "HARM_CATEGORY_HATE_SPEECH", "probability": "NEGLIGIBLE" },
                        { "category": "HARM_CATEGORY_HARASSMENT", "probability": "NEGLIGIBLE" },
                        { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "probability": "NEGLIGIBLE" }
                    ]
                }
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider metadata");
        let pf = &pm["google"]["promptFeedback"];
        assert_eq!(pf["blockReason"], "SAFETY");
        let ratings = pf["safetyRatings"].as_array().expect("array");
        assert_eq!(ratings.len(), 4);
    }

    // ── should expose grounding metadata in provider metadata ────────────────
    // TS: "should expose grounding metadata in provider metadata"

    #[tokio::test]
    async fn should_expose_grounding_metadata_in_provider_metadata() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "test response" }], "role": "model" },
                    "finishReason": "STOP", "index": 0,
                    "groundingMetadata": {
                        "webSearchQueries": ["What's the weather?"],
                        "groundingChunks": [{
                            "web": { "uri": "https://example.com/weather", "title": "Forecast" }
                        }]
                    }
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider metadata");
        let gm = &pm["google"]["groundingMetadata"];
        assert_eq!(gm["webSearchQueries"][0], "What's the weather?");
        assert_eq!(
            gm["groundingChunks"][0]["web"]["uri"],
            "https://example.com/weather"
        );
    }

    // ── should expose response headers ───────────────────────────────────────
    // TS: "should expose the raw response headers"

    #[tokio::test]
    async fn should_expose_response_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-2.0-flash:generateContent"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("test-header", "test-value")
                    .set_body_json(json!({
                        "candidates": [{
                            "content": { "parts": [{ "text": "hi" }], "role": "model" },
                            "finishReason": "STOP", "index": 0
                        }]
                    })),
            )
            .mount(&server)
            .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let headers = result.response_headers.expect("response headers");
        assert_eq!(headers.get("test-header"), Some(&"test-value".to_string()));
    }

    // ── should handle empty content with MALFORMED_FUNCTION_CALL ─────────────
    // TS: "should handle MALFORMED_FUNCTION_CALL finish reason and empty content object"

    #[tokio::test]
    async fn should_handle_empty_content_with_malformed_function_call() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{ "content": {}, "finishReason": "MALFORMED_FUNCTION_CALL" }],
                "usageMetadata": { "promptTokenCount": 9056, "totalTokenCount": 9056 }
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert!(result.content.is_empty(), "content should be empty");
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Error);
        assert_eq!(
            result.finish_reason.raw.as_deref(),
            Some("MALFORMED_FUNCTION_CALL")
        );
    }

    // ── should extract tool call without id ──────────────────────────────────

    #[tokio::test]
    async fn should_extract_tool_call_without_id() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "functionCall": { "name": "weather", "args": { "location": "SF" } } }],
                        "role": "model"
                    },
                    "finishReason": "STOP", "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&options_with_tools(test_prompt(), vec![weather_tool()]))
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
                assert_eq!(tool_call_id, "");
                assert_eq!(tool_name, "weather");
                assert_eq!(input, &json!(r#"{"location":"SF"}"#));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    }

    // ── should extract multiple tool calls ───────────────────────────────────

    #[tokio::test]
    async fn should_extract_multiple_tool_calls() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "functionCall": { "id": "call-1", "name": "weather", "args": { "location": "SF" } } },
                            { "functionCall": { "id": "call-2", "name": "calendar", "args": { "date": "2024-01-01" } } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP", "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&options_with_tools(test_prompt(), vec![weather_tool()]))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.content.len(), 2);
        match &result.content[0] {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                ..
            } => {
                assert_eq!(tool_call_id, "call-1");
                assert_eq!(tool_name, "weather");
            }
            other => panic!("expected first ToolCall, got {other:?}"),
        }
        match &result.content[1] {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                assert_eq!(tool_call_id, "call-2");
                assert_eq!(tool_name, "calendar");
                assert_eq!(input, &json!(r#"{"date":"2024-01-01"}"#));
            }
            other => panic!("expected second ToolCall, got {other:?}"),
        }
    }

    // ── should extract text and tool call together ───────────────────────────

    #[tokio::test]
    async fn should_extract_text_and_tool_call() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            "gemini-2.0-flash",
            json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            { "text": "Let me check the weather." },
                            { "functionCall": { "id": "call-1", "name": "weather", "args": { "location": "SF" } } }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP", "index": 0
                }]
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&options_with_tools(test_prompt(), vec![weather_tool()]))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.content.len(), 2);
        match &result.content[0] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "Let me check the weather."),
            other => panic!("expected Text, got {other:?}"),
        }
        match &result.content[1] {
            GenerateContent::ToolCall { tool_name, .. } => assert_eq!(tool_name, "weather"),
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    // ── should map RECITATION finish reason ──────────────────────────────────

    #[tokio::test]
    async fn should_map_recitation_finish_reason() {
        let server = MockServer::start().await;
        mock_json_response(
            &server, "gemini-2.0-flash",
            json!({ "candidates": [{ "content": { "parts": [], "role": "model" }, "finishReason": "RECITATION", "index": 0 }] }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("ok");
        assert_eq!(
            result.finish_reason.unified,
            FinishReasonUnified::ContentFilter
        );
        assert_eq!(result.finish_reason.raw.as_deref(), Some("RECITATION"));
    }

    // ── should map unknown finish reason to Other ────────────────────────────

    #[tokio::test]
    async fn should_map_unknown_finish_reason_to_other() {
        let server = MockServer::start().await;
        mock_json_response(
            &server, "gemini-2.0-flash",
            json!({ "candidates": [{ "content": { "parts": [{ "text": "hi" }], "role": "model" }, "finishReason": "UNKNOWN_FUTURE", "index": 0 }] }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("ok");
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("UNKNOWN_FUTURE"));
    }

    // ── should return default usage when usageMetadata is missing ────────────

    #[tokio::test]
    async fn should_return_default_usage_when_missing() {
        let server = MockServer::start().await;
        mock_json_response(
            &server, "gemini-2.0-flash",
            json!({ "candidates": [{ "content": { "parts": [{ "text": "hi" }], "role": "model" }, "finishReason": "STOP", "index": 0 }] }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("ok");
        assert_eq!(result.usage.input_tokens.total, None);
        assert_eq!(result.usage.output_tokens.total, None);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doStream
// ════════════════════════════════════════════════════════════════════════════

mod do_stream {
    use super::*;

    // ── should stream text deltas ─────────────────────────────────────────────

    #[tokio::test]
    async fn should_stream_text_deltas() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"},"index":0}],"responseId":"resp-1"}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":", World"}],"role":"model"},"index":0}],"responseId":"resp-1"}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"!"}],"role":"model"},"finishReason":"STOP","index":0}],"responseId":"resp-1"}"#,
            ),
        ]);
        mock_sse_response(&server, "gemini-2.0-flash", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // stream-start
        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));

        // response-metadata
        match &parts[1] {
            StreamPart::ResponseMetadata { id, .. } => {
                assert_eq!(id.as_deref(), Some("resp-1"));
            }
            other => panic!("expected ResponseMetadata, got {other:?}"),
        }

        // text-start
        let _text_start = parts
            .iter()
            .find(|p| matches!(p, StreamPart::TextStart { .. }))
            .expect("should have TextStart");

        // text-deltas
        let deltas = text_deltas(&parts);
        assert_eq!(
            deltas,
            vec!["Hello".to_string(), ", World".to_string(), "!".to_string(),]
        );

        // finish
        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { finish_reason, .. }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
                assert_eq!(finish_reason.raw.as_deref(), Some("STOP"));
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // ── should stream a tool call ─────────────────────────────────────────────

    #[tokio::test]
    async fn should_stream_tool_call() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"functionCall":{"id":"call-1","name":"weather","args":{"location":"San Francisco"}}}],"role":"model"},"index":0}],"responseId":"resp-2"}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},"finishReason":"STOP","index":0}],"responseId":"resp-2"}"#,
            ),
        ]);
        mock_sse_response(&server, "gemini-2.0-flash", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_stream(&options_with_tools(test_prompt(), vec![weather_tool()]))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Verify tool-input-start
        let tool_start = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolInputStart { id, tool_name, .. } if id == "call-1" && tool_name == "weather")
        });
        assert!(tool_start.is_some(), "should have ToolInputStart");

        // Verify tool-input-end
        let tool_end = parts
            .iter()
            .find(|p| matches!(p, StreamPart::ToolInputEnd { id, .. } if id == "call-1"));
        assert!(tool_end.is_some(), "should have ToolInputEnd");

        // Verify tool-call
        let tool_call = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolCall { tool_call_id, tool_name, input, .. }
                if tool_call_id == "call-1"
                && tool_name == "weather"
                && input == &json!(r#"{"location":"San Francisco"}"#))
        });
        assert!(
            tool_call.is_some(),
            "should have ToolCall with correct input"
        );

        // Verify finish with tool-calls reason (STOP + has_tool_calls)
        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { finish_reason, .. }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // ── should stream tool call with thought signature (thinking models) ──────

    #[tokio::test]
    async fn should_stream_tool_call_with_thought_signature() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"functionCall":{"id":"call-1","name":"weather","args":{"location":"San Francisco"}},"thoughtSignature":"EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH"}],"role":"model"},"index":0}],"responseId":"resp-2"}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},"finishReason":"STOP","index":0}],"responseId":"resp-2"}"#,
            ),
        ]);
        mock_sse_response(&server, "gemini-2.5-pro", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.5-pro");

        let result = model
            .do_stream(&options_with_tools(test_prompt(), vec![weather_tool()]))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // The complete tool-call event carries the thought signature.
        let tool_call = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolCall { tool_call_id, thought_signature, .. }
                if tool_call_id == "call-1"
                && thought_signature.as_deref()
                    == Some("EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH"))
        });
        assert!(
            tool_call.is_some(),
            "should have ToolCall carrying the thought signature"
        );
    }

    // ── empty-text thoughtSignature while a reasoning block is open ───────────
    // Regression: the signature used to be silently dropped unless a *text*
    // block was open. Gemini thinking models emit the signature on an
    // empty-text part right after the thought text — while the reasoning
    // block is the open one.

    #[tokio::test]
    async fn should_stream_empty_text_signature_onto_open_reasoning_block() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"Thinking hard","thought":true}],"role":"model"},"index":0}],"responseId":"resp-3"}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"","thoughtSignature":"sig-on-empty-text"}],"role":"model"},"index":0}],"responseId":"resp-3"}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"The answer"}],"role":"model"},"finishReason":"STOP","index":0}],"responseId":"resp-3"}"#,
            ),
        ]);
        mock_sse_response(&server, "gemini-2.5-pro", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.5-pro");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // The reasoning block opens with id "0" and receives the thought text.
        let reasoning_delta = parts.iter().find(
            |p| matches!(p, StreamPart::ReasoningDelta { delta, .. } if delta == "Thinking hard"),
        );
        assert!(
            reasoning_delta.is_some(),
            "thought text should stream as ReasoningDelta"
        );

        // The signature on the empty-text part must attach to the open
        // reasoning block as a zero-length ReasoningDelta — not be dropped.
        let sig_delta = parts.iter().find(|p| {
            matches!(p, StreamPart::ReasoningDelta { id, delta, provider_metadata }
                if id == "0"
                && delta.is_empty()
                && provider_metadata.as_ref().and_then(|m| m["google"]["thoughtSignature"].as_str())
                    == Some("sig-on-empty-text"))
        });
        assert!(
            sig_delta.is_some(),
            "empty-text thoughtSignature must ride the open reasoning block, got {parts:?}"
        );
    }

    // ── should expose usage from the final chunk ──────────────────────────────

    #[tokio::test]
    async fn should_expose_usage_from_final_chunk() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"Hi"}],"role":"model"},"index":0}]}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":3,"totalTokenCount":13}}"#,
            ),
        ]);
        mock_sse_response(&server, "gemini-2.0-flash", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { usage, .. }) => {
                assert_eq!(usage.input_tokens.total, Some(10));
                assert_eq!(usage.output_tokens.total, Some(3));
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // ── should stream text then tool call ────────────────────────────────────
    // TS: "should set finishReason to tool-calls when chunk contains functionCall"

    #[tokio::test]
    async fn should_stream_text_then_tool_call() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"Initial text response"}],"role":"model"},"index":0}]}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"test-tool","args":{"value":"example value"}}}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":20,"totalTokenCount":30}}"#,
            ),
        ]);
        mock_sse_response(&server, "gemini-2.0-flash", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_stream(&options_with_tools(test_prompt(), vec![weather_tool()]))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Should have text deltas
        let deltas = text_deltas(&parts);
        assert_eq!(deltas, vec!["Initial text response".to_string()]);

        // Should have a tool call
        let tool_call = parts.iter().find(
            |p| matches!(p, StreamPart::ToolCall { tool_name, .. } if tool_name == "test-tool"),
        );
        assert!(tool_call.is_some(), "should have ToolCall");

        // STOP + has_tool_calls → ToolCalls
        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish {
                finish_reason,
                usage,
                ..
            }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
                assert_eq!(finish_reason.raw.as_deref(), Some("STOP"));
                assert_eq!(usage.input_tokens.total, Some(10));
                assert_eq!(usage.output_tokens.total, Some(20));
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // ── should stream with no candidates (only usage) ────────────────────────

    #[tokio::test]
    async fn should_stream_with_no_candidates() {
        let server = MockServer::start().await;
        let body = sse_body(&[&sse_event(
            r#"{"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":0,"totalTokenCount":5}}"#,
        )]);
        mock_sse_response(&server, "gemini-2.0-flash", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // No text deltas
        let deltas = text_deltas(&parts);
        assert!(deltas.is_empty(), "should have no text deltas");

        // Should still have a Finish with usage
        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { usage, .. }) => {
                assert_eq!(usage.input_tokens.total, Some(5));
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // ── should stream response metadata with model version ───────────────────
    // TS: "should emit response-metadata with the provider responseId"

    #[tokio::test]
    async fn should_stream_response_metadata_with_model_version() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"Hi"}],"role":"model"},"index":0}],"responseId":"resp-xyz","modelVersion":"gemini-2.0-flash-001"}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
            ),
        ]);
        mock_sse_response(&server, "gemini-2.0-flash", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let rm = parts
            .iter()
            .find(|p| matches!(p, StreamPart::ResponseMetadata { .. }));
        match rm {
            Some(StreamPart::ResponseMetadata { id, model_id, .. }) => {
                assert_eq!(id.as_deref(), Some("resp-xyz"));
                assert_eq!(model_id.as_deref(), Some("gemini-2.0-flash-001"));
            }
            other => panic!("expected ResponseMetadata, got {other:?}"),
        }
    }

    // ── should close text segment on finish ──────────────────────────────────

    #[tokio::test]
    async fn should_close_text_segment_on_finish() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":"Hello world"}],"role":"model"},"index":0}]}"#,
            ),
            &sse_event(
                r#"{"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
            ),
        ]);
        mock_sse_response(&server, "gemini-2.0-flash", &body).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Should have TextStart and TextEnd
        let text_start = parts
            .iter()
            .find(|p| matches!(p, StreamPart::TextStart { .. }));
        assert!(text_start.is_some(), "should have TextStart");

        let text_end = parts
            .iter()
            .find(|p| matches!(p, StreamPart::TextEnd { .. }));
        assert!(text_end.is_some(), "should have TextEnd");

        // TextStart id should match TextEnd id
        if let (
            Some(StreamPart::TextStart { id: start_id, .. }),
            Some(StreamPart::TextEnd { id: end_id, .. }),
        ) = (text_start, text_end)
        {
            assert_eq!(start_id, end_id);
        }
    }
}
// ════════════════════════════════════════════════════════════════════════════

mod error_handling {
    use super::*;

    // ── should surface 401 as Auth ────────────────────────────────────────────

    #[tokio::test]
    async fn should_surface_401_as_auth_error() {
        let server = MockServer::start().await;
        mock_json_error(
            &server,
            "gemini-2.0-flash",
            401,
            json!({
                "error": {
                    "code": 401,
                    "message": "API key not valid.",
                    "status": "UNAUTHENTICATED"
                }
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("should error");

        assert_eq!(err.status_code(), Some(401), "got {err:?}");
        assert!(err.to_string().contains("API key not valid."));
    }

    // ── should surface 404 as ModelNotFound ───────────────────────────────────

    #[tokio::test]
    async fn should_surface_404_as_model_not_found() {
        let server = MockServer::start().await;
        mock_json_error(
            &server,
            "gemini-2.0-flash",
            404,
            json!({
                "error": {
                    "code": 404,
                    "message": "models/not-a-model is not found",
                    "status": "NOT_FOUND"
                }
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("should error");

        assert_eq!(err.status_code(), Some(404), "got {err:?}");
        assert!(err.to_string().contains("not found"));
    }

    // ── should surface 429 as RateLimited ─────────────────────────────────────

    #[tokio::test]
    async fn should_surface_429_as_rate_limited() {
        let server = MockServer::start().await;
        mock_json_error(
            &server,
            "gemini-2.0-flash",
            429,
            json!({
                "error": {
                    "code": 429,
                    "message": "Resource exhausted.",
                    "status": "RESOURCE_EXHAUSTED"
                }
            }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("should error");

        assert!(matches!(err, ref e if e.status_code() == Some(429)));
    }

    // ── should surface 500 as Provider ────────────────────────────────────────

    #[tokio::test]
    async fn should_surface_500_as_provider_error() {
        let server = MockServer::start().await;
        mock_json_error(
            &server,
            "gemini-2.0-flash",
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

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("should error");

        match err {
            AiMuxError::ApiCall(d) => assert!(d.to_string().contains("Internal error")),
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    // ── should surface stream errors ──────────────────────────────────────────

    #[tokio::test]
    async fn should_surface_stream_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-2.0-flash:streamGenerateContent"))
            .and(query_param("alt", "sse"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": { "code": 401, "message": "bad key", "status": "UNAUTHENTICATED" }
            })))
            .mount(&server)
            .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect_err("should error");

        assert!(matches!(err, ref e if e.status_code() == Some(401)));
    }

    // ── should surface 400 as Provider ───────────────────────────────────────

    #[tokio::test]
    async fn should_surface_400_as_provider_error() {
        let server = MockServer::start().await;
        mock_json_error(
            &server, "gemini-2.0-flash", 400,
            json!({ "error": { "code": 400, "message": "Invalid request.", "status": "INVALID_ARGUMENT" } }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("should error");
        match err {
            AiMuxError::ApiCall(msg_d) => {
                let msg = msg_d.to_string();
                assert!(msg.contains("Invalid request"), "msg was: {msg}")
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    // ── should surface 403 as Provider ───────────────────────────────────────

    #[tokio::test]
    async fn should_surface_403_as_provider_error() {
        let server = MockServer::start().await;
        mock_json_error(
            &server, "gemini-2.0-flash", 403,
            json!({ "error": { "code": 403, "message": "Permission denied.", "status": "PERMISSION_DENIED" } }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("should error");
        match err {
            AiMuxError::ApiCall(msg_d) => {
                let msg = msg_d.to_string();
                assert!(msg.contains("Permission denied"), "msg was: {msg}")
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    // ── should surface 500 on stream as Provider ─────────────────────────────

    #[tokio::test]
    async fn should_surface_stream_500_as_provider_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-2.0-flash:streamGenerateContent"))
            .and(query_param("alt", "sse"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "error": { "code": 500, "message": "Internal error.", "status": "INTERNAL" }
            })))
            .mount(&server)
            .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect_err("should error");
        match err {
            AiMuxError::ApiCall(msg_d) => {
                let msg = msg_d.to_string();
                assert!(msg.contains("Internal error"), "msg was: {msg}")
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    // ── should surface 404 on stream as ModelNotFound ────────────────────────

    #[tokio::test]
    async fn should_surface_stream_404_as_model_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-2.0-flash:streamGenerateContent"))
            .and(query_param("alt", "sse"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "error": { "code": 404, "message": "model not found", "status": "NOT_FOUND" }
            })))
            .mount(&server)
            .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let err = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect_err("should error");
        assert!(matches!(err, ref e if e.status_code() == Some(404)));
    }
}
// ════════════════════════════════════════════════════════════════════════════

mod request_body {
    use super::*;

    // ── system message → top-level systemInstruction ──────────────────────────

    #[test]
    fn system_message_becomes_system_instruction() {
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::System,
                content: vec![ContentPart::text("You are a robot.")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hi")],
                ..Default::default()
            },
        ];
        let options = default_options(prompt);
        let body = build_request_body("gemini-2.0-flash", &options);

        // systemInstruction is a top-level field, NOT in contents.
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "You are a robot."
        );
        // contents should have only the user message.
        assert_eq!(body["contents"].as_array().unwrap().len(), 1);
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "Hi");
    }

    // ── no system message → no systemInstruction ──────────────────────────────

    #[test]
    fn no_system_message_omits_system_instruction() {
        let body = build_request_body("gemini-2.0-flash", &default_options(test_prompt()));
        assert!(body.get("systemInstruction").is_none());
    }

    // ── assistant role → model role ───────────────────────────────────────────

    #[test]
    fn assistant_role_becomes_model_role() {
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::text("Hi there")],
                ..Default::default()
            },
        ];
        let body = build_request_body("gemini-2.0-flash", &default_options(prompt));

        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["contents"][1]["parts"][0]["text"], "Hi there");
    }

    // ── assistant tool call → functionCall part ───────────────────────────────

    #[test]
    fn assistant_tool_call_becomes_function_call_part() {
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("What's the weather?")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::tool_call(
                    "call-1".to_string(),
                    "weather".to_string(),
                    json!({ "location": "SF" }),
                )],
                ..Default::default()
            },
        ];
        let body = build_request_body("gemini-2.0-flash", &default_options(prompt));

        let fc = &body["contents"][1]["parts"][0]["functionCall"];
        assert_eq!(fc["id"], "call-1");
        assert_eq!(fc["name"], "weather");
        assert_eq!(fc["args"]["location"], "SF");
        // No signature on the part → no thoughtSignature emitted.
        assert!(
            body["contents"][1]["parts"][0]
                .get("thoughtSignature")
                .is_none()
        );
    }

    // ── assistant tool call with thought signature → part sibling ─────────────

    #[test]
    fn assistant_tool_call_thought_signature_becomes_part_sibling() {
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("What's the weather?")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::ToolCall {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "weather".to_string(),
                    input: json!({ "location": "SF" }),
                    provider_executed: None,
                    thought_signature: Some(
                        "EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH".to_string(),
                    ),
                    provider_options: None,
                }],
                ..Default::default()
            },
        ];
        let body = build_request_body("gemini-2.5-pro", &default_options(prompt));

        // The signature must be a SIBLING of `functionCall` on the part
        // (matching the response shape), not nested inside `functionCall`.
        let part = &body["contents"][1]["parts"][0];
        assert_eq!(
            part["thoughtSignature"],
            "EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH"
        );
        assert!(part["functionCall"].get("thoughtSignature").is_none());
    }

    #[test]
    fn assistant_server_tool_round_trips_with_shared_google_vertex_wire_shape() {
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Search the web")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![
                    ContentPart::ToolCall {
                        tool_call_id: "logical-call-id".to_string(),
                        tool_name: "server:GOOGLE_SEARCH_WEB".to_string(),
                        input: json!(r#"{"query":"Singapore weather"}"#),
                        provider_executed: Some(true),
                        thought_signature: None,
                        provider_options: Some(json!({
                            "google": {
                                "serverToolCallId": "server-call-1",
                                "serverToolType": "GOOGLE_SEARCH_WEB",
                                "thoughtSignature": "call-signature"
                            }
                        })),
                    },
                    ContentPart::ToolResult {
                        tool_call_id: "logical-call-id".to_string(),
                        tool_name: Some("server:GOOGLE_SEARCH_WEB".to_string()),
                        result: json!({ "results": [{ "title": "Sunny" }] }),
                        is_error: None,
                        preliminary: None,
                        dynamic: None,
                        provider_options: Some(json!({
                            "google": {
                                "serverToolCallId": "server-call-1",
                                "serverToolType": "GOOGLE_SEARCH_WEB",
                                "thoughtSignature": "result-signature"
                            }
                        })),
                    },
                ],
                ..Default::default()
            },
        ];

        let body = build_request_body("gemini-3-pro-preview", &default_options(prompt));
        assert_eq!(
            body["contents"][1]["parts"],
            json!([
                {
                    "toolCall": {
                        "toolType": "GOOGLE_SEARCH_WEB",
                        "args": { "query": "Singapore weather" },
                        "id": "server-call-1"
                    },
                    "thoughtSignature": "call-signature"
                },
                {
                    "toolResponse": {
                        "toolType": "GOOGLE_SEARCH_WEB",
                        "response": { "results": [{ "title": "Sunny" }] },
                        "id": "server-call-1"
                    },
                    "thoughtSignature": "result-signature"
                }
            ])
        );
    }

    // ── tool result → functionResponse part in a user message ─────────────────

    #[test]
    fn tool_result_becomes_function_response_part() {
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("weather?")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::tool_call(
                    "call-1".to_string(),
                    "weather".to_string(),
                    json!({ "location": "SF" }),
                )],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Tool,
                content: vec![ContentPart::tool_result(
                    "call-1".to_string(),
                    json!({ "temp": 70 }),
                )],
                ..Default::default()
            },
        ];
        let body = build_request_body("gemini-2.0-flash", &default_options(prompt));

        // The tool message becomes a user-role message with a functionResponse part.
        let tool_msg = &body["contents"][2];
        assert_eq!(tool_msg["role"], "user");
        let fr = &tool_msg["parts"][0]["functionResponse"];
        assert_eq!(fr["id"], "call-1");
        // Without a tool_name the call id is used as the required `name`
        // (fallback path).
        assert_eq!(fr["name"], "call-1");
        assert_eq!(fr["response"]["name"], "call-1");
        // The serialized JSON output is a string under response.content.
        let content_str = fr["response"]["content"].as_str().unwrap();
        assert!(content_str.contains("\"temp\":70"));
    }

    #[test]
    fn tool_result_uses_tool_name_for_function_response() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: "call-9".to_string(),
                result: json!({ "temp": 21 }),
                tool_name: Some("weather".to_string()),
                is_error: None,
                preliminary: None,
                dynamic: None,
                provider_options: None,
            }],
            ..Default::default()
        }];
        let body = build_request_body("gemini-2.0-flash", &default_options(prompt));

        let fr = &body["contents"][0]["parts"][0]["functionResponse"];
        assert_eq!(fr["id"], "call-9");
        // `name` carries the real tool name, not the opaque call id —
        // Gemini pairs functionResponse with the prior functionCall by name.
        assert_eq!(fr["name"], "weather");
        assert_eq!(fr["response"]["name"], "weather");
    }

    #[test]
    fn tool_result_blank_tool_name_falls_back_to_call_id() {
        // An explicitly blank tool_name is treated as unset and falls back
        // to the call id (locks the empty-string filter branch).
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult {
                tool_call_id: "call-blank".to_string(),
                result: json!({ "ok": true }),
                tool_name: Some(String::new()),
                is_error: None,
                preliminary: None,
                dynamic: None,
                provider_options: None,
            }],
            ..Default::default()
        }];
        let body = build_request_body("gemini-2.0-flash", &default_options(prompt));
        let fr = &body["contents"][0]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "call-blank");
    }

    // ── tools → tools array with functionDeclarations ─────────────────────────

    #[test]
    fn tools_become_function_declarations() {
        let body = build_request_body(
            "gemini-2.0-flash",
            &options_with_tools(test_prompt(), vec![weather_tool()]),
        );

        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        let decls = tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0]["name"], "weather");
        assert_eq!(decls[0]["description"], "Get the weather");
        // The JSON schema's $schema / additionalProperties are dropped by the
        // OpenAPI conversion; the resulting parameters have type/properties/required.
        let params = &decls[0]["parameters"];
        assert_eq!(params["type"], "object");
        assert_eq!(params["properties"]["location"]["type"], "string");
        assert_eq!(params["required"][0], "location");
    }

    // ── tool_choice auto → AUTO mode ──────────────────────────────────────────

    #[test]
    fn tool_choice_auto_becomes_auto_mode() {
        let body = build_request_body(
            "gemini-2.0-flash",
            &options_with_tools(test_prompt(), vec![weather_tool()]),
        );
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");
    }

    // ── tool_choice required → ANY mode ───────────────────────────────────────

    #[test]
    fn tool_choice_required_becomes_any_mode() {
        let mut opts = options_with_tools(test_prompt(), vec![weather_tool()]);
        opts.tool_choice = ToolChoice::Required;
        let body = build_request_body("gemini-2.0-flash", &opts);
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
    }

    // ── tool_choice tool → ANY + allowedFunctionNames ─────────────────────────

    #[test]
    fn tool_choice_tool_becomes_any_with_allowed_function_names() {
        let mut opts = options_with_tools(test_prompt(), vec![weather_tool()]);
        opts.tool_choice = ToolChoice::Tool {
            tool_name: "weather".to_string(),
        };
        let body = build_request_body("gemini-2.0-flash", &opts);
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
        let allowed = body["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"]
            .as_array()
            .unwrap();
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0], "weather");
    }

    // ── generation config maps standardized options ───────────────────────────

    #[test]
    fn generation_config_maps_standardized_options() {
        let opts = CallOptions {
            prompt: test_prompt(),
            max_output_tokens: Some(256),
            temperature: Some(0.5),
            top_p: Some(0.9),
            top_k: Some(40.0),
            stop_sequences: Some(vec!["STOP".to_string()]),
            ..default_options(test_prompt())
        };
        let body = build_request_body("gemini-2.0-flash", &opts);
        let gc = &body["generationConfig"];
        assert_eq!(gc["maxOutputTokens"], 256);
        assert_eq!(gc["temperature"], 0.5);
        // f32 -> f64 round-trip introduces tiny error; compare with tolerance.
        let top_p = gc["topP"].as_f64().unwrap();
        assert!((top_p - 0.9).abs() < 1e-5, "topP was {top_p}");
        assert_eq!(gc["topK"], 40.0);
        assert_eq!(gc["stopSequences"][0], "STOP");
    }

    // ── JSON response format → responseMimeType ───────────────────────────────

    #[test]
    fn json_response_format_sets_response_mime_type() {
        let mut opts = default_options(test_prompt());
        opts.response_format = Some(ResponseFormat::Json {
            schema: Some(json!({
                "type": "object",
                "properties": { "answer": { "type": "string" } },
                "required": ["answer"]
            })),
            name: None,
            description: None,
        });
        let body = build_request_body("gemini-2.0-flash", &opts);
        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(body["generationConfig"]["responseSchema"]["type"], "object");
    }

    // ── should pass seed in generationConfig ──────────────────────────────────
    // TS: "should pass the model, messages, and options" (seed + temperature)

    #[test]
    fn should_pass_seed_in_generation_config() {
        let mut opts = default_options(test_prompt());
        opts.seed = Some(123);
        opts.temperature = Some(0.5);
        let body = build_request_body("gemini-2.0-flash", &opts);
        assert_eq!(body["generationConfig"]["seed"], 123);
        assert_eq!(body["generationConfig"]["temperature"], 0.5);
    }

    // ── should pass presence and frequency penalty ───────────────────────────

    #[test]
    fn should_pass_presence_and_frequency_penalty() {
        let mut opts = default_options(test_prompt());
        opts.presence_penalty = Some(0.5);
        opts.frequency_penalty = Some(0.3);
        let body = build_request_body("gemini-2.0-flash", &opts);
        // f32 -> f64 round-trip introduces tiny error; compare with tolerance.
        let pp = body["generationConfig"]["presencePenalty"]
            .as_f64()
            .unwrap();
        assert!((pp - 0.5).abs() < 1e-5, "presencePenalty was {pp}");
        let fp = body["generationConfig"]["frequencyPenalty"]
            .as_f64()
            .unwrap();
        assert!((fp - 0.3).abs() < 1e-5, "frequencyPenalty was {fp}");
    }

    // ── should omit generationConfig when no options set ─────────────────────

    #[test]
    fn should_omit_generation_config_when_empty() {
        let body = build_request_body("gemini-2.0-flash", &default_options(test_prompt()));
        // The Rust impl omits generationConfig when empty (unlike TS which sends {}).
        assert!(body.get("generationConfig").is_none());
    }

    // ── JSON response format without schema ──────────────────────────────────

    #[test]
    fn json_response_format_without_schema() {
        let mut opts = default_options(test_prompt());
        opts.response_format = Some(ResponseFormat::Json {
            schema: None,
            name: None,
            description: None,
        });
        let body = build_request_body("gemini-2.0-flash", &opts);
        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            "application/json"
        );
        // No schema → no responseSchema field.
        assert!(body["generationConfig"].get("responseSchema").is_none());
    }

    // ── should pass custom headers via do_generate ───────────────────────────
    // TS: "should pass headers"

    #[tokio::test]
    async fn should_pass_custom_headers() {
        let server = MockServer::start().await;
        mock_json_response(
            &server, "gemini-2.0-flash",
            json!({ "candidates": [{ "content": { "parts": [{ "text": "hi" }], "role": "model" }, "finishReason": "STOP", "index": 0 }] }),
        )
        .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let mut opts = default_options(test_prompt());
        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Custom-Request-Header".to_string(),
            "request-header-value".to_string(),
        );
        opts.headers = Some(headers);

        model
            .do_generate(&opts)
            .await
            .expect("do_generate should succeed");

        // wiremock recorded the request — verify the custom header was sent.
        let requests = server.received_requests().await;
        assert!(requests.is_some(), "should have recorded requests");
        let reqs = requests.unwrap();
        assert!(!reqs.is_empty());
        let custom_header = reqs[0]
            .headers
            .iter()
            .find(|(k, _)| k.as_str() == "custom-request-header");
        assert!(custom_header.is_some(), "custom header should be present");
        assert_eq!(
            custom_header.unwrap().1.to_str().unwrap(),
            "request-header-value"
        );
    }

    // ── should send api key via x-goog-api-key header ────────────────────────

    #[tokio::test]
    async fn should_send_api_key_header() {
        let server = MockServer::start().await;
        mock_json_response(
            &server, "gemini-2.0-flash",
            json!({ "candidates": [{ "content": { "parts": [{ "text": "hi" }], "role": "model" }, "finishReason": "STOP", "index": 0 }] }),
        )
        .await;

        let config = GoogleConfig::new("my-secret-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("ok");

        let requests = server.received_requests().await;
        let reqs = requests.as_ref().expect("recorded requests");
        let key_header = reqs[0]
            .headers
            .iter()
            .find(|(k, _)| k.as_str() == "x-goog-api-key");
        assert!(
            key_header.is_some(),
            "x-goog-api-key header should be present"
        );
        assert_eq!(key_header.unwrap().1.to_str().unwrap(), "my-secret-key");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convert_to_google_messages (pure-function tests)
// ════════════════════════════════════════════════════════════════════════════

mod convert_messages {
    use super::*;

    // ── multi-part user message ───────────────────────────────────────────────

    #[test]
    fn multi_part_user_message() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hello"), ContentPart::text("World")],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        assert!(gp.system_instruction.is_none());
        assert_eq!(gp.contents.len(), 1);
        assert_eq!(gp.contents[0]["role"], "user");
        let parts = gp.contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "Hello");
        assert_eq!(parts[1]["text"], "World");
    }

    // ── multiple system messages ──────────────────────────────────────────────

    #[test]
    fn multiple_system_messages_are_merged() {
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::System,
                content: vec![ContentPart::text("Rule 1")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::System,
                content: vec![ContentPart::text("Rule 2")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hi")],
                ..Default::default()
            },
        ];
        let gp = convert_to_google_messages(&prompt);
        let sys = gp.system_instruction.expect("system instruction");
        let parts = sys["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "Rule 1");
        assert_eq!(parts[1]["text"], "Rule 2");
        assert_eq!(gp.contents.len(), 1);
    }

    // ── system message after a user message is dropped ──────────────────────

    #[test]
    fn system_message_after_user_is_dropped() {
        // The TS SDK throws `UnsupportedFunctionalityError` for this ordering.
        // We can't return an error (function returns GooglePrompt, not Result),
        // so the late system message is dropped rather than folded into
        // `systemInstruction` (which would make Gemini treat it as a global rule).
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hi")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::System,
                content: vec![ContentPart::text("Late rule")],
                ..Default::default()
            },
        ];
        let gp = convert_to_google_messages(&prompt);
        // The late system message must NOT appear in systemInstruction.
        assert!(
            gp.system_instruction.is_none(),
            "late system message must not leak into systemInstruction"
        );
        assert_eq!(gp.contents.len(), 1);
        assert_eq!(gp.contents[0]["role"], "user");
    }

    // ── image part → inlineData ───────────────────────────────────────────────

    #[test]
    fn image_part_becomes_inline_data() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::image(
                vec![1, 2, 3, 4],
                "image/png".to_string(),
            )],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        let part = &gp.contents[0]["parts"][0];
        assert_eq!(part["inlineData"]["mimeType"], "image/png");
        // base64 of [1,2,3,4] = "AQIDBA=="
        assert_eq!(part["inlineData"]["data"], "AQIDBA==");
    }

    // ── file part → inlineData ────────────────────────────────────────────────
    // TS: "should add file parts for base64 encoded files"

    #[test]
    fn file_part_becomes_inline_data() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::file(vec![0, 1, 2, 3], "image/png".to_string())],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        assert!(gp.system_instruction.is_none());
        assert_eq!(gp.contents.len(), 1);
        let part = &gp.contents[0]["parts"][0];
        assert_eq!(part["inlineData"]["mimeType"], "image/png");
        // base64 of [0,1,2,3] = "AAECAw=="
        assert_eq!(part["inlineData"]["data"], "AAECAw==");
    }

    // ── tool result with string output ───────────────────────────────────────
    // TS: "should convert tool result messages to function responses"

    #[test]
    fn tool_result_with_string_output() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::tool_result(
                "testCallId".to_string(),
                json!("test result string"),
            )],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        assert_eq!(gp.contents.len(), 1);
        assert_eq!(gp.contents[0]["role"], "user");
        let fr = &gp.contents[0]["parts"][0]["functionResponse"];
        assert_eq!(fr["id"], "testCallId");
        // String outputs pass through as-is (not stringified).
        assert_eq!(fr["response"]["content"], "test result string");
    }

    // ── mixed text and image user message ────────────────────────────────────

    #[test]
    fn mixed_text_and_image_user_message() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![
                ContentPart::text("What's in this image?"),
                ContentPart::image(vec![1, 2, 3], "image/jpeg".to_string()),
            ],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        assert_eq!(gp.contents.len(), 1);
        let parts = gp.contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "What's in this image?");
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/jpeg");
    }

    // ── empty assistant text is skipped ──────────────────────────────────────

    #[test]
    fn empty_assistant_text_is_skipped() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::text("")],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        // Empty assistant text → no parts → no contents entry.
        assert!(gp.contents.is_empty());
    }

    // ── system only message (no user content) ────────────────────────────────
    // TS: "should store system message in system instruction"

    #[test]
    fn system_only_message() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::System,
            content: vec![ContentPart::text("You are a robot.")],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        let sys = gp.system_instruction.expect("system instruction");
        assert_eq!(sys["parts"][0]["text"], "You are a robot.");
        assert!(gp.contents.is_empty());
    }

    // ── tool call without id omits id field ──────────────────────────────────

    #[test]
    fn tool_call_without_id_omits_id_field() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::tool_call(
                "".to_string(),
                "weather".to_string(),
                json!({ "location": "SF" }),
            )],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        let fc = &gp.contents[0]["parts"][0]["functionCall"];
        assert_eq!(fc["name"], "weather");
        assert_eq!(fc["args"]["location"], "SF");
        // Empty tool_call_id → no "id" field emitted.
        assert!(fc.get("id").is_none());
    }

    // ── mixed assistant text and tool call ───────────────────────────────────

    #[test]
    fn mixed_assistant_text_and_tool_call() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![
                ContentPart::text("I'll check that for you."),
                ContentPart::tool_call(
                    "call-1".to_string(),
                    "weather".to_string(),
                    json!({ "location": "SF" }),
                ),
            ],
            ..Default::default()
        }];
        let gp = convert_to_google_messages(&prompt);
        assert_eq!(gp.contents[0]["role"], "model");
        let parts = gp.contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "I'll check that for you.");
        assert_eq!(parts[1]["functionCall"]["name"], "weather");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// prepare_tools (pure-function tests)
// ════════════════════════════════════════════════════════════════════════════

mod prepare_tools_tests {
    use super::*;

    // ── no tools → empty result ───────────────────────────────────────────────

    #[test]
    fn no_tools_yields_empty_result() {
        let result = prepare_tools(&None, &ToolChoice::Auto);
        assert!(result.tools.is_none());
        assert!(result.tool_config.is_none());
    }

    // ── empty tools array → empty result ──────────────────────────────────────

    #[test]
    fn empty_tools_array_yields_empty_result() {
        let result = prepare_tools(&Some(vec![]), &ToolChoice::Auto);
        assert!(result.tools.is_none());
        assert!(result.tool_config.is_none());
    }

    // ── strict tool → VALIDATED mode ──────────────────────────────────────────

    #[test]
    fn strict_tool_yields_validated_mode() {
        let mut tool = weather_tool();
        tool.strict = Some(true);
        let result = prepare_tools(&Some(vec![tool]), &ToolChoice::Auto);
        let tc = result.tool_config.expect("tool config");
        assert_eq!(tc["functionCallingConfig"]["mode"], "VALIDATED");
    }

    // ── tool_choice none → NONE mode ─────────────────────────────────────────
    // TS: "should handle tool choice 'none'"

    #[test]
    fn tool_choice_none_becomes_none_mode() {
        let result = prepare_tools(&Some(vec![weather_tool()]), &ToolChoice::None);
        assert!(result.tools.is_some());
        assert_eq!(
            result.tool_config.unwrap()["functionCallingConfig"]["mode"],
            "NONE"
        );
    }

    // ── strict tool with required choice → VALIDATED ─────────────────────────
    // TS: "should use VALIDATED mode with toolChoice required when strict: true"

    #[test]
    fn strict_tool_with_required_choice() {
        let mut tool = weather_tool();
        tool.strict = Some(true);
        let result = prepare_tools(&Some(vec![tool]), &ToolChoice::Required);
        let tc = result.tool_config.expect("tool config");
        assert_eq!(tc["functionCallingConfig"]["mode"], "VALIDATED");
    }

    // ── strict tool with tool choice → VALIDATED + allowedFunctionNames ──────

    #[test]
    fn strict_tool_with_tool_choice() {
        let mut tool = weather_tool();
        tool.strict = Some(true);
        let result = prepare_tools(
            &Some(vec![tool]),
            &ToolChoice::Tool {
                tool_name: "weather".to_string(),
            },
        );
        let tc = result.tool_config.expect("tool config");
        assert_eq!(tc["functionCallingConfig"]["mode"], "VALIDATED");
        let allowed = tc["functionCallingConfig"]["allowedFunctionNames"]
            .as_array()
            .unwrap();
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0], "weather");
    }

    // ── multiple tools → multiple functionDeclarations ───────────────────────

    #[test]
    fn multiple_tools_become_multiple_declarations() {
        let tool2 = FunctionTool::new(
            "calendar".to_string(),
            json!({
                "type": "object",
                "properties": { "date": { "type": "string" } },
                "required": ["date"],
                "additionalProperties": false,
            }),
        )
        .with_description("Get calendar".to_string());
        let result = prepare_tools(&Some(vec![weather_tool(), tool2]), &ToolChoice::Auto);
        let tools = result.tools.expect("tools");
        let decls = tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0]["name"], "weather");
        assert_eq!(decls[1]["name"], "calendar");
    }

    // ── tool without description → empty string ──────────────────────────────

    #[test]
    fn tool_without_description_uses_empty_string() {
        let tool = FunctionTool::new(
            "noDesc".to_string(),
            json!({ "type": "object", "properties": {} }),
        );
        let result = prepare_tools(&Some(vec![tool]), &ToolChoice::Auto);
        let tools = result.tools.unwrap();
        let decls = tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls[0]["name"], "noDesc");
        assert_eq!(decls[0]["description"], "");
    }

    // ── function tool with empty schema → null parameters ────────────────────
    // TS: "should correctly prepare function tools" (empty schema → parameters: undefined)

    #[test]
    fn function_tool_with_empty_schema_yields_null_parameters() {
        let tool = FunctionTool::new(
            "testFunction".to_string(),
            json!({ "type": "object", "properties": {} }),
        )
        .with_description("A test function".to_string());
        let result = prepare_tools(&Some(vec![tool]), &ToolChoice::Auto);
        let tools = result.tools.unwrap();
        let decls = tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls[0]["name"], "testFunction");
        assert_eq!(decls[0]["description"], "A test function");
        // Empty object schema at root → null parameters (OpenAPI conversion).
        assert!(decls[0]["parameters"].is_null());
        // No tool_config when not strict and choice is auto → AUTO.
        assert_eq!(
            result.tool_config.unwrap()["functionCallingConfig"]["mode"],
            "AUTO"
        );
    }
}
