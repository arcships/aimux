//! Ports of `src/chat/deepseek-chat-language-model.test.ts` from `@ai-sdk/deepseek@3.0.56`:
//! the snapshot cases of `doGenerate` (`text`, `reasoning`, `tool call`, and
//! `tool call › json response format`) and of `doStream` (`text`, `reasoning`,
//! `tool call`).
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `toMatchSnapshot()` values are the `.snap` entries in `snapshots/`, as JSON:
//!   `undefined` properties are absent (as `JSON.stringify` drops them) and the
//!   `Date` timestamps are ISO strings. The result, or each stream part, is
//!   serialized and compared as a `serde_json::Value`.
//! - The `prepare*FixtureResponse` helpers are the `fixtures/` files, served by
//!   [`MockFetch`]. The TypeScript test server adds `content-length` to JSON
//!   responses and `cache-control` / `connection` to event streams; the mock does
//!   the same.
//! - A `stream-chunks` response is one `data:` frame per line of the
//!   `.chunks.txt` fixture, then `data: [DONE]`. Blank lines carry no frame and
//!   are skipped.
//!
//! Not ported: `json response format with structured outputs` (three cases:
//! `should send json_schema response format and skip schema injection`,
//! `should honor strictJsonSchema provider option` and `should fall back to
//! json_object without a schema`). The upstream builds that model directly with
//! `supportsStructuredOutputs: true`. The public API has no such setting:
//! `create_deepseek(..).chat(..)` only turns on the beta-endpoint flags, and the
//! chat model never sends a `json_schema` response format.

use std::sync::Arc;

use crate::deepseek::{DeepSeekProvider, DeepSeekProviderSettings, create_deepseek};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::StreamResult;
use aimux_core::tool::{FunctionTool, Tool};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch};

const TEXT_FIXTURE: &str = include_str!("fixtures/deepseek-text.json");
const REASONING_FIXTURE: &str = include_str!("fixtures/deepseek-reasoning.json");
const JSON_FIXTURE: &str = include_str!("fixtures/deepseek-json.json");
const TOOL_CALL_FIXTURE: &str = include_str!("fixtures/deepseek-tool-call.json");
const TEXT_CHUNKS: &str = include_str!("fixtures/deepseek-text.chunks.txt");
const REASONING_CHUNKS: &str = include_str!("fixtures/deepseek-reasoning.chunks.txt");
const TOOL_CALL_CHUNKS: &str = include_str!("fixtures/deepseek-tool-call.chunks.txt");

const SNAPSHOT_DO_GENERATE_TEXT: &str = include_str!("snapshots/do_generate_text.json");
const SNAPSHOT_DO_GENERATE_REASONING: &str = include_str!("snapshots/do_generate_reasoning.json");
const SNAPSHOT_DO_GENERATE_JSON_RESPONSE_FORMAT: &str =
    include_str!("snapshots/do_generate_json_response_format.json");
const SNAPSHOT_DO_GENERATE_TOOL_CALL: &str = include_str!("snapshots/do_generate_tool_call.json");
const SNAPSHOT_DO_STREAM_TEXT: &str = include_str!("snapshots/do_stream_text.json");
const SNAPSHOT_DO_STREAM_REASONING: &str = include_str!("snapshots/do_stream_reasoning.json");
const SNAPSHOT_DO_STREAM_TOOL_CALL: &str = include_str!("snapshots/do_stream_tool_call.json");

// ── helpers ─────────────────────────────────────────────────────────────────

fn json_value(source: &str) -> Value {
    serde_json::from_str(source).expect("fixture or snapshot is JSON")
}

/// `createDeepSeek({ apiKey })` with a mock transport.
fn provider(fetch: &Arc<MockFetch>) -> DeepSeekProvider {
    create_deepseek(DeepSeekProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// `prompt: TEST_PROMPT`.
fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn weather_tool() -> Tool {
    Tool::Function(FunctionTool::new(
        "weather",
        json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
    ))
}

/// The `tools: [weather]` and `providerOptions.deepseek.thinking: enabled` of
/// the upstream tool-call cases.
fn tool_options() -> CallOptions {
    let mut call = options();
    call.tools = Some(vec![weather_tool()]);
    call.provider_options = Some(
        serde_json::from_value(json!({ "deepseek": { "thinking": { "type": "enabled" } } }))
            .unwrap(),
    );
    call
}

/// `responseFormat: { type: 'json' }` (no schema) on top of [`tool_options`].
fn json_options() -> CallOptions {
    let mut call = tool_options();
    call.response_format = Some(ResponseFormat::Json {
        schema: None,
        name: None,
        description: None,
    });
    call
}

/// A non-streaming response with the headers the TypeScript test server sends.
fn json_response(body: &Value) -> Canned {
    let bytes = serde_json::to_vec(body).expect("serializable");
    Canned {
        status: 200,
        headers: vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("content-length".to_string(), bytes.len().to_string()),
        ],
        body: bytes,
    }
}

/// A single-response mock for a JSON body.
fn json_fetch(body: &Value) -> Arc<MockFetch> {
    MockFetch::new(vec![json_response(body)])
}

/// A single event-stream mock: one `data:` frame per payload, then `[DONE]`.
fn stream_fetch(payloads: &[String]) -> Arc<MockFetch> {
    let mut body: String = payloads
        .iter()
        .map(|payload| format!("data: {payload}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![
            ("content-type".to_string(), "text/event-stream".to_string()),
            ("cache-control".to_string(), "no-cache".to_string()),
            ("connection".to_string(), "keep-alive".to_string()),
        ],
        body: body.into_bytes(),
    }])
}

