//! Ports of `src/cohere-chat-language-model.test.ts` from `@ai-sdk/cohere@4.0.52`.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `toMatchSnapshot` values are the `.snap` entries in `snapshots/`, with
//!   `undefined` properties absent (as `JSON.stringify` drops them).
//! - Inline snapshots and `toStrictEqual` that list `undefined` properties
//!   are written without those keys, since the serialized results omit them.
//! - The TypeScript test server adds `content-length` to JSON responses and
//!   `cache-control` / `connection` to event streams; the mock does the same.
//!   The `content-length` is the byte length of the compact JSON body.
//! - The mock records `user-agent`, which the TypeScript `requestHeaders` do
//!   not list; the header comparisons drop it.
//!
//! Cases that are not here (`should stream text deltas` and
//! `should handle unparsable stream parts`) are ported in
//! `upstream_e2e.rs`. `should extract text response` and
//! `should extract tool calls` compare the whole snapshot here; the older
//! cases in that file check part of it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::cohere::{CohereProvider, CohereProviderSettings, create_cohere};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{FilePart, LanguageModelMessage, TextPart, UserPart};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::StreamResult;
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_core::tool::{FunctionTool, LanguageModelV4ToolChoice, Tool};
use aimux_core::types::ReasoningEffort;
use aimux_provider_utils::HeaderMapOpt;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch};

const MODEL: &str = "command-r-plus";
const TEXT_FIXTURE: &str = include_str!("fixtures/cohere-text.json");
const MAX_TOKENS_FIXTURE: &str = include_str!("fixtures/cohere-max-tokens.json");
const NULL_ARGS_FIXTURE: &str = include_str!("fixtures/cohere-null-args.json");
const REASONING_FIXTURE: &str = include_str!("fixtures/cohere-reasoning.json");
const CITATIONS_FIXTURE: &str = include_str!("fixtures/cohere-citations.json");
const TOOL_CALL_FIXTURE: &str = include_str!("fixtures/cohere-tool-call.json");
const TEXT_CHUNKS: &str = include_str!("fixtures/cohere-text.chunks.txt");
const REASONING_CHUNKS: &str = include_str!("fixtures/cohere-reasoning.chunks.txt");
const TOOL_CALL_CHUNKS: &str = include_str!("fixtures/cohere-tool-call.chunks.txt");
const EMPTY_TOOL_CALL_CHUNKS: &str = include_str!("fixtures/cohere-empty-tool-call.chunks.txt");

const SNAPSHOT_DO_GENERATE_CITATIONS: &str = include_str!("snapshots/do_generate_citations.json");
const SNAPSHOT_DO_GENERATE_NULL_TOOL_CALL_ARGS: &str =
    include_str!("snapshots/do_generate_null_tool_call_args.json");
const SNAPSHOT_DO_GENERATE_REASONING: &str = include_str!("snapshots/do_generate_reasoning.json");
const SNAPSHOT_DO_GENERATE_TEXT: &str = include_str!("snapshots/do_generate_text.json");
const SNAPSHOT_DO_GENERATE_TOOL_CALL: &str = include_str!("snapshots/do_generate_tool_call.json");
const SNAPSHOT_DO_STREAM_EMPTY_TOOL_CALL: &str =
    include_str!("snapshots/do_stream_empty_tool_call.json");
const SNAPSHOT_DO_STREAM_REASONING: &str = include_str!("snapshots/do_stream_reasoning.json");
const SNAPSHOT_DO_STREAM_RAW_CHUNKS: &str = include_str!("snapshots/do_stream_raw_chunks.json");
const SNAPSHOT_DO_STREAM_TOOL_CALL: &str = include_str!("snapshots/do_stream_tool_call.json");

// ── helpers ─────────────────────────────────────────────────────────────────

fn json_value(source: &str) -> Value {
    serde_json::from_str(source).expect("fixture or snapshot is JSON")
}

/// `createCohere({ apiKey, headers, generateId })` with a mock transport.
fn provider(fetch: &Arc<MockFetch>) -> CohereProvider {
    provider_with(fetch, None, None)
}

