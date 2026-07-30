//! Hugging Face Responses API tests, translated from the Vercel AI SDK
//! TypeScript suite.
//!
//! Translation source:
//! - `packages/huggingface/src/responses/huggingface-responses-language-model.test.ts`
//!   (31 test cases)
//!
//! HTTP is mocked with `wiremock` (a real loopback server), replacing the TS
//! `createTestServer`. Each test starts its own `MockServer` so parallel
//! `#[tokio::test]` runs do not collide.
//!
//! # Mock URL note
//!
//! The real Hugging Face base URL is `https://router.huggingface.co/v1`, so
//! production requests hit `/v1/responses`. In these tests `with_base_url` is
//! overridden with the mock server's root URI (no `/v1` suffix), so the
//! resulting request path is `/responses`.

use std::collections::HashMap;

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, Tool, ToolChoice};
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::huggingface::responses::convert_to_huggingface_responses_messages;
use aimux_providers::{HuggingFaceConfig, HuggingFaceProvider};

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
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

/// Build a Hugging Face provider whose base URL points at the mock server.
fn make_provider(server: &MockServer) -> HuggingFaceProvider {
    let config = HuggingFaceConfig::new("APIKEY").with_base_url(server.uri());
    HuggingFaceProvider::new(config)
}

/// Mount a JSON response on `/responses`.
async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE stream response on `/responses`.
async fn mock_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/responses"))
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

/// Concatenate SSE events (no `[DONE]` sentinel — HF Responses SSE has none).
fn sse_body(events: &[String]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
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

/// A minimal completed response body with a single message output.
fn basic_response_body() -> Value {
    json!({
        "id": "resp_67c97c0203188190a025beb4a75242bc",
        "model": "deepseek-ai/DeepSeek-V3-0324",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "error": null,
        "instructions": null,
        "max_output_tokens": null,
        "metadata": null,
        "tool_choice": "auto",
        "tools": [],
        "temperature": 1.0,
        "top_p": 1.0,
        "incomplete_details": null,
        "usage": {
            "input_tokens": 12,
            "output_tokens": 25,
            "total_tokens": 37
        },
        "output": [
            {
                "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [
                    {
                        "type": "output_text",
                        "text": "Hello! How can I help you today?"
                    }
                ]
            }
        ],
        "output_text": "Hello! How can I help you today?"
    })
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — basic text response
// ════════════════════════════════════════════════════════════════════════════

/// TS: doGenerate › basic text response › "should generate text"
#[tokio::test]
async fn should_generate_text() {
    let server = MockServer::start().await;
    mock_json(&server, basic_response_body()).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "Hello! How can I help you today?");
        }
        other => panic!("expected Text, got {:?}", other),
    }
}

/// TS: doGenerate › basic text response › "should extract usage"
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    mock_json(&server, basic_response_body()).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // usage: input_tokens=12, output_tokens=25, no cache/reasoning details
    assert_eq!(result.usage.input_tokens.total, Some(12));
    assert_eq!(result.usage.input_tokens.no_cache, Some(12));
    assert_eq!(result.usage.input_tokens.cache_read, Some(0));
    assert_eq!(result.usage.input_tokens.cache_write, None);
    assert_eq!(result.usage.output_tokens.total, Some(25));
    assert_eq!(result.usage.output_tokens.text, Some(25));
    assert_eq!(result.usage.output_tokens.reasoning, Some(0));
}

/// TS: doGenerate › basic text response › "should extract text from output
/// array when output_text is missing"
#[tokio::test]
async fn should_extract_text_from_output_array_when_output_text_missing() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_test",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [
                {
                    "id": "msg_test",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        { "type": "output_text", "text": "Extracted from output array" }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "Extracted from output array");
        }
        other => panic!("expected Text, got {:?}", other),
    }
}

/// TS: doGenerate › basic text response › "should handle missing usage
/// gracefully"
#[tokio::test]
async fn should_handle_missing_usage_gracefully() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_test",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [],
            "output_text": "Test response"
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // All usage fields should be None.
    assert_eq!(result.usage.input_tokens.total, None);
    assert_eq!(result.usage.input_tokens.no_cache, None);
    assert_eq!(result.usage.input_tokens.cache_read, None);
    assert_eq!(result.usage.input_tokens.cache_write, None);
    assert_eq!(result.usage.output_tokens.total, None);
    assert_eq!(result.usage.output_tokens.text, None);
    assert_eq!(result.usage.output_tokens.reasoning, None);
}

