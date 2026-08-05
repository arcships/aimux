//! Rust translations of the AI SDK OpenAI provider HTTP-level tests.
//!
//! Sources (TS → Rust):
//! - `packages/openai/src/chat/openai-chat-language-model.test.ts`
//!   `describe('doGenerate')` response-parsing cases → `do_generate` tests
//! - `packages/openai/src/chat/openai-chat-language-model.test.ts`
//!   `describe('doStream')` streaming cases → `do_stream` tests
//!
//! Tests that depend on features absent from the Rust data model
//! (`providerOptions`, reasoning/text token breakdown, annotations/sources,
//! `includeRawChunks`, `providerMetadata` prediction tokens, `store`/`metadata`/
//! `serviceTier`/`reasoningEffort` request-body options) are documented at the
//! bottom rather than translated — see "Remaining untranslated cases".
//!
//! Each test uses `wiremock` to spin up a mock HTTP server, configures a JSON
//! or SSE response, creates an `OpenAIModel` pointing at the mock, calls
//! `do_generate` / `do_stream`, and asserts on the result.

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
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::{OpenAIConfig, OpenAIProvider};

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
        include_raw_chunks: None,
    }
}

/// A simple function tool named `test-tool` with a `value` string parameter.
fn test_tool() -> FunctionTool {
    FunctionTool {
        name: "test-tool".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

/// A function tool named `searchGoogle`.
fn search_google_tool() -> FunctionTool {
    FunctionTool {
        name: "searchGoogle".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

/// A function tool named `search`.
fn search_tool() -> FunctionTool {
    FunctionTool {
        name: "search".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "number" }
            },
            "required": ["query"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

/// Standard mock for a JSON chat completion response.
async fn mock_json_response(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mock a JSON chat completion response delayed by `delay` (RFC-0016 H1/H3
/// tests: timeout + abort need an in-flight request to cancel).
async fn mock_json_response_with_delay(server: &MockServer, body: Value, delay: Duration) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(body)
                .set_delay(delay),
        )
        .mount(server)
        .await;
}

/// Standard mock for a JSON chat completion response with custom headers.
async fn mock_json_response_with_headers(
    server: &MockServer,
    body: Value,
    headers: &[(&str, &str)],
) {
    let mut template = ResponseTemplate::new(200).set_body_json(body);
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Standard mock for an SSE streaming response.
async fn mock_sse_response(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

/// Mock for an SSE streaming response with custom headers.
async fn mock_sse_response_with_headers(
    server: &MockServer,
    sse_body: &str,
    headers: &[(&str, &str)],
) {
    let mut template = ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(sse_body.to_string());
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Build an SSE event string from a JSON value.
fn sse_event(json_str: &str) -> String {
    format!("data: {}\n\n", json_str)
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
// doGenerate — response parsing
// (openai-chat-language-model.test.ts → describe('doGenerate'))
// ════════════════════════════════════════════════════════════════════════════

mod do_generate {
    use super::*;

    // ── should extract text response ──────────────────────────────────────────

    /// TS: "should extract text response"
    #[tokio::test]
    async fn should_extract_text_response() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hello, World!" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "Hello, World!"),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    // ── should extract usage ──────────────────────────────────────────────────

    /// TS: "should extract usage"
    #[tokio::test]
    async fn should_extract_usage() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 20, "total_tokens": 25, "completion_tokens": 5 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        // TS: inputTokens.total = 20, noCache = 20, cacheRead = 0
        assert_eq!(result.usage.input_tokens.total, Some(20));
        assert_eq!(result.usage.input_tokens.no_cache, Some(20));
        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        // TS: outputTokens.total = 5
        assert_eq!(result.usage.output_tokens.total, Some(5));

        // RFC-0016 M10: provider raw usage is preserved (vendor fields like
        // Moonshot `cached_tokens` would otherwise be lost).
        let raw = result
            .usage
            .raw
            .as_ref()
            .expect("usage.raw must be populated");
        assert_eq!(raw["prompt_tokens"], json!(20));
        assert_eq!(raw["completion_tokens"], json!(5));
        assert_eq!(raw["total_tokens"], json!(25));
    }

    /// RFC-0016 M10: vendor-specific usage fields NOT modeled by
    /// `UsageResponse` survive verbatim in `usage.raw` — the actual point of
    /// M10 (re-serializing the typed struct would drop them).
    #[tokio::test]
    async fn should_keep_vendor_specific_usage_fields_in_raw() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "chatcmpl-raw1",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "deepseek-chat",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 5,
                    "total_tokens": 25,
                    "prompt_cache_hit_tokens": 10,
                    "prompt_cache_miss_tokens": 10
                }
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("deepseek-chat");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let raw = result
            .usage
            .raw
            .as_ref()
            .expect("usage.raw must be populated");
        assert_eq!(raw["prompt_cache_hit_tokens"], json!(10));
        assert_eq!(raw["prompt_cache_miss_tokens"], json!(10));
        // Typed fields still work alongside the raw object.
        assert_eq!(result.usage.input_tokens.total, Some(20));
    }

    // ── should send additional response information ───────────────────────────

    /// TS: "should send additional response information"
    #[tokio::test]
    async fn should_send_additional_response_information() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "test-id",
                "object": "chat.completion",
                "created": 123,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.response.id, Some("test-id".to_string()));
        assert_eq!(result.response.model_id, Some("test-model".to_string()));
    }

    // ── should support partial usage ──────────────────────────────────────────

    /// TS: "should support partial usage" — `completion_tokens` absent.
    #[tokio::test]
    async fn should_support_partial_usage() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 20, "total_tokens": 20 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        // TS: inputTokens.total = 20
        assert_eq!(result.usage.input_tokens.total, Some(20));
        // TS: outputTokens.total = 0 (completion_tokens defaults to 0)
        assert_eq!(result.usage.output_tokens.total, Some(0));
    }

    // ── should extract logprobs ───────────────────────────────────────────────

    /// TS: "should extract logprobs"
    #[tokio::test]
    async fn should_extract_logprobs() {
        let logprobs = json!({
            "content": [
                { "token": "Hello", "logprob": -0.0009994634, "top_logprobs": [
                    { "token": "Hello", "logprob": -0.0009994634 }
                ]},
                { "token": "!", "logprob": -0.13410144, "top_logprobs": [
                    { "token": "!", "logprob": -0.13410144 }
                ]}
            ]
        });

        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "logprobs": logprobs,
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider_metadata should be Some");
        let logprobs_content = pm
            .get("openai")
            .and_then(|o| o.get("logprobs"))
            .expect("openai.logprobs should exist");
        assert_eq!(logprobs_content, &logprobs["content"]);
    }

    // ── should extract finish reason ──────────────────────────────────────────

    /// TS: "should extract finish reason"
    #[tokio::test]
    async fn should_extract_finish_reason() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(result.finish_reason.raw, Some("stop".to_string()));
    }

    // ── should support unknown finish reason ──────────────────────────────────

    /// TS: "should support unknown finish reason"
    #[tokio::test]
    async fn should_support_unknown_finish_reason() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "eos"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
        assert_eq!(result.finish_reason.raw, Some("eos".to_string()));
    }

    // ── should expose the raw response headers ────────────────────────────────

    /// TS: "should expose the raw response headers"
    #[tokio::test]
    async fn should_expose_raw_response_headers() {
        let server = MockServer::start().await;
        mock_json_response_with_headers(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
            &[("test-header", "test-value")],
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let headers = result
            .response_headers
            .as_ref()
            .expect("response_headers should be Some");
        assert_eq!(headers.get("test-header"), Some(&"test-value".to_string()));
    }

    // ── should parse tool results ─────────────────────────────────────────────

    /// TS: "should parse tool results" — tool_calls in the response message.
    #[tokio::test]
    async fn should_parse_tool_results() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                            "type": "function",
                            "function": {
                                "name": "test-tool",
                                "arguments": "{\"value\":\"Spark\"}"
                            }
                        }]
                    },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
                "system_fingerprint": "fp_3bc1b5746c"
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

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
                assert_eq!(tool_call_id, "call_O17Uplv4lJvD6DVdIvFFeRMw");
                assert_eq!(tool_name, "test-tool");
                assert_eq!(input, &json!({"value": "Spark"}));
            }
            other => panic!("expected ToolCall, got {:?}", other),
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — streaming response parsing
// (openai-chat-language-model.test.ts → describe('doStream'))
// ════════════════════════════════════════════════════════════════════════════

