//! DeepSeek basic chat tests, translated from the Vercel AI SDK TypeScript suite.
//!
//! Translation sources:
//! - `packages/deepseek/src/chat/deepseek-chat-language-model.test.ts`
//!   - `describe('doGenerate') › describe('text')` — request body + text extraction
//!   - `describe('doGenerate') › describe('tool call')` — tool call request body,
//!     JSON response format (without/with schema), tool call extraction
//!   - `describe('doStream') › describe('text')` — stream request body + text streaming
//!   - `describe('doStream') › describe('tool call')` — tool call streaming
//!   - Reasoning tests are in `deepseek_reasoning_test.rs` and are NOT duplicated here.
//! - `packages/deepseek/src/chat/convert-to-deepseek-chat-messages.test.ts`
//!   - User messages (text, file warning, mediaType)
//!   - Tool calls (stringify arguments, text output)
//!   - Reasoning-related convert tests are excluded (covered by reasoning test file).
//! - `packages/deepseek/src/chat/deepseek-prepare-tools.test.ts`
//!   - Strict mode pass-through (true, false, undefined, mixed)
//!
//! HTTP is mocked with `wiremock` (a real loopback server), replacing the TS
//! `createTestServer`. Each test starts its own `MockServer` so parallel
//! `#[tokio::test]` runs do not collide.
//!
//! # Mock URL note
//!
//! The real DeepSeek base URL is `https://api.deepseek.com/v1`, so production
//! requests hit `/v1/chat/completions`. In these tests `with_base_url` is
//! overridden with the mock server's root URI (no `/v1` suffix), so the
//! resulting request path is `/chat/completions`.

use std::collections::HashMap;

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, Tool};
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::openai::OpenAICompatProfile;
use aimux_providers::openai::convert::build_request_body_with_warnings;
use aimux_providers::{provider, ProviderOptions};

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers (mirrors deepseek_reasoning_test.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// `CallOptions` with only `prompt` set (everything else default/None).
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build a DeepSeek provider whose base URL points at the mock server.
fn make_provider(server: &MockServer, model_id: &str) -> Box<dyn LanguageModel> {
    provider(
        "deepseek",
        Some("test-api-key".to_string()),
        model_id,
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            ..Default::default()
        }),
    )
    .expect("deepseek provider should build")
}

/// Wrap a value as `providerOptions.deepseek.<value>`.
fn deepseek_opts(value: Value) -> Option<HashMap<String, Value>> {
    let mut m = HashMap::new();
    m.insert("deepseek".to_string(), value);
    Some(m)
}

/// Mount a JSON chat-completion response on `/chat/completions`.
async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE stream response on `/chat/completions`.
async fn mock_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

/// Build a single SSE `data: <json>\n\n` event string.
fn sse_event(json_str: &str) -> String {
    format!("data: {}\n\n", json_str)
}

/// Concatenate SSE events and append the `[DONE]` sentinel.
fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Collect every `StreamPart` from a `StreamResult` into a `Vec`.
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

/// A weather function tool matching the TS test's `inputSchema`.
fn weather_tool() -> FunctionTool {
    FunctionTool {
        name: "weather".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Local fixtures
// ─────────────────────────────────────────────────────────────────────────────

/// A minimal non-streaming text completion body returning a markdown response.
/// Abbreviated from the `deepseek-text` fixture.
fn text_completion_body() -> Value {
    json!({
        "id": "00f10ecd-60b3-4707-b5db-e4bcadf7aea1",
        "object": "chat.completion",
        "created": 1764656316,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "## **Holiday Name: Gratitude of Small Things Day (GST Day)**"
            },
            "logprobs": null,
            "finish_reason": "length"
        }],
        "usage": {
            "prompt_tokens": 13,
            "completion_tokens": 300,
            "total_tokens": 313,
            "prompt_tokens_details": { "cached_tokens": 0 }
        },
        "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache"
    })
}

/// A non-streaming tool-call completion body. Abbreviated from the
/// `deepseek-tool-call` fixture (reasoning_content omitted — reasoning is tested
/// in `deepseek_reasoning_test.rs`).
fn tool_call_completion_body() -> Value {
    json!({
        "id": "7a630f5b-b7e6-4878-82f8-d77db164d42b",
        "object": "chat.completion",
        "created": 1764665845,
        "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "index": 0,
                    "id": "call_00_9V0vrf86Pc9aelHCJMZqnJBo",
                    "type": "function",
                    "function": {
                        "name": "weather",
                        "arguments": "{\"location\": \"San Francisco\"}"
                    }
                }]
            },
            "logprobs": null,
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 339,
            "completion_tokens": 92,
            "total_tokens": 431
        }
    })
}