/// TS: doGenerate › basic text response › "should send model id, settings, and
/// input"
#[tokio::test]
async fn should_send_model_id_settings_and_input() {
    let server = MockServer::start().await;
    mock_json(&server, basic_response_body()).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

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
    options.max_output_tokens = Some(100);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["model"], json!("deepseek-ai/DeepSeek-V3-0324"));
    assert_eq!(body["temperature"], json!(0.5));
    assert_eq!(body["top_p"], json!(0.3));
    assert_eq!(body["max_output_tokens"], json!(100));
    assert_eq!(body["stream"], json!(false));
    assert_eq!(
        body["input"],
        json!([
            { "role": "system", "content": "You are a helpful assistant." },
            { "role": "user", "content": [{ "type": "input_text", "text": "Hello" }] }
        ])
    );
}

/// TS: doGenerate › basic text response › "should handle unsupported settings
/// with warnings"
#[tokio::test]
async fn should_handle_unsupported_settings_with_warnings() {
    let server = MockServer::start().await;
    mock_json(&server, basic_response_body()).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let mut options = default_options(test_prompt());
    options.top_k = Some(10.0);
    options.seed = Some(123);
    options.presence_penalty = Some(0.5);
    options.frequency_penalty = Some(0.3);
    options.stop_sequences = Some(vec!["stop".to_string()]);

    let result = model.do_generate(&options).await.expect("should succeed");

    let features: Vec<String> = result
        .warnings
        .iter()
        .filter_map(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => Some(feature.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(
        features,
        vec![
            "topK",
            "seed",
            "presencePenalty",
            "frequencyPenalty",
            "stopSequences"
        ]
    );
}

/// TS: doGenerate › basic text response › "should generate text and sources
/// from annotations"
#[tokio::test]
async fn should_generate_text_and_sources_from_annotations() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_test_annotations",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": { "input_tokens": 20, "output_tokens": 50, "total_tokens": 70 },
            "output": [
                {
                    "id": "msg_test_annotations",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "Here are some recent articles about AI.",
                            "annotations": [
                                { "type": "url_citation", "url": "https://example.com/article1", "title": "AI Developments Article" },
                                { "type": "url_citation", "url": "https://test.com/article2", "title": "Industry Trends Report" }
                            ]
                        }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // content: text + 2 sources
    assert_eq!(result.content.len(), 3);
    match &result.content[0] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "Here are some recent articles about AI.");
        }
        other => panic!("expected Text at [0], got {:?}", other),
    }
    match &result.content[1] {
        GenerateContent::Source { id, url, title, .. } => {
            assert_eq!(id, "id-0");
            assert_eq!(url.as_deref(), Some("https://example.com/article1"));
            assert_eq!(title.as_deref(), Some("AI Developments Article"));
        }
        other => panic!("expected Source at [1], got {:?}", other),
    }
    match &result.content[2] {
        GenerateContent::Source { id, url, title, .. } => {
            assert_eq!(id, "id-1");
            assert_eq!(url.as_deref(), Some("https://test.com/article2"));
            assert_eq!(title.as_deref(), Some("Industry Trends Report"));
        }
        other => panic!("expected Source at [2], got {:?}", other),
    }
}