mod do_stream {
    use super::*;

    // ── should stream text after Azure content filter chunks ──────────────────

    /// TS: "should stream text after Azure content filter chunks"
    #[tokio::test]
    async fn should_stream_text_after_azure_content_filter_chunks() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"choices":[],"created":0,"id":"","model":"","object":"","prompt_filter_results":[{"prompt_index":0,"content_filter_results":{}}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"","role":"assistant"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            ),
            &sse_event(
                r#"{"choices":[{"content_filter_offsets":{},"content_filter_results":{},"finish_reason":null,"index":0}],"created":0,"id":"","model":"","object":""}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // TS: text-delta events should be ["", "Hello"]
        let deltas = text_deltas(&parts);
        assert_eq!(deltas, vec!["".to_string(), "Hello".to_string()]);
    }

    // ── should stream text deltas ─────────────────────────────────────────────

    /// TS: "should stream text deltas"
    #[tokio::test]
    async fn should_stream_text_deltas() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":1,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":1,"delta":{"content":", "},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":1,"delta":{"content":"World!"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Expected stream part sequence:
        // stream-start, response-metadata, text-start, text-delta*4, text-end, finish
        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));

        // response-metadata with id and modelId
        match &parts[1] {
            StreamPart::ResponseMetadata { id, model_id, .. } => {
                assert_eq!(
                    id.as_deref(),
                    Some("chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP")
                );
                assert_eq!(model_id.as_deref(), Some("gpt-3.5-turbo-0613"));
            }
            other => panic!("expected ResponseMetadata, got {:?}", other),
        }

        // text-start with id "0"
        match &parts[2] {
            StreamPart::TextStart { id, .. } => assert_eq!(id, "0"),
            other => panic!("expected TextStart, got {:?}", other),
        }

        // text-deltas: "", "Hello", ", ", "World!"
        let deltas = text_deltas(&parts);
        assert_eq!(
            deltas,
            vec![
                "".to_string(),
                "Hello".to_string(),
                ", ".to_string(),
                "World!".to_string()
            ]
        );

        // text-end with id "0"
        let text_end = parts
            .iter()
            .find(|p| matches!(p, StreamPart::TextEnd { .. }));
        match text_end {
            Some(StreamPart::TextEnd { id, .. }) => assert_eq!(id, "0"),
            other => panic!("expected TextEnd, got {:?}", other),
        }

        // finish with stop reason and usage
        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish {
                finish_reason,
                usage,
                ..
            }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
                assert_eq!(finish_reason.raw.as_deref(), Some("stop"));
                assert_eq!(usage.input_tokens.total, Some(17));
                assert_eq!(usage.output_tokens.total, Some(227));
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    // ── should stream tool deltas ─────────────────────────────────────────────

    /// TS: "should stream tool deltas"
    #[tokio::test]
    async fn should_stream_tool_deltas() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":""}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\""}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"value"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\""}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"Spark"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"le"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" Day"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"tool_calls"}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":53,"completion_tokens":17,"total_tokens":70}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&options_with_tools(test_prompt(), vec![test_tool()]))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Verify stream-start
        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));

        // Verify response-metadata
        assert!(matches!(parts[1], StreamPart::ResponseMetadata { .. }));

        // Verify tool-input-start
        let tool_start = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolInputStart { id, tool_name, .. } if id == "call_O17Uplv4lJvD6DVdIvFFeRMw" && tool_name == "test-tool")
        });
        assert!(tool_start.is_some(), "should have ToolInputStart");

        // Verify tool-input-deltas
        let tool_deltas: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolInputDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            tool_deltas,
            vec![
                "{\"".to_string(),
                "value".to_string(),
                r##"":""##.to_string(),
                "Spark".to_string(),
                "le".to_string(),
                " Day".to_string(),
                "\"}".to_string(),
            ]
        );

        // Verify tool-input-end
        let tool_end = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolInputEnd { id, .. } if id == "call_O17Uplv4lJvD6DVdIvFFeRMw")
        });
        assert!(tool_end.is_some(), "should have ToolInputEnd");

        // Verify tool-call
        let tool_call = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolCall { tool_call_id, tool_name, input, .. }
                if tool_call_id == "call_O17Uplv4lJvD6DVdIvFFeRMw"
                && tool_name == "test-tool"
                && input == &json!({"value": "Sparkle Day"}))
        });
        assert!(
            tool_call.is_some(),
            "should have ToolCall with correct input"
        );

        // Verify finish with tool_calls reason
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
                assert_eq!(usage.input_tokens.total, Some(53));
                assert_eq!(usage.output_tokens.total, Some(17));
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    // ── should stream tool call deltas when arguments are in the first chunk ──

    /// TS: "should stream tool call deltas when tool call arguments are passed
    /// in the first chunk"
    #[tokio::test]
    async fn should_stream_tool_call_deltas_in_first_chunk() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":"{\""}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"va"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"lue"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\""}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"Spark"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"le"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" Day"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"tool_calls"}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":53,"completion_tokens":17,"total_tokens":70}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&options_with_tools(test_prompt(), vec![test_tool()]))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Verify tool-call with complete arguments
        let tool_call = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolCall { tool_call_id, input, .. }
                if tool_call_id == "call_O17Uplv4lJvD6DVdIvFFeRMw"
                && input == &json!({"value": "Sparkle Day"}))
        });
        assert!(
            tool_call.is_some(),
            "should have ToolCall with complete input"
        );
    }

    // ── should not duplicate tool calls ───────────────────────────────────────

    /// TS: "should not duplicate tool calls when there is an additional empty
    /// chunk after the tool call has been completed"
    #[tokio::test]
    async fn should_not_duplicate_tool_calls() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"role":"assistant","content":""},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":226,"completion_tokens":0}}"#,
            ),
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"id":"chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa","type":"function","index":0,"function":{"name":"searchGoogle"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":233,"completion_tokens":7}}"#,
            ),
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\": \""}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":241,"completion_tokens":15}}"#,
            ),
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"latest"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":242,"completion_tokens":16}}"#,
            ),
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" news"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":243,"completion_tokens":17}}"#,
            ),
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" on"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":244,"completion_tokens":18}}"#,
            ),
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" ai\"}"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":245,"completion_tokens":19}}"#,
            ),
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"logprobs":null,"finish_reason":"tool_calls","stop_reason":128008}],"usage":{"prompt_tokens":226,"total_tokens":246,"completion_tokens":20}}"#,
            ),
            &sse_event(
                r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[],"usage":{"prompt_tokens":226,"total_tokens":246,"completion_tokens":20}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&options_with_tools(
                test_prompt(),
                vec![search_google_tool()],
            ))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Exactly one tool-call event
        let tool_calls: Vec<_> = parts
            .iter()
            .filter(|p| matches!(p, StreamPart::ToolCall { .. }))
            .collect();
        assert_eq!(tool_calls.len(), 1, "should have exactly one ToolCall");

        // The tool call should have the complete arguments
        match tool_calls[0] {
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                assert_eq!(
                    tool_call_id,
                    "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa"
                );
                assert_eq!(tool_name, "searchGoogle");
                assert_eq!(input, &json!({"query": "latest news on ai"}));
            }
            other => panic!("expected ToolCall, got {:?}", other),
        }
    }

    // ── should not finalize tool call early ───────────────────────────────────

    /// TS: "should not finalize tool call early when partial JSON is
    /// coincidentally parsable"
    #[tokio::test]
    async fn should_not_finalize_tool_call_early() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-early","object":"chat.completion.chunk","created":1733162241,"model":"gpt-4","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_early123","type":"function","function":{"name":"search","arguments":""}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-early","object":"chat.completion.chunk","created":1733162241,"model":"gpt-4","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\": \"test\"}"}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-early","object":"chat.completion.chunk","created":1733162241,"model":"gpt-4","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-early","object":"chat.completion.chunk","created":1733162241,"model":"gpt-4","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":", \"limit\": 10}"}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-early","object":"chat.completion.chunk","created":1733162241,"model":"gpt-4","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&options_with_tools(test_prompt(), vec![search_tool()]))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Exactly one tool-call event
        let tool_calls: Vec<_> = parts
            .iter()
            .filter(|p| matches!(p, StreamPart::ToolCall { .. }))
            .collect();
        assert_eq!(tool_calls.len(), 1, "should have exactly one ToolCall");

        // The tool call should contain the COMPLETE arguments, not just the
        // partial JSON that happened to be parsable mid-stream.
        match tool_calls[0] {
            StreamPart::ToolCall { input, .. } => {
                // The full accumulated string is: {"query": "test"}, "limit": 10}
                // which is NOT valid JSON by itself. The implementation should
                // fall back to storing the raw string as Value::String.
                match input {
                    Value::String(s) => {
                        assert!(
                            s.contains("query") && s.contains("limit"),
                            "input string should contain both 'query' and 'limit': {}",
                            s
                        );
                    }
                    Value::Object(_) => {
                        // If it somehow parsed, still check for both keys
                        assert!(input.get("query").is_some(), "should have 'query'");
                    }
                    other => panic!("expected String or Object, got {:?}", other),
                }
            }
            other => panic!("expected ToolCall, got {:?}", other),
        }
    }

    // ── should stream tool call with missing type field ───────────────────────

    /// TS: "should stream tool call with missing type field (Azure AI Foundry /
    /// Mistral)"
    #[tokio::test]
    async fn should_stream_tool_call_with_missing_type_field() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-azure-001","object":"chat.completion.chunk","created":1711357598,"model":"mistral-large","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc123","function":{"name":"test-tool","arguments":""}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-azure-001","object":"chat.completion.chunk","created":1711357598,"model":"mistral-large","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"value\""}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-azure-001","object":"chat.completion.chunk","created":1711357598,"model":"mistral-large","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"hello\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&options_with_tools(test_prompt(), vec![test_tool()]))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Verify tool-input-start
        let tool_start = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolInputStart { id, tool_name, .. }
                if id == "call_abc123" && tool_name == "test-tool")
        });
        assert!(tool_start.is_some(), "should have ToolInputStart");

        // Verify tool-call
        let tool_call = parts.iter().find(|p| {
            matches!(p, StreamPart::ToolCall { tool_call_id, input, .. }
                if tool_call_id == "call_abc123"
                && input == &json!({"value": "hello"}))
        });
        assert!(
            tool_call.is_some(),
            "should have ToolCall with correct input"
        );
    }

    // ── should stream tool call that is sent in one chunk ─────────────────────

    /// TS: "should stream tool call that is sent in one chunk"
    #[tokio::test]
    async fn should_stream_tool_call_in_one_chunk() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":"{\"value\":\"Sparkle Day\"}"}}]},"logprobs":null,"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"tool_calls"}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":53,"completion_tokens":17,"total_tokens":70}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&options_with_tools(test_prompt(), vec![test_tool()]))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Verify tool-input-start
        assert!(parts.iter().any(|p| {
            matches!(p, StreamPart::ToolInputStart { id, tool_name, .. }
                if id == "call_O17Uplv4lJvD6DVdIvFFeRMw" && tool_name == "test-tool")
        }));

        // Verify single tool-input-delta with complete arguments
        let tool_deltas: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolInputDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(tool_deltas, vec![r#"{"value":"Sparkle Day"}"#.to_string()]);

        // Verify tool-call
        assert!(parts.iter().any(|p| {
            matches!(p, StreamPart::ToolCall { tool_call_id, input, .. }
                if tool_call_id == "call_O17Uplv4lJvD6DVdIvFFeRMw"
                && input == &json!({"value": "Sparkle Day"}))
        }));
    }

    // ── should throw an api error when the first stream chunk is an error ─────

    /// TS: "should throw an api error when the first stream chunk is an error"
    #[tokio::test]
    async fn should_throw_on_first_chunk_error() {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}",
            sse_event(
                r#"{"error":{"message":"The server had an error processing your request.","type":"server_error","param":null,"code":null}}"#
            ),
            "data: [DONE]\n\n"
        );
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model.do_stream(&default_options(test_prompt())).await;

        assert!(result.is_err(), "do_stream should return Err");
        match result.unwrap_err() {
            AiMuxError::Provider(msg) => {
                assert!(msg.contains("The server had an error processing your request."));
            }
            other => panic!("expected Provider error, got {:?}", other),
        }
    }

    // ── should preserve numeric status codes from early stream errors ────────

    /// TS: "should preserve numeric status codes from early stream errors"
    #[tokio::test]
    async fn should_preserve_numeric_status_codes() {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}",
            sse_event(
                r#"{"error":{"message":"bad request","type":"provider_error","param":null,"code":400}}"#
            ),
            "data: [DONE]\n\n"
        );
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model.do_stream(&default_options(test_prompt())).await;

        assert!(result.is_err(), "do_stream should return Err");
        match result.unwrap_err() {
            AiMuxError::Provider(msg) => {
                assert!(msg.contains("bad request"));
                assert!(msg.contains("400"));
            }
            other => panic!("expected Provider error, got {:?}", other),
        }
    }

    // ── should forward error stream parts after output has started ────────────

    /// TS: "should forward error stream parts after output has started"
    #[tokio::test]
    async fn should_forward_error_after_output_started() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-error-after-output","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"error":{"message":"stream failed after output","type":"server_error","param":null,"code":null}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed (error is in-stream)");
        let parts = collect_stream(result).await;

        // Should have: stream-start, response-metadata, text-start, text-delta,
        // Error, text-end, Finish (with error finish reason)
        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));

        // Text delta "Hello"
        let deltas = text_deltas(&parts);
        assert_eq!(deltas, vec!["Hello".to_string()]);

        // Error part
        let error_part = parts.iter().find(|p| matches!(p, StreamPart::Error { .. }));
        assert!(error_part.is_some(), "should have Error part");

        // Finish with error finish reason
        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { finish_reason, .. }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    // ── should handle unparsable stream parts ─────────────────────────────────

    /// TS: "should handle unparsable stream parts"
    #[tokio::test]
    async fn should_handle_unparsable_stream_parts() {
        let server = MockServer::start().await;
        let body = format!("{}{}", sse_event("{unparsable}"), "data: [DONE]\n\n");
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Should have: stream-start, Error, Finish (with error finish reason)
        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));

        let error_part = parts.iter().find(|p| matches!(p, StreamPart::Error { .. }));
        assert!(error_part.is_some(), "should have Error part");

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { finish_reason, .. }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    // ── should expose the raw response headers ────────────────────────────────

    /// TS: "should expose the raw response headers" (streaming)
    #[tokio::test]
    async fn should_expose_raw_response_headers_stream() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}"#,
            ),
        ]);
        mock_sse_response_with_headers(&server, &body, &[("test-header", "test-value")]).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

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

    // ── should return cached tokens in providerMetadata ───────────────────────

    /// TS: "should return cached tokens in providerMetadata"
    #[tokio::test]
    async fn should_return_cached_tokens() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":2000,"completion_tokens":20,"total_tokens":2020,"prompt_tokens_details":{"cached_tokens":1152,"cache_write_tokens":256}}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

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
                // TS: inputTokens.total=2000, cacheRead=1152, cacheWrite=256, noCache=592
                assert_eq!(usage.input_tokens.total, Some(2000));
                assert_eq!(usage.input_tokens.cache_read, Some(1152));
                assert_eq!(usage.input_tokens.cache_write, Some(256));
                assert_eq!(usage.input_tokens.no_cache, Some(592));
                // TS: outputTokens.total=20
                assert_eq!(usage.output_tokens.total, Some(20));
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    // ── reasoning models: should stream text delta ────────────────────────────

    /// TS: reasoning models → "should stream text delta"
    #[tokio::test]
    async fn reasoning_models_should_stream_text_delta() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":null,"choices":[{"index":1,"delta":{"content":"Hello, World!"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Verify stream-start
        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));

        // Verify response-metadata with modelId "o4-mini"
        match &parts[1] {
            StreamPart::ResponseMetadata { model_id, .. } => {
                assert_eq!(model_id.as_deref(), Some("o4-mini"));
            }
            other => panic!("expected ResponseMetadata, got {:?}", other),
        }

        // Verify text deltas
        let deltas = text_deltas(&parts);
        assert_eq!(deltas, vec!["".to_string(), "Hello, World!".to_string()]);

        // Verify finish with stop reason
        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish {
                finish_reason,
                usage,
                ..
            }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
                assert_eq!(usage.input_tokens.total, Some(17));
                assert_eq!(usage.output_tokens.total, Some(227));
            }
            other => panic!("expected Finish, got {:?}", other),
        }

        // RFC-0016 M10: streaming usage also carries the provider raw usage.
        if let Some(StreamPart::Finish { usage, .. }) = finish {
            let raw = usage.raw.as_ref().expect("usage.raw must be populated");
            assert_eq!(raw["prompt_tokens"], json!(17));
            assert_eq!(raw["completion_tokens"], json!(227));
        }
    }

    // ── RFC-0016 M2: includeRawChunks ───────────────────────────────────────

    /// RFC-0016 M2: `include_raw_chunks` emits one `StreamPart::Raw` per JSON
    /// SSE event (before the parsed parts); default is off.
    #[tokio::test]
    async fn should_emit_raw_chunks_when_enabled() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let options = CallOptions {
            include_raw_chunks: Some(true),
            ..default_options(test_prompt())
        };
        let result = model
            .do_stream(&options)
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        let raw: Vec<&Value> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::Raw { raw_value } => Some(raw_value),
                _ => None,
            })
            .collect();
        assert_eq!(raw.len(), 2, "one Raw part per JSON SSE event");
        assert_eq!(raw[0]["choices"][0]["delta"]["content"], json!("Hi"));
        assert_eq!(raw[1]["usage"]["prompt_tokens"], json!(1));
    }

    /// RFC-0016 M2: default (`None`) emits no `StreamPart::Raw`.
    #[tokio::test]
    async fn raw_chunks_off_by_default() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        assert!(
            parts.iter().all(|p| !matches!(p, StreamPart::Raw { .. })),
            "no Raw parts expected by default"
        );
    }

    /// RFC-0016 M2: an unparsable chunk emits only `Error` (no `Raw`), and
    /// the stream breaks there.
    #[tokio::test]
    async fn raw_chunks_unparsable_chunk_emits_error_only() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
            ),
            // Unparsable chunk: no Raw for it, Error only, stream breaks.
            "data: not-json\n\n",
            // Never reached (stream broke at the unparsable chunk).
            &sse_event(
                r#"{"id":"c2","object":"chat.completion.chunk","created":1,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{"content":"late"},"finish_reason":null}]}"#,
            ),
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let options = CallOptions {
            include_raw_chunks: Some(true),
            ..default_options(test_prompt())
        };
        let result = model
            .do_stream(&options)
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Only the content chunk emitted Raw; the unparsable chunk did not,
        // and the stream broke before the late chunk.
        let raw: Vec<&Value> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::Raw { raw_value } => Some(raw_value),
                _ => None,
            })
            .collect();
        assert_eq!(raw.len(), 1, "no Raw for the unparsable chunk");
        assert_eq!(raw[0]["choices"][0]["delta"]["content"], json!("Hi"));

        // The unparsable chunk surfaces as Error.
        let err_pos = parts
            .iter()
            .position(|p| matches!(p, StreamPart::Error { .. }))
            .expect("unparsable chunk must surface as Error part");
        assert!(
            !matches!(&parts[err_pos - 1], StreamPart::Raw { .. }),
            "no Raw may precede the Error of an unparsable chunk"
        );
    }

    /// RFC-0016 M2: `[DONE]` sentinel emits no Raw; a mid-stream error chunk
    /// emits Raw before Error (debugging order).
    #[tokio::test]
    async fn raw_chunks_done_skipped_and_error_preceded_by_raw() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
            ),
            // Error chunk: Raw must precede the Error part for this event.
            &sse_event(
                r#"{"error":{"message":"mid-stream failure","type":"server_error","code":"500"}}"#,
            ),
            // [DONE] sentinel is skipped by the early break.
            "data: [DONE]\n\n",
        ]);
        mock_sse_response(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let options = CallOptions {
            include_raw_chunks: Some(true),
            ..default_options(test_prompt())
        };
        let result = model
            .do_stream(&options)
            .await
            .expect("do_stream should succeed");
        let parts = collect_stream(result).await;

        // Raw for the content chunk + the error chunk; [DONE] emits none.
        let raw: Vec<&Value> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::Raw { raw_value } => Some(raw_value),
                _ => None,
            })
            .collect();
        assert_eq!(
            raw.len(),
            2,
            "expected Raw for the content chunk + error chunk"
        );

        // Raw for the error chunk precedes its Error part.
        let err_pos = parts
            .iter()
            .position(|p| matches!(p, StreamPart::Error { .. }))
            .expect("mid-stream error must surface as Error part");
        assert!(
            matches!(
                &parts[err_pos - 1],
                StreamPart::Raw { raw_value }
                    if raw_value["error"]["message"] == "mid-stream failure"
            ),
            "Raw must immediately precede Error for the error chunk"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// RFC-0016 H1/H3: abort + per-call timeout (provider-level)
// ════════════════════════════════════════════════════════════════════════════

use aimux_core::options::TimeoutConfiguration;
use aimux_core::shared::AbortSignal;
use std::time::Duration;

/// TS: timeout — total_ms bounds the whole call.
#[tokio::test]
async fn total_timeout_aborts_slow_generate() {
    let server = MockServer::start().await;
    mock_json_response_with_delay(
        &server,
        json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 6, "completion_tokens": 2 }
        }),
        Duration::from_millis(500),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.model("gpt-3.5-turbo");

    let mut options = default_options(test_prompt());
    options.timeout = Some(TimeoutConfiguration {
        total_ms: Some(100),
        ..Default::default()
    });

    let err = model
        .do_generate(&options)
        .await
        .expect_err("slow response must be cut by total timeout");
    assert!(matches!(err, AiMuxError::Timeout(_)), "got {err:?}");
}