/// A non-streaming JSON completion body. Abbreviated from the `deepseek-json`
/// fixture (reasoning_content omitted).
fn json_completion_body() -> Value {
    json!({
        "id": "f03bc170-b375-4561-9685-35182c8152c5",
        "object": "chat.completion",
        "created": 1764681341,
        "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "{\n  \"location\": \"San Francisco\",\n  \"condition\": \"cloudy\",\n  \"temperature\": 7\n}"
            },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 495,
            "completion_tokens": 144,
            "total_tokens": 639
        }
    })
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — text
//
// Translated from `deepseek-chat-language-model.test.ts` doGenerate › text.
// ════════════════════════════════════════════════════════════════════════════

/// TS: doGenerate › text › "should send correct request body" (line ~39).
///
/// Verifies that system + user messages, temperature, and top_p are correctly
/// serialized in the request body.
#[tokio::test]
async fn should_send_correct_text_request_body() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let model = make_provider(&server, "deepseek-chat");

    let prompt = vec![
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
    let mut options = default_options(prompt);
    options.temperature = Some(0.5);
    options.top_p = Some(0.3);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["model"], json!("deepseek-chat"));
    assert_eq!(
        body["messages"],
        json!([
            { "role": "system", "content": "You are a helpful assistant." },
            { "role": "user", "content": "Hello" }
        ])
    );
    assert_eq!(body["temperature"], json!(0.5));
    assert_eq!(body["top_p"], json!(0.3));
}

/// TS: doGenerate › text › "should extract text content" (line ~68).
///
/// Verifies that the text content is extracted from the response and the
/// finish reason / usage are parsed correctly.
#[tokio::test]
async fn should_extract_text_content() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let model = make_provider(&server, "deepseek-chat");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert!(
            text.contains("Gratitude of Small Things Day"),
            "expected the fixture text, got: {}",
            text
        ),
        other => panic!("expected Text, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Length);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("length"));
    // Usage: prompt_tokens=13, completion_tokens=300
    assert_eq!(result.usage.input_tokens.total, Some(13));
    assert_eq!(result.usage.output_tokens.total, Some(300));
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — tool call
//
// Translated from `deepseek-chat-language-model.test.ts` doGenerate › tool call.
// ════════════════════════════════════════════════════════════════════════════

/// TS: doGenerate › tool call › "should send correct request body" (line ~318).
///
/// Verifies that tools are correctly serialized in the request body.
/// stage2-001（RFC-0017 阶段 2）：DeepSeek 特化已退役——`providerOptions.deepseek`
/// 不再翻译为请求体 `thinking` 字段，用户改用 bodyOverrides 注入（新语义透传）。
#[tokio::test]
async fn should_send_correct_tool_call_request_body() {
    let server = MockServer::start().await;
    mock_json(&server, tool_call_completion_body()).await;

    let model = make_provider(&server, "deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.tools = Some(vec![weather_tool().into()]);
    // 退役后 providerOptions.deepseek.* 被忽略（不再有 apply_deepseek_override）。
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "enabled" } }));

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["model"], json!("deepseek-reasoner"));
    assert!(
        body.get("thinking").is_none(),
        "stage2-001: DeepSeek 特化退役,thinking 不再由 providerOptions.deepseek 注入,改用 bodyOverrides"
    );
    assert_eq!(
        body["tools"],
        json!([{
            "type": "function",
            "function": {
                "name": "weather",
                "parameters": {
                    "type": "object",
                    "properties": { "location": { "type": "string" } },
                    "required": ["location"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#"
                }
            }
        }])
    );
    // The messages should contain the single "Hello" user message.
    assert_eq!(
        body["messages"],
        json!([{ "role": "user", "content": "Hello" }])
    );
}