/// TS: doGenerate › basic text response › "should handle MCP tools with
/// annotations"
#[tokio::test]
async fn should_handle_mcp_tools_with_annotations() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_mcp_test",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": { "input_tokens": 50, "output_tokens": 100, "total_tokens": 150 },
            "output": [
                {
                    "id": "mcp_search_test",
                    "type": "mcp_call",
                    "server_label": "web_search",
                    "name": "search",
                    "arguments": "{\"query\": \"San Francisco tech events\"}",
                    "output": "Found 25 tech events in San Francisco"
                },
                {
                    "id": "msg_mcp_response",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "Based on the search results.",
                            "annotations": [
                                { "type": "url_citation", "url": "https://techevents.com/sf-ai", "title": "SF AI Conference 2025" },
                                { "type": "url_citation", "url": "https://eventbrite.com/sf-startups", "title": "SF Startup Meetups" }
                            ]
                        }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // content: tool-call, text, source, source
    // (Rust GenerateContent has no ToolResult variant — the TS tool-result is
    // omitted.)
    assert_eq!(result.content.len(), 4);
    match &result.content[0] {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            assert_eq!(tool_call_id, "mcp_search_test");
            assert_eq!(tool_name, "search");
            assert_eq!(input, &json!({ "query": "San Francisco tech events" }));
        }
        other => panic!("expected ToolCall at [0], got {:?}", other),
    }
    match &result.content[1] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Based on the search results."),
        other => panic!("expected Text at [1], got {:?}", other),
    }
    match &result.content[2] {
        GenerateContent::Source { id, .. } => assert_eq!(id, "id-0"),
        other => panic!("expected Source at [2], got {:?}", other),
    }
    match &result.content[3] {
        GenerateContent::Source { id, .. } => assert_eq!(id, "id-1"),
        other => panic!("expected Source at [3], got {:?}", other),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doStream
// ════════════════════════════════════════════════════════════════════════════

/// TS: doStream › "should stream text deltas"
#[tokio::test]
async fn should_stream_text_deltas() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.created","response":{"id":"resp_test","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-V3-0324"}}"#,
        ),
        sse_event(
            r#"{"type":"response.in_progress","response":{"id":"resp_test","object":"response","created_at":1741269019,"status":"in_progress"}}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"in_progress","content":[]},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":"Hello,","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":" World!","sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello, World!"}]},"sequence_number":4}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_test","model":"deepseek-ai/DeepSeek-V3-0324","object":"response","created_at":1741269112,"status":"completed","incomplete_details":null,"usage":{"input_tokens":12,"output_tokens":25,"total_tokens":37},"output":[{"id":"msg_test","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello, World!"}]}]},"sequence_number":5}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    // Expected: stream-start, response-metadata, text-start, text-delta×2,
    // text-end, finish
    assert_eq!(parts.len(), 7);

    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));

    match &parts[1] {
        StreamPart::ResponseMetadata { id, model_id, .. } => {
            assert_eq!(id.as_deref(), Some("resp_test"));
            assert_eq!(model_id.as_deref(), Some("deepseek-ai/DeepSeek-V3-0324"));
        }
        other => panic!("expected ResponseMetadata, got {:?}", other),
    }

    match &parts[2] {
        StreamPart::TextStart { id, .. } => assert_eq!(id, "msg_test"),
        other => panic!("expected TextStart, got {:?}", other),
    }

    match &parts[3] {
        StreamPart::TextDelta { id, delta, .. } => {
            assert_eq!(id, "msg_test");
            assert_eq!(delta, "Hello,");
        }
        other => panic!("expected TextDelta, got {:?}", other),
    }
    match &parts[4] {
        StreamPart::TextDelta { id, delta, .. } => {
            assert_eq!(id, "msg_test");
            assert_eq!(delta, " World!");
        }
        other => panic!("expected TextDelta, got {:?}", other),
    }
    match &parts[5] {
        StreamPart::TextEnd { id, .. } => assert_eq!(id, "msg_test"),
        other => panic!("expected TextEnd, got {:?}", other),
    }
    match &parts[6] {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.input_tokens.total, Some(12));
            assert_eq!(usage.output_tokens.total, Some(25));
        }
        other => panic!("expected Finish, got {:?}", other),
    }
}

/// TS: doStream › "should handle streaming without usage"
#[tokio::test]
async fn should_handle_streaming_without_usage() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"in_progress"},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_test","output_index":0,"content_index":0,"delta":"Hi!","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant","status":"completed"},"sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_test","status":"completed","incomplete_details":null,"usage":null},"sequence_number":4}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;
    let finish = parts
        .iter()
        .find(|p| matches!(p, StreamPart::Finish { .. }))
        .expect("should have a Finish part");

    match finish {
        StreamPart::Finish { usage, .. } => {
            assert_eq!(usage.input_tokens.total, None);
            assert_eq!(usage.output_tokens.total, None);
        }
        _ => unreachable!(),
    }
}

/// TS: doStream › "should handle non-message item types"
#[tokio::test]
async fn should_handle_non_message_item_types() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"mcp_test","type":"mcp_list_tools","server_label":"test"},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"mcp_test","type":"mcp_list_tools","server_label":"test"},"sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_test","status":"completed","incomplete_details":null},"sequence_number":3}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    // Should only have stream-start and finish events (no text events for
    // non-message items).
    let types: Vec<&str> = parts
        .iter()
        .map(|p| match p {
            StreamPart::StreamStart { .. } => "stream-start",
            StreamPart::Finish { .. } => "finish",
            _ => "other",
        })
        .collect();
    assert_eq!(types, vec!["stream-start", "finish"]);
}