fn provider_with(
    fetch: &Arc<MockFetch>,
    headers: Option<HeaderMapOpt>,
    generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
) -> CohereProvider {
    create_cohere(CohereProviderSettings {
        api_key: Some("test-api-key".into()),
        headers,
        generate_id,
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// `TEST_PROMPT`: a system message and the user message "Hello".
fn options() -> CallOptions {
    CallOptions::new(vec![
        LanguageModelMessage::System {
            content: "you are a friendly bot!".into(),
            provider_options: None,
        },
        LanguageModelMessage::user_text("Hello"),
    ])
}

/// A prompt with one user message made of the given parts.
fn user_options(parts: Vec<UserPart>) -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::User {
        content: parts,
        provider_options: None,
    }])
}

fn text_part(text: &str) -> UserPart {
    UserPart::Text(TextPart {
        text: text.into(),
        provider_options: None,
    })
}

/// A file part with inline bytes, as `Buffer.from(...)` or a string gives.
fn file_part(bytes: &[u8], media_type: &str, filename: &str) -> UserPart {
    UserPart::File(FilePart {
        data: FileData::Data {
            data: FileBytes::Binary(bytes.to_vec()),
        },
        media_type: media_type.into(),
        filename: Some(filename.into()),
        provider_options: None,
    })
}

fn test_tool() -> Tool {
    Tool::Function(FunctionTool::new(
        "test-tool",
        json!({
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
    ))
}

fn provider_options(value: Value) -> SharedProviderOptions {
    serde_json::from_value(value).expect("provider options are an object")
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// The request headers the TypeScript test checks with `toStrictEqual`.
/// The user agent is recorded by the mock and not compared here.
fn assert_sent_headers(fetch: &MockFetch, expected: &[(&str, &str)]) {
    let mut headers = fetch.seen()[0].headers.clone();
    headers.remove("user-agent").expect("user-agent is sent");
    assert_eq!(headers, map(expected));
}

/// A non-streaming response with the headers the TypeScript test server sends.
fn json_response(body: &Value, extra_headers: &[(&str, &str)]) -> Canned {
    let bytes = serde_json::to_vec(body).expect("serializable");
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), bytes.len().to_string()),
    ];
    headers.extend(
        extra_headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    Canned {
        status: 200,
        headers,
        body: bytes,
    }
}

/// A single-response mock for a JSON fixture.
fn json_fetch(source: &str, extra_headers: &[(&str, &str)]) -> Arc<MockFetch> {
    MockFetch::new(vec![json_response(&json_value(source), extra_headers)])
}

/// One `event:` / `data:` frame, as the TypeScript helpers write it.
fn frame(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// `prepareChunksFixtureResponse`: every non-blank line of a `.chunks.txt`
/// fixture is one frame, named by its `type`.
fn chunks_frames(source: &str) -> Vec<String> {
    source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let parsed: Value = serde_json::from_str(line).expect("fixture line is JSON");
            frame(parsed["type"].as_str().unwrap(), line)
        })
        .collect()
}

/// `prepareChunkLinesResponse`: each chunk is serialized into one frame.
fn chunk_frames(chunks: &[Value]) -> Vec<String> {
    chunks
        .iter()
        .map(|chunk| frame(chunk["type"].as_str().unwrap(), &chunk.to_string()))
        .collect()
}