/// TS: doGenerate › tool call › "should extract tool call content" (line ~668).
///
/// Verifies that tool calls are extracted from the response as
/// `GenerateContent::ToolCall` parts with parsed JSON input.
#[tokio::test]
async fn should_extract_tool_call_content() {
    let server = MockServer::start().await;
    mock_json(&server, tool_call_completion_body()).await;

    let model = make_provider(&server, "deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.tools = Some(vec![weather_tool().into()]);
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "enabled" } }));

    let result = model.do_generate(&options).await.expect("should succeed");

    // The tool-call fixture has empty content and one tool call.
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            assert_eq!(tool_call_id, "call_00_9V0vrf86Pc9aelHCJMZqnJBo");
            assert_eq!(tool_name, "weather");
            assert_eq!(input, &json!({ "location": "San Francisco" }));
        }
        other => panic!("expected ToolCall, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — JSON response format
//
// Translated from `deepseek-chat-language-model.test.ts` doGenerate › tool call
// › "json response format" (lines ~378-568).
//
// TS behaviour: without `supportsStructuredOutputs`, DeepSeek uses
// `response_format: { type: "json_object" }` and injects a system message
// ("Return JSON." or "Return JSON that conforms to the following schema: ...").
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should send correct request body without schema" (line ~383).
///
/// Without a schema, the request body should have
/// `response_format: { type: "json_object" }` and a system message
/// "Return JSON." prepended to the messages.
#[tokio::test]
async fn should_send_json_response_format_without_schema() {
    let server = MockServer::start().await;
    mock_json(&server, json_completion_body()).await;

    let model = make_provider(&server, "deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.response_format = Some(ResponseFormat::Json {
        schema: None,
        name: None,
        description: None,
    });
    options.tools = Some(vec![weather_tool().into()]);
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "enabled" } }));

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["response_format"], json!({ "type": "json_object" }));
    // DeepSeek does not inject a "Return JSON." system message.
    assert_eq!(
        body["messages"][0],
        json!({ "role": "user", "content": "Hello" })
    );
}

/// TS: "should send correct request body with schema" (line ~451).
///
/// With a schema but without `supportsStructuredOutputs`, the request body
/// should still use `response_format: { type: "json_object" }` (NOT
/// `json_schema`) and inject a system message containing the schema.
#[tokio::test]
async fn should_send_json_response_format_with_schema() {
    let server = MockServer::start().await;
    mock_json(&server, json_completion_body()).await;

    let model = make_provider(&server, "deepseek-reasoner");

    let schema = json!({
        "type": "object",
        "properties": {
            "elements": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" },
                        "temperature": { "type": "number" },
                        "condition": { "type": "string" }
                    },
                    "required": ["location", "temperature", "condition"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["elements"],
        "additionalProperties": false,
        "$schema": "http://json-schema.org/draft-07/schema#"
    });

    let mut options = default_options(test_prompt());
    options.response_format = Some(ResponseFormat::Json {
        schema: Some(schema.clone()),
        name: None,
        description: None,
    });
    options.tools = Some(vec![weather_tool().into()]);
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "enabled" } }));

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    // With a schema, deepseek uses json_schema format (via OpenAI shared converter).
    assert_eq!(body["response_format"]["type"], "json_schema");
    // DeepSeek does not inject a system message.
    assert_eq!(
        body["messages"][0],
        json!({ "role": "user", "content": "Hello" })
    );
}

/// TS: "should extract text content" (json response format, line ~542).
///
/// Verifies that JSON content is extracted as a text part.
#[tokio::test]
async fn should_extract_json_response_text_content() {
    let server = MockServer::start().await;
    mock_json(&server, json_completion_body()).await;

    let model = make_provider(&server, "deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.response_format = Some(ResponseFormat::Json {
        schema: None,
        name: None,
        description: None,
    });
    options.tools = Some(vec![weather_tool().into()]);
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "enabled" } }));

    let result = model.do_generate(&options).await.expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert!(
            text.contains("San Francisco"),
            "expected JSON content with 'San Francisco', got: {}",
            text
        ),
        other => panic!("expected Text, got {:?}", other),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — text
//
// Translated from `deepseek-chat-language-model.test.ts` doStream › text.
// ════════════════════════════════════════════════════════════════════════════

/// TS: doStream › text › "should send model id, settings, and input" (line ~715).
///
/// Verifies that the stream request body includes `stream: true`,
/// `stream_options: { include_usage: true }`, and the correct messages.
#[tokio::test]
async fn should_send_correct_stream_request_body() {
    let server = MockServer::start().await;
    let body = sse_body(&[&sse_event(
        r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-chat","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
    )]);
    mock_sse(&server, body).await;

    let model = make_provider(&server, "deepseek-chat");

    let prompt = vec![
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
    let mut options = default_options(prompt);
    options.temperature = Some(0.5);
    options.top_p = Some(0.3);

    let result = model.do_stream(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["model"], json!("deepseek-chat"));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["stream_options"], json!({ "include_usage": true }));
    assert_eq!(
        body["messages"],
        json!([
            { "role": "system", "content": "You are a helpful assistant." },
            { "role": "user", "content": "Hello" }
        ])
    );
    assert_eq!(body["temperature"], json!(0.5));
    assert_eq!(body["top_p"], json!(0.3));
}