/// TS: doStream › "should handle streaming errors"
#[tokio::test]
async fn should_handle_streaming_errors() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_test","type":"message","role":"assistant"},"sequence_number":1}"#,
        ),
        "data:invalid json}\n\n".to_string(),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    let has_error = parts.iter().any(|p| matches!(p, StreamPart::Error { .. }));
    assert!(has_error, "should have an Error part");

    let finish = parts
        .iter()
        .find(|p| matches!(p, StreamPart::Finish { .. }))
        .expect("should have a Finish part");
    match finish {
        StreamPart::Finish { finish_reason, .. } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
        }
        _ => unreachable!(),
    }
}

/// TS: doStream › "should send correct streaming request"
#[tokio::test]
async fn should_send_correct_streaming_request() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[sse_event(
        r#"{"type":"response.completed","response":{"id":"resp_test","status":"completed"},"sequence_number":1}"#,
    )]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let mut options = default_options(test_prompt());
    options.temperature = Some(0.7);

    let result = model.do_stream(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["model"], json!("deepseek-ai/DeepSeek-V3-0324"));
    assert_eq!(body["temperature"], json!(0.7));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(
        body["input"],
        json!([{ "role": "user", "content": [{ "type": "input_text", "text": "Hello" }] }])
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Message conversion
// ════════════════════════════════════════════════════════════════════════════

/// Helper: mount a stub response so do_generate succeeds and we can inspect
/// the request body.
async fn stub_empty_response(server: &MockServer) {
    mock_json(
        server,
        json!({
            "id": "resp_test",
            "model": "moonshotai/Kimi-K2-Instruct",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [],
            "output_text": "Test response"
        }),
    )
    .await;
}

/// TS: message conversion › "should convert user messages with images"
#[tokio::test]
async fn should_convert_user_messages_with_images() {
    let server = MockServer::start().await;
    stub_empty_response(&server).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let prompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("What do you see?"),
            ContentPart::file_base64("AQIDBA==", "image/jpeg"),
        ],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("ok");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["input"][0]["content"],
        json!([
            { "text": "What do you see?", "type": "input_text" },
            { "image_url": "data:image/jpeg;base64,AQIDBA==", "type": "input_image" }
        ])
    );
}

/// TS: message conversion › "should throw for file parts with provider
/// references"
#[tokio::test]
async fn should_throw_for_file_parts_with_provider_references() {
    let server = MockServer::start().await;
    stub_empty_response(&server).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("Qwen/Qwen2.5-VL-32B-Instruct");

    let prompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::file_reference(
            "image/jpeg",
            json!({ "huggingface": "file-ref-123" }),
        )],
        ..Default::default()
    }];

    let result = model.do_generate(&default_options(prompt)).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string()
            .contains("file parts with provider references"),
        "error should mention provider references, got: {}",
        err
    );
}

/// TS: message conversion › "should handle assistant messages"
#[tokio::test]
async fn should_handle_assistant_messages() {
    let server = MockServer::start().await;
    stub_empty_response(&server).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let prompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hello")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::text("Hi there!")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("How are you?")],
            ..Default::default()
        },
    ];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("ok");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["input"],
        json!([
            { "content": [{ "text": "Hello", "type": "input_text" }], "role": "user" },
            { "content": [{ "text": "Hi there!", "type": "output_text" }], "role": "assistant" },
            { "content": [{ "text": "How are you?", "type": "input_text" }], "role": "user" }
        ])
    );
}