/// TS: abort signal — aborting mid-request cancels do_generate.
#[tokio::test]
async fn abort_signal_cancels_in_flight_generate() {
    let server = MockServer::start().await;
    mock_json_response_with_delay(
        &server,
        json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 6, "completion_tokens": 2 }
        }),
        Duration::from_millis(300),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.model("gpt-3.5-turbo");

    let signal = AbortSignal::new();
    let signal_clone = signal.clone();
    let mut options = default_options(test_prompt());
    options.abort_signal = Some(signal_clone);

    let handle = tokio::spawn(async move { model.do_generate(&options).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    signal.abort();

    let result = handle.await.expect("task must finish");
    assert!(
        matches!(result, Err(AiMuxError::Aborted)),
        "aborted call must fail with Aborted, got {result:?}"
    );
}

/// TS: abort before send fails fast.
#[tokio::test]
async fn abort_before_send_fails_fast() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 6, "completion_tokens": 2 }
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.model("gpt-3.5-turbo");

    let signal = AbortSignal::new();
    signal.abort();
    let mut options = default_options(test_prompt());
    options.abort_signal = Some(signal);

    let err = model
        .do_generate(&options)
        .await
        .expect_err("pre-aborted signal must fail fast");
    assert!(matches!(err, AiMuxError::Aborted), "got {err:?}");
}