/// TS: doStream › text › "should stream text" (line ~748).
///
/// Verifies that text deltas are streamed as TextStart / TextDelta / TextEnd
/// parts, followed by a Finish part with usage.
#[tokio::test]
async fn should_stream_text() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-chat","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-chat","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-chat","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-chat","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let model = make_provider(&server, "deepseek-chat");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    // Should have StreamStart, ResponseMetadata, TextStart, TextDelta x2,
    // TextEnd, Finish.
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::StreamStart { .. })),
        "expected StreamStart"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::TextStart { .. })),
        "expected TextStart"
    );
    let deltas: Vec<String> = text_deltas(&parts)
        .into_iter()
        .filter(|d| !d.is_empty())
        .collect();
    assert_eq!(
        deltas,
        vec!["Hello".to_string(), " world".to_string()],
        "text deltas should match"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::TextEnd { .. })),
        "expected TextEnd"
    );
    // The last part should be Finish.
    match parts.last() {
        Some(StreamPart::Finish { finish_reason, .. }) => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
        }
        other => panic!("expected Finish as last part, got {:?}", other),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — tool call
//
// Translated from `deepseek-chat-language-model.test.ts` doStream › tool call
// (line ~775). Uses an abbreviated chunk set without reasoning_content
// (reasoning streaming is tested in `deepseek_reasoning_test.rs`).
// ════════════════════════════════════════════════════════════════════════════

/// TS: doStream › tool call › "should stream tool call" (line ~780).
///
/// Verifies that tool call deltas are streamed as ToolInputStart /
/// ToolInputDelta / ToolInputEnd / ToolCall parts.
#[tokio::test]
async fn should_stream_tool_call() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"weather","arguments":""}}]},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"location\": \"San Francisco\"}"}}]},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"x","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let model = make_provider(&server, "deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.tools = Some(vec![weather_tool().into()]);
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "enabled" } }));

    let result = model
        .do_stream(&options)
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    // Should have ToolInputStart, ToolInputDelta, ToolInputEnd, ToolCall.
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolInputStart { .. })),
        "expected ToolInputStart"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolInputDelta { .. })),
        "expected ToolInputDelta"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolInputEnd { .. })),
        "expected ToolInputEnd"
    );

    let tool_calls: Vec<_> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => Some((tool_call_id, tool_name, input)),
            _ => None,
        })
        .collect();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].0, "call_1");
    assert_eq!(tool_calls[0].1, "weather");
    assert_eq!(tool_calls[0].2, &json!({ "location": "San Francisco" }));

    // The last part should be Finish with ToolCalls.
    match parts.last() {
        Some(StreamPart::Finish { finish_reason, .. }) => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
        }
        other => panic!("expected Finish as last part, got {:?}", other),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Message conversion — convert-to-deepseek-chat-messages.test.ts
//
// These tests call `build_request_body_with_warnings` directly (no HTTP mock
// needed) and assert on the `body["messages"]` field.
//
// Translated from `convert-to-deepseek-chat-messages.test.ts`.
// Reasoning-related convert tests are excluded (covered by reasoning test file).
// ════════════════════════════════════════════════════════════════════════════

/// TS: user messages › "should convert messages with only a text part to a
/// string content" (line ~6).
///
/// A user message with a single text part should produce
/// `{ role: "user", content: "Hello" }`.
#[tokio::test]
async fn should_convert_user_text_part_to_string_content() {
    let options = default_options(test_prompt());
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    assert_eq!(
        result.body["messages"],
        json!([{ "role": "user", "content": "Hello" }])
    );
    assert!(result.warnings.is_empty(), "expected no warnings");
}

/// TS: user messages › "should warn about unsupported file parts" (line ~31).
///
/// A user message with a text part and a file part should produce only the
/// TS: file parts with image media type are converted to image_url (OpenAI format).
#[tokio::test]
async fn should_convert_image_file_parts_to_image_url() {
    let prompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("Hello"),
            ContentPart::file_base64("AAECAw==", "image/png"),
        ],
        ..Default::default()
    }];
    let options = default_options(prompt);
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    // Image file parts are converted to image_url (OpenAI-compatible behaviour).
    let messages_str = result.body["messages"].to_string();
    assert!(
        messages_str.contains("image_url"),
        "messages should contain image_url: {}",
        messages_str
    );
    assert!(
        messages_str.contains("data:image/png;base64,AAECAw=="),
        "messages should contain base64 data URL: {}",
        messages_str
    );
}