/// TS: message conversion › "should warn about unsupported assistant content
/// types" — tool-call, tool-result, and reasoning in assistant messages
/// produce NO warnings (they are silently skipped).
#[tokio::test]
async fn should_not_warn_about_assistant_content_types() {
    let server = MockServer::start().await;
    stub_empty_response(&server).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let prompt = vec![LanguageModelPromptMessage {
        role: Role::Assistant,
        content: vec![
            ContentPart::tool_call("test", "test", json!({})),
            ContentPart::tool_result("test", json!({ "type": "text", "value": "test" })),
            ContentPart::reasoning("thinking..."),
        ],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("ok");

    // TS expects no warnings (tool calls/results/reasoning are silently
    // skipped, reasoning is included as output_text).
    assert!(
        result.warnings.is_empty(),
        "expected no warnings, got: {:?}",
        result.warnings
    );
}

/// TS: message conversion › "should warn about tool messages"
#[tokio::test]
async fn should_warn_about_tool_messages() {
    let server = MockServer::start().await;
    stub_empty_response(&server).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let prompt = vec![LanguageModelPromptMessage {
        role: Role::Tool,
        content: vec![ContentPart::tool_result(
            "test",
            json!({ "type": "text", "value": "test" }),
        )],
        ..Default::default()
    }];

    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("ok");

    let features: Vec<String> = result
        .warnings
        .iter()
        .filter_map(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => Some(feature.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(features, vec!["tool messages"]);
}

// ════════════════════════════════════════════════════════════════════════════
// Tool calls
// ════════════════════════════════════════════════════════════════════════════

/// TS: tool calls › "should handle function_call tool responses"
#[tokio::test]
async fn should_handle_function_call_tool_responses() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_tool_test",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": { "input_tokens": 50, "output_tokens": 30, "total_tokens": 80 },
            "output": [
                {
                    "id": "fc_test",
                    "type": "function_call",
                    "call_id": "call_123",
                    "name": "getWeather",
                    "arguments": "{\"location\": \"New York\"}",
                    "output": "{\"temperature\": \"72°F\", \"condition\": \"sunny\"}"
                },
                {
                    "id": "msg_after_tool",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        { "type": "output_text", "text": "The weather in New York is 72°F and sunny." }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    // content: tool-call, text (Rust has no ToolResult in GenerateContent)
    assert_eq!(result.content.len(), 2);
    match &result.content[0] {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            assert_eq!(tool_call_id, "call_123");
            assert_eq!(tool_name, "getWeather");
            assert_eq!(input, &json!({ "location": "New York" }));
        }
        other => panic!("expected ToolCall at [0], got {:?}", other),
    }
    match &result.content[1] {
        GenerateContent::Text { text, .. } => {
            assert_eq!(text, "The weather in New York is 72°F and sunny.");
        }
        other => panic!("expected Text at [1], got {:?}", other),
    }
}

/// TS: tool calls › "should stream tool calls"
#[tokio::test]
async fn should_stream_tool_calls() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.created","response":{"id":"resp_tool_stream","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-V3-0324"}}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_stream","type":"function_call","call_id":"call_456","name":"calculator","arguments":""},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_stream","output_index":0,"delta":"{\"operation\"","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_stream","output_index":0,"delta":": \"add\", \"a\": 5, \"b\": 3}","sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"fc_stream","type":"function_call","call_id":"call_456","name":"calculator","arguments":"{\"operation\": \"add\", \"a\": 5, \"b\": 3}","output":"8"},"sequence_number":4}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_tool_stream","status":"completed","usage":{"input_tokens":20,"output_tokens":15,"total_tokens":35}},"sequence_number":5}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    // Expected: stream-start, response-metadata, tool-input-start,
    // tool-input-end, tool-call, tool-result, finish
    assert_eq!(parts.len(), 7);

    assert!(matches!(&parts[0], StreamPart::StreamStart { .. }));

    match &parts[1] {
        StreamPart::ResponseMetadata { id, model_id, .. } => {
            assert_eq!(id.as_deref(), Some("resp_tool_stream"));
            assert_eq!(model_id.as_deref(), Some("deepseek-ai/DeepSeek-V3-0324"));
        }
        other => panic!("expected ResponseMetadata, got {:?}", other),
    }

    match &parts[2] {
        StreamPart::ToolInputStart { id, tool_name, .. } => {
            assert_eq!(id, "call_456");
            assert_eq!(tool_name, "calculator");
        }
        other => panic!("expected ToolInputStart, got {:?}", other),
    }
    match &parts[3] {
        StreamPart::ToolInputEnd { id, .. } => assert_eq!(id, "call_456"),
        other => panic!("expected ToolInputEnd, got {:?}", other),
    }
    match &parts[4] {
        StreamPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            assert_eq!(tool_call_id, "call_456");
            assert_eq!(tool_name, "calculator");
            assert_eq!(input, &json!({ "operation": "add", "a": 5, "b": 3 }));
        }
        other => panic!("expected ToolCall, got {:?}", other),
    }
    match &parts[5] {
        StreamPart::ToolResult {
            tool_call_id,
            result,
            ..
        } => {
            assert_eq!(tool_call_id, "call_456");
            assert_eq!(result, &json!("8"));
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
    match &parts[6] {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.input_tokens.total, Some(20));
            assert_eq!(usage.output_tokens.total, Some(15));
        }
        other => panic!("expected Finish, got {:?}", other),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Structured output
// ════════════════════════════════════════════════════════════════════════════

/// TS: structured output › "should send text.format for structured output"
#[tokio::test]
async fn should_send_text_format_for_structured_output() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_structured",
            "model": "moonshotai/Kimi-K2-Instruct",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [
                {
                    "id": "msg_structured",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        { "type": "output_text", "text": "{\"name\": \"John Doe\", \"age\": 30}" }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("moonshotai/Kimi-K2-Instruct");

    let mut options = default_options(test_prompt());
    options.response_format = Some(ResponseFormat::Json {
        schema: Some(json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "number" }
            },
            "required": ["name", "age"]
        })),
        name: None,
        description: None,
    });

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["text"]["format"],
        json!({
            "type": "json_schema",
            "strict": false,
            "name": "response",
            "schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "age": { "type": "number" }
                },
                "required": ["name", "age"]
            }
        })
    );
    // description should NOT be present (it was None).
    assert!(
        body["text"]["format"].get("description").is_none(),
        "description should be absent when None"
    );
}