/// An event-stream response with the headers the TypeScript test server sends.
fn sse_response(frames: &[String], extra_headers: &[(&str, &str)]) -> Canned {
    let mut headers = vec![
        ("content-type".to_string(), "text/event-stream".to_string()),
        ("cache-control".to_string(), "no-cache".to_string()),
        ("connection".to_string(), "keep-alive".to_string()),
    ];
    headers.extend(
        extra_headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    Canned {
        status: 200,
        headers,
        body: frames.concat().into_bytes(),
    }
}

/// A single event-stream mock.
fn stream_fetch(frames: &[String], extra_headers: &[(&str, &str)]) -> Arc<MockFetch> {
    MockFetch::new(vec![sse_response(frames, extra_headers)])
}

/// `message-end` chunk with the given usage.
fn message_end(usage: Value) -> Value {
    json!({
        "type": "message-end",
        "delta": {"finish_reason": "COMPLETE", "usage": usage}
    })
}

/// All parts of a stream, each serialized as the AI SDK writes it.
async fn stream_parts(result: StreamResult) -> Vec<Value> {
    result
        .stream
        .map(|part| serde_json::to_value(part.expect("stream part")).unwrap())
        .collect()
        .await
}

/// `toMatchObject`: every property of `expected` is in `actual`, recursively;
/// arrays have the same length and match element by element.
fn assert_match_object(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(actual_map), Value::Object(expected_map)) => {
            for (key, value) in expected_map {
                let got = actual_map
                    .get(key)
                    .unwrap_or_else(|| panic!("missing key {key} in {actual}"));
                assert_match_object(got, value);
            }
        }
        (Value::Array(actual_items), Value::Array(expected_items)) => {
            assert_eq!(actual_items.len(), expected_items.len(), "in {actual}");
            for (got, want) in actual_items.iter().zip(expected_items) {
                assert_match_object(got, want);
            }
        }
        _ => assert_eq!(actual, expected),
    }
}

// ── doGenerate ──────────────────────────────────────────────────────────────