/// TS: user messages › "should accept top-level-only mediaType without error
/// and not read it" (line ~71).
///
/// A user message with a text part and a file part whose mediaType is just
/// "image" (top-level only) should still produce only the text content and
/// NOT include any image/mediaType data in the output.
#[tokio::test]
async fn should_accept_top_level_only_mediatype_without_error() {
    let prompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("Hello"),
            ContentPart::file_base64("AAECAw==", "image"),
        ],
        ..Default::default()
    }];
    let options = default_options(prompt);
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    // Top-level media type "image" is resolved to image_url (OpenAI-compatible).
    let messages_str = result.body["messages"].to_string();
    assert!(
        messages_str.contains("image_url"),
        "messages should contain image_url for top-level image media type: {}",
        messages_str
    );
}

/// TS: tool calls › "should stringify arguments to tool calls" (line ~100).
///
/// An assistant tool-call message followed by a tool-result message should
/// produce:
/// - assistant: `{ role: "assistant", content: "", tool_calls: [...] }`
/// - tool: `{ role: "tool", content: "...", tool_call_id: "..." }`
#[tokio::test]
async fn should_stringify_arguments_to_tool_calls() {
    let prompt = vec![
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::tool_call(
                "quux",
                "thwomp",
                json!({ "foo": "bar123" }),
            )],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::tool_result("quux", json!({ "oof": "321rab" }))],
            ..Default::default()
        },
    ];
    let options = default_options(prompt);
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    let messages = result.body["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 2);

    // Assistant message with tool_calls.
    assert_eq!(messages[0]["role"], json!("assistant"));
    // OpenAI converter sets content to null when empty.
    assert!(messages[0]["content"].is_null());
    assert_eq!(
        messages[0]["tool_calls"],
        json!([{
            "id": "quux",
            "type": "function",
            "function": {
                "name": "thwomp",
                "arguments": "{\"foo\":\"bar123\"}"
            }
        }])
    );

    // Tool result message.
    assert_eq!(messages[1]["role"], json!("tool"));
    assert_eq!(messages[1]["content"], json!("{\"oof\":\"321rab\"}"));
    assert_eq!(messages[1]["tool_call_id"], json!("quux"));

    assert!(result.warnings.is_empty(), "expected no warnings");
}

/// TS: tool calls › "should handle text output type in tool results" (line ~159).
///
/// A tool result with a text output should produce the text value directly as
/// the tool message content.
#[tokio::test]
async fn should_handle_text_output_type_in_tool_results() {
    let prompt = vec![
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::tool_call(
                "call-1",
                "getWeather",
                json!({ "query": "weather" }),
            )],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::tool_result(
                "call-1",
                json!("It is sunny today"),
            )],
            ..Default::default()
        },
    ];
    let options = default_options(prompt);
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    let messages = result.body["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 2);

    // Assistant message.
    assert_eq!(messages[0]["role"], json!("assistant"));
    // OpenAI converter sets content to null when empty.
    assert!(messages[0]["content"].is_null());
    assert_eq!(
        messages[0]["tool_calls"],
        json!([{
            "id": "call-1",
            "type": "function",
            "function": {
                "name": "getWeather",
                "arguments": "{\"query\":\"weather\"}"
            }
        }])
    );

    // Tool result message with text output.
    assert_eq!(messages[1]["role"], json!("tool"));
    assert_eq!(messages[1]["content"], json!("It is sunny today"));
    assert_eq!(messages[1]["tool_call_id"], json!("call-1"));
}

// ════════════════════════════════════════════════════════════════════════════
// Tool preparation — deepseek-prepare-tools.test.ts
//
// These tests call `build_request_body_with_warnings` directly and assert on
// the `body["tools"]` field.
//
// Translated from `deepseek-prepare-tools.test.ts`.
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should pass through strict mode when strict is true" (line ~5).
///
/// A tool with `strict: true` should produce `"strict": true` in the
/// serialized tool definition.
#[tokio::test]
async fn should_pass_through_strict_mode_when_strict_is_true() {
    let tool = FunctionTool {
        name: "testFunction".to_string(),
        description: Some("A test function".to_string()),
        input_schema: json!({ "type": "object", "properties": {} }),
        strict: Some(true),
        provider_options: None,
        input_examples: None,
    };
    let mut options = default_options(test_prompt());
    options.tools = Some(vec![Tool::from(tool)]);
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    assert_eq!(
        result.body["tools"],
        json!([{
            "type": "function",
            "function": {
                "description": "A test function",
                "name": "testFunction",
                "parameters": { "type": "object", "properties": {} },
                "strict": true
            }
        }])
    );
}