/// TS: structured output › "should handle structured output with custom name
/// and description"
#[tokio::test]
async fn should_handle_structured_output_with_custom_name_and_description() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_structured",
            "model": "moonshotai/Kimi-K2-Instruct",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [],
            "output_text": "{}"
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("moonshotai/Kimi-K2-Instruct");

    let mut options = default_options(test_prompt());
    options.response_format = Some(ResponseFormat::Json {
        schema: Some(json!({ "type": "object", "properties": { "name": { "type": "string" } } })),
        name: Some("person_profile".to_string()),
        description: Some("A person profile with basic information".to_string()),
    });

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["text"]["format"]["name"], json!("person_profile"));
    assert_eq!(
        body["text"]["format"]["description"],
        json!("A person profile with basic information")
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Reasoning
// ════════════════════════════════════════════════════════════════════════════

/// TS: reasoning › "should handle reasoning content in responses"
#[tokio::test]
async fn should_handle_reasoning_content_in_responses() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_reasoning",
            "model": "deepseek-ai/DeepSeek-R1",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": { "input_tokens": 10, "output_tokens": 50, "total_tokens": 60 },
            "output": [
                {
                    "id": "reasoning_1",
                    "type": "reasoning",
                    "content": [
                        { "type": "reasoning_text", "text": "Let me think about this problem step by step..." }
                    ]
                },
                {
                    "id": "msg_after_reasoning",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        { "type": "output_text", "text": "The answer is 42." }
                    ]
                }
            ],
            "output_text": null
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-R1");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 2);
    match &result.content[0] {
        GenerateContent::Reasoning {
            text,
            provider_metadata,
        } => {
            assert_eq!(text, "Let me think about this problem step by step...");
            assert_eq!(
                provider_metadata.as_ref(),
                Some(&json!({ "huggingface": { "itemId": "reasoning_1" } }))
            );
        }
        other => panic!("expected Reasoning at [0], got {:?}", other),
    }
    match &result.content[1] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "The answer is 42."),
        other => panic!("expected Text at [1], got {:?}", other),
    }
}