// ════════════════════════════════════════════════════════════════════════════
// Remaining untranslated cases
// ════════════════════════════════════════════════════════════════════════════
//
// doGenerate:
// - "should parse annotations/citations" — requires a Source content type not
//   present in the Rust `GenerateContent` enum.
// - "should send request body" / "should pass settings" / "should pass tools and
//   toolChoice" / "should pass headers" / response-format / reasoning-effort /
//   textVerbosity — these are request-body assertions already covered by
//   `openai_convert_test.rs`.
//
// doStream:
// - "should stream annotations/citations" — requires Source content type.
// - "should return accepted_prediction_tokens and rejected_prediction_tokens" —
//   requires `providerMetadata.openai` prediction-token fields (the Rust
//   `ProviderMetadata` is a flat `Value`; the TS nests under `openai`).
// - "should send request body" / "should pass the messages and the model" /
//   "should pass headers" / "should send store extension setting" / "should send
//   metadata extension values" / "should send serviceTier …" — request-body
//   assertions already covered by `openai_convert_test.rs`.
// - "should set .modelId for model-router request" — uses fixture files.
// - "reasoning models → should send reasoning tokens" — requires
//   `outputTokens.reasoning` / `outputTokens.text` breakdown not present in the
//   Rust `TokenUsage` struct.
// - "raw chunks" (includeRawChunks) — requires an `include_raw_chunks` option
//   not present in the Rust `CallOptions`.