/// A `stream-chunks` mock from a `.chunks.txt` fixture: one frame per non-blank line.
fn chunks_fetch(source: &str) -> Arc<MockFetch> {
    let payloads: Vec<String> = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    stream_fetch(&payloads)
}

/// All parts of a stream, each serialized as the AI SDK writes it.
async fn stream_parts(result: StreamResult) -> Value {
    Value::Array(
        result
            .stream
            .map(|part| serde_json::to_value(part.expect("stream part")).unwrap())
            .collect()
            .await,
    )
}

// ── doGenerate and doStream ────────────────────────────────────────────────

mod deep_seek_chat_language_model {
    use super::*;

    mod do_generate {
        use super::*;

        mod text {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › text › should extract text content
            #[tokio::test]
            async fn should_extract_text_content() {
                let fetch = json_fetch(&json_value(TEXT_FIXTURE));
                let result = provider(&fetch)
                    .chat("deepseek-chat")
                    .do_generate(&options())
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&result).unwrap(),
                    json_value(SNAPSHOT_DO_GENERATE_TEXT)
                );
            }
        }

        mod reasoning {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › reasoning › should extract text content
            #[tokio::test]
            async fn should_extract_text_content() {
                let fetch = json_fetch(&json_value(REASONING_FIXTURE));
                let result = provider(&fetch)
                    .chat("deepseek-chat")
                    .do_generate(&options())
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&result).unwrap(),
                    json_value(SNAPSHOT_DO_GENERATE_REASONING)
                );
            }
        }

        mod tool_call {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › tool call › should extract tool call content
            #[tokio::test]
            async fn should_extract_tool_call_content() {
                let fetch = json_fetch(&json_value(TOOL_CALL_FIXTURE));
                let result = provider(&fetch)
                    .chat("deepseek-reasoner")
                    .do_generate(&tool_options())
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&result).unwrap(),
                    json_value(SNAPSHOT_DO_GENERATE_TOOL_CALL)
                );
            }

            mod json_response_format {
                use super::*;

                /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doGenerate › tool call › json response format › should extract text content
                #[tokio::test]
                async fn should_extract_text_content() {
                    let fetch = json_fetch(&json_value(JSON_FIXTURE));
                    let result = provider(&fetch)
                        .chat("deepseek-reasoner")
                        .do_generate(&json_options())
                        .await
                        .unwrap();
                    assert_eq!(
                        serde_json::to_value(&result).unwrap(),
                        json_value(SNAPSHOT_DO_GENERATE_JSON_RESPONSE_FORMAT)
                    );
                }
            }
        }
    }

    mod do_stream {
        use super::*;

        mod text {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › text › should stream text
            #[tokio::test]
            async fn should_stream_text() {
                let fetch = chunks_fetch(TEXT_CHUNKS);
                let result = provider(&fetch)
                    .chat("deepseek-chat")
                    .do_stream(&options())
                    .await
                    .unwrap();
                assert_eq!(
                    stream_parts(result).await,
                    json_value(SNAPSHOT_DO_STREAM_TEXT)
                );
            }
        }

        mod reasoning {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › reasoning › should stream reasoning
            #[tokio::test]
            async fn should_stream_reasoning() {
                let fetch = chunks_fetch(REASONING_CHUNKS);
                let result = provider(&fetch)
                    .chat("deepseek-reasoning")
                    .do_stream(&options())
                    .await
                    .unwrap();
                assert_eq!(
                    stream_parts(result).await,
                    json_value(SNAPSHOT_DO_STREAM_REASONING)
                );
            }
        }

        mod tool_call {
            use super::*;

            /// upstream: @ai-sdk/deepseek src/chat/deepseek-chat-language-model.test.ts › DeepSeekChatLanguageModel › doStream › tool call › should stream tool call
            #[tokio::test]
            async fn should_stream_tool_call() {
                let fetch = chunks_fetch(TOOL_CALL_CHUNKS);
                let result = provider(&fetch)
                    .chat("deepseek-reasoner")
                    .do_stream(&tool_options())
                    .await
                    .unwrap();
                assert_eq!(
                    stream_parts(result).await,
                    json_value(SNAPSHOT_DO_STREAM_TOOL_CALL)
                );
            }
        }
    }
}