/// TS: reasoning › "should stream reasoning content"
#[tokio::test]
async fn should_stream_reasoning_content() {
    let server = MockServer::start().await;
    let chunks = sse_body(&[
        sse_event(
            r#"{"type":"response.created","response":{"id":"resp_reasoning_stream","object":"response","created_at":1741269019,"status":"in_progress","model":"deepseek-ai/DeepSeek-R1"}}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"reasoning_stream","type":"reasoning"},"sequence_number":1}"#,
        ),
        sse_event(
            r#"{"type":"response.reasoning_text.delta","item_id":"reasoning_stream","output_index":0,"content_index":0,"delta":"Thinking about","sequence_number":2}"#,
        ),
        sse_event(
            r#"{"type":"response.reasoning_text.delta","item_id":"reasoning_stream","output_index":0,"content_index":0,"delta":" the problem...","sequence_number":3}"#,
        ),
        sse_event(
            r#"{"type":"response.reasoning_text.done","item_id":"reasoning_stream","output_index":0,"content_index":0,"text":"Thinking about the problem...","sequence_number":4}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"reasoning_stream","type":"reasoning","content":[{"type":"reasoning_text","text":"Thinking about the problem..."}]},"sequence_number":5}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"msg_stream","type":"message","role":"assistant"},"sequence_number":6}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_stream","output_index":1,"content_index":0,"delta":"The solution is","sequence_number":7}"#,
        ),
        sse_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_stream","output_index":1,"content_index":0,"delta":" simple.","sequence_number":8}"#,
        ),
        sse_event(
            r#"{"type":"response.output_item.done","output_index":1,"item":{"id":"msg_stream","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"The solution is simple."}]},"sequence_number":9}"#,
        ),
        sse_event(
            r#"{"type":"response.completed","response":{"id":"resp_reasoning_stream","status":"completed","usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30}},"sequence_number":10}"#,
        ),
    ]);
    mock_sse(&server, chunks).await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-R1");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let parts = collect_stream(result).await;

    // Expected: stream-start, response-metadata, reasoning-start,
    // reasoning-delta×2, reasoning-end, text-start, text-delta×2, text-end,
    // finish
    assert_eq!(parts.len(), 11);

    assert!(matches!(&parts[0], StreamPart::StreamStart { .. }));

    match &parts[1] {
        StreamPart::ResponseMetadata { id, model_id, .. } => {
            assert_eq!(id.as_deref(), Some("resp_reasoning_stream"));
            assert_eq!(model_id.as_deref(), Some("deepseek-ai/DeepSeek-R1"));
        }
        other => panic!("expected ResponseMetadata, got {:?}", other),
    }

    match &parts[2] {
        StreamPart::ReasoningStart {
            id,
            provider_metadata,
        } => {
            assert_eq!(id, "reasoning_stream");
            assert_eq!(
                provider_metadata.as_ref(),
                Some(&json!({ "huggingface": { "itemId": "reasoning_stream" } }))
            );
        }
        other => panic!("expected ReasoningStart, got {:?}", other),
    }
    match &parts[3] {
        StreamPart::ReasoningDelta { id, delta, .. } => {
            assert_eq!(id, "reasoning_stream");
            assert_eq!(delta, "Thinking about");
        }
        other => panic!("expected ReasoningDelta, got {:?}", other),
    }
    match &parts[4] {
        StreamPart::ReasoningDelta { id, delta, .. } => {
            assert_eq!(id, "reasoning_stream");
            assert_eq!(delta, " the problem...");
        }
        other => panic!("expected ReasoningDelta, got {:?}", other),
    }
    match &parts[5] {
        StreamPart::ReasoningEnd { id, .. } => assert_eq!(id, "reasoning_stream"),
        other => panic!("expected ReasoningEnd, got {:?}", other),
    }
    match &parts[6] {
        StreamPart::TextStart { id, .. } => assert_eq!(id, "msg_stream"),
        other => panic!("expected TextStart, got {:?}", other),
    }
    match &parts[7] {
        StreamPart::TextDelta { id, delta, .. } => {
            assert_eq!(id, "msg_stream");
            assert_eq!(delta, "The solution is");
        }
        other => panic!("expected TextDelta, got {:?}", other),
    }
    match &parts[8] {
        StreamPart::TextDelta { id, delta, .. } => {
            assert_eq!(id, "msg_stream");
            assert_eq!(delta, " simple.");
        }
        other => panic!("expected TextDelta, got {:?}", other),
    }
    match &parts[9] {
        StreamPart::TextEnd { id, .. } => assert_eq!(id, "msg_stream"),
        other => panic!("expected TextEnd, got {:?}", other),
    }
    match &parts[10] {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(usage.input_tokens.total, Some(10));
            assert_eq!(usage.output_tokens.total, Some(20));
        }
        other => panic!("expected Finish, got {:?}", other),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Provider options
// ════════════════════════════════════════════════════════════════════════════

/// TS: provider options › "should send provider-specific options"
#[tokio::test]
async fn should_send_provider_specific_options() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_provider_options",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [],
            "output_text": "Test"
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let mut options = default_options(test_prompt());
    let mut po = HashMap::new();
    po.insert(
        "huggingface".to_string(),
        json!({
            "metadata": { "key": "value" },
            "instructions": "Be concise",
            "strictJsonSchema": true
        }),
    );
    options.provider_options = Some(po);
    options.response_format = Some(ResponseFormat::Json {
        schema: Some(json!({ "type": "object" })),
        name: None,
        description: None,
    });

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["metadata"], json!({ "key": "value" }));
    assert_eq!(body["instructions"], json!("Be concise"));
    assert_eq!(body["text"]["format"]["strict"], json!(true));
}

// ════════════════════════════════════════════════════════════════════════════
// Tool preparation
// ════════════════════════════════════════════════════════════════════════════