mod do_generate {
    mod text {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › text › should extract text response
        #[tokio::test]
        async fn should_extract_text_response() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_TEXT)
            );
        }
    }

    mod tool_call {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › tool call › should extract tool calls
        #[tokio::test]
        async fn should_extract_tool_calls() {
            let fetch = json_fetch(TOOL_CALL_FIXTURE, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&CallOptions {
                    tools: Some(vec![test_tool()]),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_TOOL_CALL)
            );
        }
    }

    mod max_tokens {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › max tokens › should map MAX_TOKENS finish reason to length
        #[tokio::test]
        async fn should_map_max_tokens_finish_reason_to_length() {
            let fetch = json_fetch(MAX_TOKENS_FIXTURE, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result.finish_reason).unwrap(),
                json!({"raw": "MAX_TOKENS", "unified": "length"})
            );
        }
    }

    mod null_tool_call_arguments {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › null tool call arguments › should handle string "null" tool call arguments
        #[tokio::test]
        async fn should_handle_string_null_tool_call_arguments() {
            let fetch = json_fetch(NULL_ARGS_FIXTURE, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&CallOptions {
                    tools: Some(vec![Tool::Function(FunctionTool::new(
                        "currentTime",
                        json!({
                            "type": "object",
                            "properties": {},
                            "required": [],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#"
                        }),
                    ))]),
                    ..CallOptions::new(vec![LanguageModelMessage::user_text(
                        "What is the current time?",
                    )])
                })
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result.content).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_NULL_TOOL_CALL_ARGS)
            );
        }
    }

    mod reasoning {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › reasoning › should extract reasoning from response
        #[tokio::test]
        async fn should_extract_reasoning_from_response() {
            let fetch = json_fetch(REASONING_FIXTURE, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_REASONING)
            );
        }
    }

    mod top_level_reasoning {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › top-level reasoning › should map top-level reasoning to thinking enabled with budget
        #[tokio::test]
        async fn should_map_top_level_reasoning_to_thinking_enabled_with_budget() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&CallOptions {
                    reasoning: Some(ReasoningEffort::High),
                    ..options()
                })
                .await
                .unwrap();
            let body = fetch.seen()[0].json_body();
            assert_eq!(body["thinking"]["type"], "enabled");
            assert!(
                body["thinking"]["token_budget"]
                    .as_f64()
                    .is_some_and(|budget| budget > 0.0)
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › top-level reasoning › should map top-level reasoning none to thinking disabled
        #[tokio::test]
        async fn should_map_top_level_reasoning_none_to_thinking_disabled() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&CallOptions {
                    reasoning: Some(ReasoningEffort::None),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body()["thinking"],
                json!({"type": "disabled"})
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › top-level reasoning › should prefer providerOptions over top-level reasoning
        #[tokio::test]
        async fn should_prefer_provider_options_over_top_level_reasoning() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&CallOptions {
                    reasoning: Some(ReasoningEffort::None),
                    provider_options: Some(provider_options(json!({
                        "cohere": {"thinking": {"type": "enabled"}}
                    }))),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(fetch.seen()[0].json_body()["thinking"]["type"], "enabled");
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › top-level reasoning › should not set thinking when reasoning is not specified
        #[tokio::test]
        async fn should_not_set_thinking_when_reasoning_is_not_specified() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert!(fetch.seen()[0].json_body().get("thinking").is_none());
        }
    }

    mod citations {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › citations › should extract citations from response
        #[tokio::test]
        async fn should_extract_citations_from_response() {
            let fetch = json_fetch(CITATIONS_FIXTURE, &[]);
            let provider = provider_with(
                &fetch,
                None,
                Some(Arc::new(|| "test-citation-id".to_string())),
            );
            let result = provider
                .chat(MODEL)
                .do_generate(&user_options(vec![
                    text_part("What are AI benefits?"),
                    file_part(
                        b"AI provides automation and efficiency.",
                        "text/plain",
                        "ai-benefits.txt",
                    ),
                ]))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json_value(SNAPSHOT_DO_GENERATE_CITATIONS)
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › citations › should extract text documents and send to API
        #[tokio::test]
        async fn should_extract_text_documents_and_send_to_api() {
            let fetch = json_fetch(CITATIONS_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&user_options(vec![
                    text_part("What does this say?"),
                    file_part(b"This is a test document.", "text/plain", "test.txt"),
                ]))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "documents": [
                        {"data": {"text": "This is a test document.", "title": "test.txt"}}
                    ],
                    "messages": [
                        {"content": "What does this say?", "role": "user"}
                    ],
                    "model": "command-r-plus"
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › citations › should extract multiple text documents
        #[tokio::test]
        async fn should_extract_multiple_text_documents() {
            let fetch = json_fetch(CITATIONS_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&user_options(vec![
                    text_part("What do these documents say?"),
                    file_part(b"First document content", "text/plain", "doc1.txt"),
                    file_part(b"Second document content", "text/plain", "doc2.txt"),
                ]))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "documents": [
                        {"data": {"text": "First document content", "title": "doc1.txt"}},
                        {"data": {"text": "Second document content", "title": "doc2.txt"}}
                    ],
                    "messages": [
                        {"content": "What do these documents say?", "role": "user"}
                    ],
                    "model": "command-r-plus"
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › citations › should support JSON files
        #[tokio::test]
        async fn should_support_json_files() {
            let fetch = json_fetch(CITATIONS_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&user_options(vec![
                    text_part("What is in this JSON?"),
                    file_part(b"{\"key\": \"value\"}", "application/json", "data.json"),
                ]))
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "documents": [
                        {"data": {"text": "{\"key\": \"value\"}", "title": "data.json"}}
                    ],
                    "messages": [
                        {"content": "What is in this JSON?", "role": "user"}
                    ],
                    "model": "command-r-plus"
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › citations › should not include mediaType in the outgoing payload (category D)
        #[tokio::test]
        async fn should_not_include_media_type_in_the_outgoing_payload() {
            let fetch = json_fetch(CITATIONS_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&user_options(vec![
                    text_part("What is this?"),
                    file_part(b"Some file content", "application/pdf", "document.pdf"),
                ]))
                .await
                .unwrap();
            let body = fetch.seen()[0].json_body();
            assert_eq!(
                body["documents"],
                json!([{"data": {"text": "Some file content", "title": "document.pdf"}}])
            );
            let text = body.to_string();
            assert!(!text.contains("application/pdf"));
            assert!(!text.contains("mediaType"));
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › citations › should successfully process supported text media types
        #[tokio::test]
        async fn should_successfully_process_supported_text_media_types() {
            let fetch = json_fetch(CITATIONS_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&user_options(vec![
                    text_part("What is this?"),
                    file_part(b"This is plain text content", "text/plain", "text.txt"),
                    file_part(b"# Markdown Header\nContent", "text/markdown", "doc.md"),
                ]))
                .await
                .unwrap();
            assert_match_object(
                &fetch.seen()[0].json_body(),
                &json!({
                    "documents": [
                        {"data": {"text": "This is plain text content", "title": "text.txt"}},
                        {"data": {"text": "# Markdown Header\nContent", "title": "doc.md"}}
                    ]
                }),
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › citations › should not include documents parameter when no files present
        #[tokio::test]
        async fn should_not_include_documents_parameter_when_no_files_present() {
            let fetch = json_fetch(CITATIONS_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert!(fetch.seen()[0].json_body().get("documents").is_none());
        }
    }

    mod request {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should pass model and messages
        #[tokio::test]
        async fn should_pass_model_and_messages() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "messages": [
                        {"content": "you are a friendly bot!", "role": "system"},
                        {"content": "Hello", "role": "user"}
                    ],
                    "model": "command-r-plus"
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should pass tools
        #[tokio::test]
        async fn should_pass_tools() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&CallOptions {
                    tool_choice: Some(LanguageModelV4ToolChoice::None),
                    tools: Some(vec![test_tool()]),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "messages": [
                        {"content": "you are a friendly bot!", "role": "system"},
                        {"content": "Hello", "role": "user"}
                    ],
                    "model": "command-r-plus",
                    "tool_choice": "NONE",
                    "tools": [
                        {
                            "function": {
                                "name": "test-tool",
                                "parameters": {
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                    "additionalProperties": false,
                                    "properties": {"value": {"type": "string"}},
                                    "required": ["value"],
                                    "type": "object"
                                }
                            },
                            "type": "function"
                        }
                    ]
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            let provider = provider_with(
                &fetch,
                Some(HashMap::from([(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )])),
                None,
            );
            provider
                .chat(MODEL)
                .do_generate(&CallOptions {
                    headers: Some(HashMap::from([(
                        "Custom-Request-Header".to_string(),
                        "request-header-value".to_string(),
                    )])),
                    ..options()
                })
                .await
                .unwrap();
            assert_sent_headers(
                &fetch,
                &[
                    ("authorization", "Bearer test-api-key"),
                    ("content-type", "application/json"),
                    ("custom-provider-header", "provider-header-value"),
                    ("custom-request-header", "request-header-value"),
                ],
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should pass response format
        #[tokio::test]
        async fn should_pass_response_format() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_generate(&CallOptions {
                    response_format: Some(ResponseFormat::Json {
                        schema: Some(json!({
                            "type": "object",
                            "properties": {"text": {"type": "string"}},
                            "required": ["text"]
                        })),
                        name: None,
                        description: None,
                    }),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "messages": [
                        {"content": "you are a friendly bot!", "role": "system"},
                        {"content": "Hello", "role": "user"}
                    ],
                    "model": "command-r-plus",
                    "response_format": {
                        "json_schema": {
                            "properties": {"text": {"type": "string"}},
                            "required": ["text"],
                            "type": "object"
                        },
                        "type": "json_object"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should send request body
        #[tokio::test]
        async fn should_send_request_body() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                result.request.unwrap().body,
                Some(json!({
                    "messages": [
                        {"content": "you are a friendly bot!", "role": "system"},
                        {"content": "Hello", "role": "user"}
                    ],
                    "model": "command-r-plus"
                }))
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should expose the raw response headers
        #[tokio::test]
        async fn should_expose_the_raw_response_headers() {
            let fetch = json_fetch(TEXT_FIXTURE, &[("test-header", "test-value")]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.response.unwrap().headers).unwrap(),
                json!({
                    "content-length": "304",
                    "content-type": "application/json",
                    "test-header": "test-value"
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should extract usage
        #[tokio::test]
        async fn should_extract_usage() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result.usage).unwrap(),
                json!({
                    "inputTokens": {"noCache": 507, "total": 507},
                    "outputTokens": {"text": 10, "total": 10},
                    "raw": {
                        "billed_units": {"input_tokens": 12, "output_tokens": 7},
                        "cached_tokens": 448,
                        "tokens": {"input_tokens": 507, "output_tokens": 10}
                    }
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should preserve extra top-level and nested fields in raw usage
        #[tokio::test]
        async fn should_preserve_extra_top_level_and_nested_fields_in_raw_usage() {
            let fetch = json_fetch(
                &json!({
                    "message": {
                        "role": "assistant",
                        "content": [{"type": "text", "text": "Hello"}]
                    },
                    "finish_reason": "COMPLETE",
                    "usage": {
                        "billed_units": {
                            "input_tokens": 12,
                            "output_tokens": 7,
                            "billing_tier": "standard"
                        },
                        "tokens": {
                            "input_tokens": 507,
                            "output_tokens": 10,
                            "tokenizer": "command"
                        },
                        "cached_tokens": 448,
                        "provider_usage_id": "usage-123"
                    }
                })
                .to_string(),
                &[],
            );
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result.usage).unwrap(),
                json!({
                    "inputTokens": {"total": 507, "noCache": 507},
                    "outputTokens": {"total": 10, "text": 10},
                    "raw": {
                        "billed_units": {
                            "input_tokens": 12,
                            "output_tokens": 7,
                            "billing_tier": "standard"
                        },
                        "tokens": {
                            "input_tokens": 507,
                            "output_tokens": 10,
                            "tokenizer": "command"
                        },
                        "cached_tokens": 448,
                        "provider_usage_id": "usage-123"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should validate cached token usage
        #[tokio::test]
        async fn should_validate_cached_token_usage() {
            let fetch = json_fetch(
                &json!({
                    "message": {
                        "role": "assistant",
                        "content": [{"type": "text", "text": "Hello"}]
                    },
                    "finish_reason": "COMPLETE",
                    "usage": {
                        "billed_units": {"input_tokens": 12, "output_tokens": 7},
                        "tokens": {"input_tokens": 507, "output_tokens": 10},
                        "cached_tokens": "448"
                    }
                })
                .to_string(),
                &[],
            );
            assert!(
                provider(&fetch)
                    .chat(MODEL)
                    .do_generate(&options())
                    .await
                    .is_err()
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doGenerate › request › should send additional response information
        #[tokio::test]
        async fn should_send_additional_response_information() {
            let fetch = json_fetch(TEXT_FIXTURE, &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_generate(&options())
                .await
                .unwrap();
            let response = result.response.unwrap();
            assert!(response.id.is_none());
            assert!(response.timestamp.is_none());
            assert!(response.model_id.is_none());
        }
    }
}

// ── doStream ────────────────────────────────────────────────────────────────

mod do_stream {
    mod text {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › text › should include raw chunks when includeRawChunks is enabled
        #[tokio::test]
        async fn should_include_raw_chunks_when_include_raw_chunks_is_enabled() {
            let fetch = stream_fetch(&chunks_frames(TEXT_CHUNKS), &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    include_raw_chunks: Some(true),
                    ..options()
                })
                .await
                .unwrap();
            let parts = stream_parts(result).await;
            let raw: Vec<Value> = parts
                .into_iter()
                .filter(|part| part["type"] == "raw")
                .collect();
            assert_eq!(Value::Array(raw), json_value(SNAPSHOT_DO_STREAM_RAW_CHUNKS));
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › text › should not include raw chunks when includeRawChunks is false
        #[tokio::test]
        async fn should_not_include_raw_chunks_when_include_raw_chunks_is_false() {
            let fetch = stream_fetch(&chunks_frames(TEXT_CHUNKS), &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            let parts = stream_parts(result).await;
            assert_eq!(parts.iter().filter(|part| part["type"] == "raw").count(), 0);
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › text › should preserve extra top-level and nested fields in raw usage
        #[tokio::test]
        async fn should_preserve_extra_top_level_and_nested_fields_in_raw_usage() {
            let raw_usage = json!({
                "billed_units": {
                    "input_tokens": 12,
                    "output_tokens": 7,
                    "billing_tier": "standard"
                },
                "tokens": {
                    "input_tokens": 507,
                    "output_tokens": 10,
                    "tokenizer": "command"
                },
                "cached_tokens": 448,
                "provider_usage_id": "usage-123"
            });
            let fetch = stream_fetch(&chunk_frames(&[message_end(raw_usage.clone())]), &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            let parts = stream_parts(result).await;
            assert_eq!(
                parts.last().unwrap(),
                &json!({
                    "type": "finish",
                    "finishReason": {"unified": "stop", "raw": "COMPLETE"},
                    "usage": {
                        "inputTokens": {"total": 507, "noCache": 507},
                        "outputTokens": {"total": 10, "text": 10},
                        "raw": raw_usage
                    }
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › text › should finish successfully when billed units are absent
        #[tokio::test]
        async fn should_finish_successfully_when_billed_units_are_absent() {
            let raw_usage = json!({"tokens": {"input_tokens": 507, "output_tokens": 10}});
            let fetch = stream_fetch(&chunk_frames(&[message_end(raw_usage.clone())]), &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            let parts = stream_parts(result).await;
            assert_eq!(
                parts.last().unwrap(),
                &json!({
                    "type": "finish",
                    "finishReason": {"unified": "stop", "raw": "COMPLETE"},
                    "usage": {
                        "inputTokens": {"total": 507, "noCache": 507},
                        "outputTokens": {"total": 10, "text": 10},
                        "raw": raw_usage
                    }
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › text › should validate streamed $field
        #[tokio::test]
        async fn should_validate_streamed_field() {
            // The `it.each` rows of the TypeScript test, one loop over them.
            let rows = [
                (
                    "billed input tokens",
                    json!({
                        "billed_units": {"input_tokens": "12", "output_tokens": 7},
                        "tokens": {"input_tokens": 507, "output_tokens": 10},
                        "cached_tokens": 448
                    }),
                ),
                (
                    "cached tokens",
                    json!({
                        "billed_units": {"input_tokens": 12, "output_tokens": 7},
                        "tokens": {"input_tokens": 507, "output_tokens": 10},
                        "cached_tokens": "448"
                    }),
                ),
            ];
            for (field, usage) in rows {
                let fetch = stream_fetch(&chunk_frames(&[message_end(usage)]), &[]);
                let result = provider(&fetch)
                    .chat(MODEL)
                    .do_stream(&CallOptions {
                        include_raw_chunks: Some(false),
                        ..options()
                    })
                    .await
                    .unwrap();
                let parts = stream_parts(result).await;
                assert_eq!(parts.len(), 3, "{field}");
                assert_eq!(
                    parts[0],
                    json!({"type": "stream-start", "warnings": []}),
                    "{field}"
                );
                assert_eq!(parts[1]["type"], "error", "{field}");
                assert_eq!(
                    parts[2],
                    json!({
                        "type": "finish",
                        "finishReason": {"unified": "error"},
                        "usage": {"inputTokens": {}, "outputTokens": {}}
                    }),
                    "{field}"
                );
            }
        }
    }

    mod reasoning {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › reasoning › should stream reasoning deltas
        #[tokio::test]
        async fn should_stream_reasoning_deltas() {
            let fetch = stream_fetch(&chunks_frames(REASONING_CHUNKS), &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                Value::Array(stream_parts(result).await),
                json_value(SNAPSHOT_DO_STREAM_REASONING)
            );
        }
    }

    mod tool_call {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › tool call › should stream tool deltas
        #[tokio::test]
        async fn should_stream_tool_deltas() {
            let fetch = stream_fetch(&chunks_frames(TOOL_CALL_CHUNKS), &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    tools: Some(vec![test_tool()]),
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                Value::Array(stream_parts(result).await),
                json_value(SNAPSHOT_DO_STREAM_TOOL_CALL)
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › tool call › rejects prototype keys in streamed tool call arguments
        #[tokio::test]
        async fn rejects_prototype_keys_in_streamed_tool_call_arguments() {
            let fetch = stream_fetch(
                &chunk_frames(&[
                    json!({
                        "type": "tool-call-start",
                        "index": 0,
                        "delta": {"message": {"tool_calls": {
                            "id": "test-tool-call",
                            "type": "function",
                            "function": {"name": "test-tool", "arguments": ""}
                        }}}
                    }),
                    json!({
                        "type": "tool-call-delta",
                        "index": 0,
                        "delta": {"message": {"tool_calls": {
                            "function": {"arguments": "{\"__proto__\":{\"polluted\":true}}"}
                        }}}
                    }),
                    json!({"type": "tool-call-end", "index": 0}),
                ]),
                &[],
            );
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    tools: Some(vec![Tool::Function(FunctionTool::new(
                        "test-tool",
                        json!({
                            "type": "object",
                            "properties": {},
                            "additionalProperties": true,
                            "$schema": "http://json-schema.org/draft-07/schema#"
                        }),
                    ))]),
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            let items: Vec<_> = result.stream.collect().await;
            let error = items
                .iter()
                .find_map(|item| item.as_ref().err())
                .expect("the stream fails");
            assert!(
                error
                    .to_string()
                    .contains("Object contains forbidden prototype property")
            );
        }
    }

    mod empty_tool_call {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › empty tool call › should handle empty tool call arguments
        #[tokio::test]
        async fn should_handle_empty_tool_call_arguments() {
            let fetch = stream_fetch(&chunks_frames(EMPTY_TOOL_CALL_CHUNKS), &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    tools: Some(vec![Tool::Function(FunctionTool::new(
                        "test-tool",
                        json!({
                            "type": "object",
                            "properties": {},
                            "required": [],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#"
                        }),
                    ))]),
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                Value::Array(stream_parts(result).await),
                json_value(SNAPSHOT_DO_STREAM_EMPTY_TOOL_CALL)
            );
        }
    }

    mod request {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › request › should pass the messages and the model
        #[tokio::test]
        async fn should_pass_the_messages_and_the_model() {
            let fetch = stream_fetch(&chunks_frames(TEXT_CHUNKS), &[]);
            provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                fetch.seen()[0].json_body(),
                json!({
                    "messages": [
                        {"content": "you are a friendly bot!", "role": "system"},
                        {"content": "Hello", "role": "user"}
                    ],
                    "model": "command-r-plus",
                    "stream": true
                })
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › request › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let fetch = stream_fetch(&chunks_frames(TEXT_CHUNKS), &[]);
            let provider = provider_with(
                &fetch,
                Some(HashMap::from([(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )])),
                None,
            );
            provider
                .chat(MODEL)
                .do_stream(&CallOptions {
                    headers: Some(HashMap::from([(
                        "Custom-Request-Header".to_string(),
                        "request-header-value".to_string(),
                    )])),
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            assert_sent_headers(
                &fetch,
                &[
                    ("authorization", "Bearer test-api-key"),
                    ("content-type", "application/json"),
                    ("custom-provider-header", "provider-header-value"),
                    ("custom-request-header", "request-header-value"),
                ],
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › request › should send request body
        #[tokio::test]
        async fn should_send_request_body() {
            let fetch = stream_fetch(&chunks_frames(TEXT_CHUNKS), &[]);
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                result.request.unwrap().body,
                Some(json!({
                    "messages": [
                        {"content": "you are a friendly bot!", "role": "system"},
                        {"content": "Hello", "role": "user"}
                    ],
                    "model": "command-r-plus",
                    "stream": true
                }))
            );
        }

        /// upstream: @ai-sdk/cohere src/cohere-chat-language-model.test.ts › doStream › request › should expose the raw response headers
        #[tokio::test]
        async fn should_expose_the_raw_response_headers() {
            let fetch = stream_fetch(
                &chunks_frames(TEXT_CHUNKS),
                &[("test-header", "test-value")],
            );
            let result = provider(&fetch)
                .chat(MODEL)
                .do_stream(&CallOptions {
                    include_raw_chunks: Some(false),
                    ..options()
                })
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.response.unwrap().headers).unwrap(),
                json!({
                    "cache-control": "no-cache",
                    "connection": "keep-alive",
                    "content-type": "text/event-stream",
                    "test-header": "test-value"
                })
            );
        }
    }
}