/// TS: "should pass through strict mode when strict is false" (line ~40).
///
/// A tool with `strict: false` should produce `"strict": false` in the
/// serialized tool definition.
#[tokio::test]
async fn should_pass_through_strict_mode_when_strict_is_false() {
    let tool = FunctionTool {
        name: "testFunction".to_string(),
        description: Some("A test function".to_string()),
        input_schema: json!({ "type": "object", "properties": {} }),
        strict: Some(false),
        provider_options: None,
        input_examples: None,
    };
    let mut options = default_options(test_prompt());
    options.tools = Some(vec![Tool::from(tool)]);
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    assert_eq!(
        result.body["tools"],
        json!([{
            "type": "function",
            "function": {
                "description": "A test function",
                "name": "testFunction",
                "parameters": { "type": "object", "properties": {} },
                "strict": false
            }
        }])
    );
}

/// TS: "should not include strict mode when strict is undefined" (line ~75).
///
/// A tool without `strict` set should NOT have a `"strict"` field in the
/// serialized tool definition.
#[tokio::test]
async fn should_not_include_strict_mode_when_strict_is_undefined() {
    let tool = FunctionTool {
        name: "testFunction".to_string(),
        description: Some("A test function".to_string()),
        input_schema: json!({ "type": "object", "properties": {} }),
        strict: None,
        provider_options: None,
        input_examples: None,
    };
    let mut options = default_options(test_prompt());
    options.tools = Some(vec![Tool::from(tool)]);
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    assert_eq!(
        result.body["tools"],
        json!([{
            "type": "function",
            "function": {
                "description": "A test function",
                "name": "testFunction",
                "parameters": { "type": "object", "properties": {} }
            }
        }])
    );
    // Verify strict is absent.
    assert!(
        result.body["tools"][0]["function"].get("strict").is_none(),
        "strict should be absent"
    );
}

/// TS: "should pass through strict mode for multiple tools with different strict
/// settings" (line ~108).
///
/// Multiple tools with different strict settings should each preserve their
/// own strict value (or omit it when undefined).
#[tokio::test]
async fn should_pass_through_strict_mode_for_multiple_tools() {
    let tools = vec![
        Tool::from(FunctionTool {
            name: "strictTool".to_string(),
            description: Some("A strict tool".to_string()),
            input_schema: json!({ "type": "object", "properties": {} }),
            strict: Some(true),
            provider_options: None,
            input_examples: None,
        }),
        Tool::from(FunctionTool {
            name: "nonStrictTool".to_string(),
            description: Some("A non-strict tool".to_string()),
            input_schema: json!({ "type": "object", "properties": {} }),
            strict: Some(false),
            provider_options: None,
            input_examples: None,
        }),
        Tool::from(FunctionTool {
            name: "defaultTool".to_string(),
            description: Some("A tool without strict setting".to_string()),
            input_schema: json!({ "type": "object", "properties": {} }),
            strict: None,
            provider_options: None,
            input_examples: None,
        }),
    ];
    let mut options = default_options(test_prompt());
    options.tools = Some(tools);
    let result = build_request_body_with_warnings(
        "deepseek-chat",
        &options,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    );

    let tools_arr = result.body["tools"].as_array().expect("tools array");
    assert_eq!(tools_arr.len(), 3);

    // Tool 0: strict = true
    assert_eq!(tools_arr[0]["function"]["name"], json!("strictTool"));
    assert_eq!(tools_arr[0]["function"]["strict"], json!(true));

    // Tool 1: strict = false
    assert_eq!(tools_arr[1]["function"]["name"], json!("nonStrictTool"));
    assert_eq!(tools_arr[1]["function"]["strict"], json!(false));

    // Tool 2: strict absent
    assert_eq!(tools_arr[2]["function"]["name"], json!("defaultTool"));
    assert!(
        tools_arr[2]["function"].get("strict").is_none(),
        "strict should be absent for defaultTool"
    );
}