/// TS: tool preparation › "should prepare tools correctly"
#[tokio::test]
async fn should_prepare_tools_correctly() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_tools",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [],
            "output_text": "Test"
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    let mut options = default_options(test_prompt());
    options.tools = Some(vec![Tool::from(
        FunctionTool::new(
            "getWeather",
            json!({
                "type": "object",
                "properties": { "location": { "type": "string" } },
                "required": ["location"]
            }),
        )
        .with_description("Get weather information"),
    )]);
    options.tool_choice = ToolChoice::Tool {
        tool_name: "getWeather".to_string(),
    };

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(
        body["tools"],
        json!([{
            "type": "function",
            "name": "getWeather",
            "description": "Get weather information",
            "parameters": {
                "type": "object",
                "properties": { "location": { "type": "string" } },
                "required": ["location"]
            }
        }])
    );
    assert_eq!(
        body["tool_choice"],
        json!({ "type": "function", "function": { "name": "getWeather" } })
    );
}

/// TS: tool preparation › "should handle auto and required tool choices"
#[tokio::test]
async fn should_handle_auto_and_required_tool_choices() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_tools",
            "model": "deepseek-ai/DeepSeek-V3-0324",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [],
            "output_text": "Test"
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.responses_model("deepseek-ai/DeepSeek-V3-0324");

    // Test auto
    let mut options = default_options(test_prompt());
    options.tools = Some(vec![Tool::from(FunctionTool::new(
        "test",
        json!({ "type": "object" }),
    ))]);
    options.tool_choice = ToolChoice::Auto;

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["tool_choice"], json!("auto"));

    // Test required
    let mut options = default_options(test_prompt());
    options.tools = Some(vec![Tool::from(FunctionTool::new(
        "test",
        json!({ "type": "object" }),
    ))]);
    options.tool_choice = ToolChoice::Required;

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");
    assert_eq!(body["tool_choice"], json!("required"));
}

// ════════════════════════════════════════════════════════════════════════════
// Top-level-only media type resolution
// ════════════════════════════════════════════════════════════════════════════

const PNG_BASE64: &str = "iVBORw0KGgo=";

/// TS: "passes full image/png through unchanged for inline data"
#[test]
fn passes_full_image_png_through_unchanged_for_inline_data() {
    let prompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::file_base64(PNG_BASE64, "image/png")],
        ..Default::default()
    }];

    let (input, warnings) =
        convert_to_huggingface_responses_messages(&prompt).expect("should succeed");

    assert!(warnings.is_empty());
    let content = &input[0]["content"][0];
    assert_eq!(
        content,
        &json!({
            "type": "input_image",
            "image_url": format!("data:image/png;base64,{}", PNG_BASE64)
        })
    );
}

/// TS: "detects image subtype from inline bytes for top-level 'image'"
#[test]
fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
    let prompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::file_base64(PNG_BASE64, "image")],
        ..Default::default()
    }];

    let (input, warnings) =
        convert_to_huggingface_responses_messages(&prompt).expect("should succeed");

    assert!(warnings.is_empty());
    let content = &input[0]["content"][0];
    assert_eq!(
        content,
        &json!({
            "type": "input_image",
            "image_url": format!("data:image/png;base64,{}", PNG_BASE64)
        })
    );
}

/// TS: "passes through URL source for top-level-only image"
#[test]
fn passes_through_url_source_for_top_level_only_image() {
    let prompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::file_url("https://example.com/x.png", "image")],
        ..Default::default()
    }];

    let (input, warnings) =
        convert_to_huggingface_responses_messages(&prompt).expect("should succeed");

    assert!(warnings.is_empty());
    let content = &input[0]["content"][0];
    assert_eq!(
        content,
        &json!({
            "type": "input_image",
            "image_url": "https://example.com/x.png"
        })
    );
}

/// TS: "normalizes image/* wildcard via detection"
#[test]
fn normalizes_image_wildcard_via_detection() {
    let prompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::file_base64(PNG_BASE64, "image/*")],
        ..Default::default()
    }];

    let (input, warnings) =
        convert_to_huggingface_responses_messages(&prompt).expect("should succeed");

    assert!(warnings.is_empty());
    let content = &input[0]["content"][0];
    assert_eq!(
        content,
        &json!({
            "type": "input_image",
            "image_url": format!("data:image/png;base64,{}", PNG_BASE64)
        })
    );
}
