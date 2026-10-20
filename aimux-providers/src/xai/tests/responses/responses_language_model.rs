//! Port of `src/responses/xai-responses-language-model.test.ts` from
//! `@ai-sdk/xai@5.0.12`: the `doGenerate`, `doStream`, schema validation,
//! `response.incomplete` / `response.failed` and error event cases.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `createTestServer` is the [`MockFetch`] transport: `server.calls[n]` is
//!   `fetch.seen()[n]`, and the canned responses are queued in call order.
//! - `mockId()`: the TS model numbers its source ids itself, the Rust model
//!   draws them from a process-wide counter. [`content_json`] renumbers them by
//!   first appearance, so each result reads `id-0`, `id-1`, ... as upstream.
//! - JavaScript has one number type: [`assert_js_eq`] compares numbers as
//!   `f64`, so `40` and `40.0` are equal.
//! - `undefined` properties are absent from the expected JSON.
//! - `toMatchSnapshot()` values are `snapshots/<test name>.json`, converted
//!   from the vitest `.snap` file (their text matches the fixtures).

use std::sync::Arc;

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart,
    ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions, provider_namespace};
use aimux_core::tool::{FunctionTool, ProviderTool, Tool};
use aimux_core::types::ReasoningEffort;
use futures::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch};
use crate::xai::options::NAMESPACE;
use crate::xai::{XAIProviderSettings, XaiResponsesModel, create_xai};

/// The TS `createModel()` default model id.
const DEFAULT_MODEL_ID: &str = "grok-4-fast-non-reasoning";

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: one user text message "hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("hello")]
}

/// The TS `createModel(modelId)`: the xAI provider at its default base URL,
/// `Authorization: Bearer test-key` and the mock transport.
fn create_model(fetch: &Arc<MockFetch>, model_id: &str) -> XaiResponsesModel {
    create_xai(XAIProviderSettings {
        api_key: Some("test-key".to_string()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .expect("valid settings")
    .responses(model_id)
}

/// `prepareJsonResponse(body)`: one JSON response, for the first request.
fn json_fetch(body: &Value) -> Arc<MockFetch> {
    MockFetch::new(vec![Canned::json(body)])
}

/// `prepareJsonFixtureResponse(name)`: the parsed `__fixtures__/<name>.json`,
/// given as `include_str!` of it.
fn json_fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture is JSON")
}

/// `prepareStreamChunks(chunks)`: each chunk as `data: <chunk>`, then
/// `data: [DONE]`.
fn sse_fetch(frames: &[String]) -> Arc<MockFetch> {
    let mut body = String::new();
    for frame in frames {
        body.push_str(&format!("data: {frame}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }])
}

/// The `requests[call]` body as JSON (the TS `server.calls[call].requestBodyJson`).
fn request_body(fetch: &MockFetch, call: usize) -> Value {
    fetch.seen()[call].json_body()
}

/// `toMatchSnapshot()` value: the parsed `snapshots/<name>.json`.
fn snapshot(text: &str) -> Value {
    serde_json::from_str(text).expect("snapshot is JSON")
}

fn to_json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("value serializes")
}

/// `result.content` as JSON, with the source ids renumbered (see the module docs).
fn content_json(result: &GenerateResult) -> Value {
    renumber_source_ids(to_json(&result.content))
}

/// Whether a string is a source id the Rust model generates (`id-<digits>`).
fn is_source_id(text: &str) -> bool {
    text.strip_prefix("id-")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Renumber the `id-N` strings by first appearance: `id-0`, `id-1`, ...
fn renumber_source_ids(value: Value) -> Value {
    fn walk(value: Value, seen: &mut Vec<String>) -> Value {
        match value {
            Value::String(text) if is_source_id(&text) => {
                let index = match seen.iter().position(|known| *known == text) {
                    Some(index) => index,
                    None => {
                        seen.push(text);
                        seen.len() - 1
                    }
                };
                json!(format!("id-{index}"))
            }
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|item| walk(item, seen)).collect())
            }
            Value::Object(fields) => Value::Object(
                fields
                    .into_iter()
                    .map(|(key, field)| (key, walk(field, seen)))
                    .collect(),
            ),
            other => other,
        }
    }
    walk(value, &mut Vec::new())
}

/// JavaScript has one number type: `40` and `40.0` are the same number.
fn js_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) => number.as_f64().map_or(Value::Number(number), |n| json!(n)),
        Value::Array(items) => Value::Array(items.into_iter().map(js_numbers).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, field)| (key, js_numbers(field)))
                .collect(),
        ),
        other => other,
    }
}

/// `expect(actual).toEqual(expected)` on JSON.
fn assert_js_eq(actual: &Value, expected: Value) {
    assert_eq!(js_numbers(actual.clone()), js_numbers(expected));
}

/// `expect(list).toContainEqual(item)`.
fn assert_contains(list: &Value, item: Value) {
    let item = js_numbers(item);
    let items = list.as_array().expect("an array");
    assert!(
        items
            .iter()
            .any(|candidate| js_numbers(candidate.clone()) == item),
        "{list:#}\ndoes not contain\n{item:#}"
    );
}

/// `expect(actual).toMatchObject(expected)`, and `expect.objectContaining`
/// for nested objects: every key of `expected` matches, recursively. A failure
/// names the path of the key.
fn assert_matches_object(actual: &Value, expected: Value) {
    assert_matches_at(actual, &expected, "$");
}

fn assert_matches_at(actual: &Value, expected: &Value, path: &str) {
    match expected {
        Value::Object(fields) => {
            for (key, value) in fields {
                let here = format!("{path}.{key}");
                match actual.get(key) {
                    Some(found) => assert_matches_at(found, value, &here),
                    None => panic!("{here}: missing from {actual}; expected {value}"),
                }
            }
        }
        _ => {
            let (actual_js, expected_js) =
                (js_numbers(actual.clone()), js_numbers(expected.clone()));
            assert!(
                actual_js == expected_js,
                "{path}: expected {expected}, got {actual}"
            );
        }
    }
}

/// `prepareStreamChunks([...])` with `JSON.stringify`-ed events: one event per chunk.
fn stream_fetch(events: Vec<Value>) -> Arc<MockFetch> {
    let chunks: Vec<String> = events.iter().map(Value::to_string).collect();
    sse_fetch(&chunks)
}

/// `prepareChunksFixtureResponse(name)`: each line of a `.chunks.txt` fixture
/// (given as `include_str!` of it) is one chunk, then `data: [DONE]`.
fn chunks_fixture_fetch(text: &str) -> Arc<MockFetch> {
    let chunks: Vec<String> = text.split('\n').map(str::to_string).collect();
    sse_fetch(&chunks)
}

/// `convertReadableStreamToArray(stream)` as JSON. An `Err` item (a failure
/// the stream cannot carry on from) is kept as `{ type: 'error', error }`.
/// Source ids are renumbered as in [`content_json`].
async fn stream_parts(result: StreamResult) -> Value {
    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(item) = stream.next().await {
        parts.push(match item {
            Ok(part) => to_json(&part),
            Err(error) => json!({ "type": "error", "error": to_json(&error) }),
        });
    }
    renumber_source_ids(Value::Array(parts))
}

/// `doStream(options)` on a model of `model_id` answering from `fetch`, and the
/// parts of the stream.
async fn run_stream(fetch: &Arc<MockFetch>, model_id: &str, options: &CallOptions) -> Value {
    let result = create_model(fetch, model_id)
        .do_stream(options)
        .await
        .expect("should succeed");
    stream_parts(result).await
}

/// `parts.filter(part => part.type === kind)`.
fn parts_of_type(parts: &Value, kind: &str) -> Vec<Value> {
    parts
        .as_array()
        .expect("an array")
        .iter()
        .filter(|part| part["type"] == kind)
        .cloned()
        .collect()
}

/// `parts.find(part => part.type === kind)`, or `null`.
fn first_part(parts: &Value, kind: &str) -> Value {
    parts_of_type(parts, kind)
        .into_iter()
        .next()
        .unwrap_or(Value::Null)
}

/// The error part as upstream's `createProviderStreamError` shapes it.
///
/// aimux has no `AI_StreamProviderError`: an error event inside the stream
/// becomes an `AI_APICallError`, whose `providerCode` holds upstream's
/// `code`. Upstream's `type` is the event's own `type`, which aimux keeps in
/// `data`. This view renames those two fields and keeps every other value.
fn stream_provider_error_view(part: &Value) -> Value {
    let error = &part["error"];
    json!({
        "type": part["type"],
        "error": {
            "message": error["message"],
            "type": error["data"]["type"],
            "code": error["providerCode"],
            "statusCode": error["statusCode"],
            "isRetryable": error["isRetryable"],
            "data": error["data"]
        }
    })
}

/// `parts.findIndex(part => part.type === kind)`: `-1` when none.
fn index_of(parts: &Value, kind: &str) -> i64 {
    parts
        .as_array()
        .expect("an array")
        .iter()
        .position(|part| part["type"] == kind)
        .map_or(-1, |index| index as i64)
}

/// `providerOptions: { xai: options }` (the key is the xAI namespace).
fn xai_options(options: Value) -> SharedProviderOptions {
    provider_namespace(NAMESPACE, options).expect("options are an object")
}

/// A function tool, as the TS `{ type: 'function', name, description, inputSchema }`.
fn function_tool(name: &str, description: &str, input_schema: Value) -> Tool {
    Tool::Function(FunctionTool::new(name, input_schema).with_description(description))
}

/// A provider-defined tool, as the TS `{ type: 'provider', id, name, args }`.
fn provider_tool(id: &str, name: &str, args: Value) -> Tool {
    Tool::Provider(ProviderTool {
        id: id.parse().expect("namespaced tool id"),
        name: name.to_string(),
        args: args.as_object().cloned().expect("args are an object"),
        supports_deferred_results: None,
    })
}

fn system_message(content: &str) -> LanguageModelMessage {
    LanguageModelMessage::System {
        content: content.to_string(),
        provider_options: None,
    }
}

fn user_message(content: Vec<UserPart>) -> LanguageModelMessage {
    LanguageModelMessage::User {
        content,
        provider_options: None,
    }
}

fn text_part(text: &str) -> UserPart {
    UserPart::Text(TextPart {
        text: text.to_string(),
        provider_options: None,
    })
}

/// `{ type: 'file', mediaType: 'image/png', data: Buffer.from([0, 1, 2, 3]) }`.
fn png_file(provider_options: Option<SharedProviderOptions>) -> FilePart {
    FilePart {
        data: FileData::Data {
            data: FileBytes::Binary(vec![0, 1, 2, 3]),
        },
        media_type: "image/png".to_string(),
        filename: None,
        provider_options,
    }
}

/// The `/responses` body with the given output and usage that most cases use.
fn empty_response(usage: Value) -> Value {
    json!({
        "id": "resp_123",
        "object": "response",
        "status": "completed",
        "model": "grok-4-fast-non-reasoning",
        "output": [],
        "usage": usage
    })
}

// ─────────────────────────────────────────────────────────────────────────────

mod xai_responses_language_model {
    use super::*;

    mod do_generate {
        use super::*;

        mod basic_text_generation {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › basic text generation › should generate text content
            #[tokio::test]
            async fn should_generate_text_content() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "created_at": 1700000000,
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "message",
                            "id": "msg_123",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "hello world",
                                    "annotations": []
                                }
                            ]
                        }
                    ],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5,
                        "total_tokens": 15
                    }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([{ "text": "hello world", "type": "text" }]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › basic text generation › should extract usage correctly
            #[tokio::test]
            async fn should_extract_usage_correctly() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [],
                    "usage": {
                        "input_tokens": 345,
                        "output_tokens": 538,
                        "total_tokens": 883,
                        "output_tokens_details": {
                            "reasoning_tokens": 123
                        }
                    }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &to_json(&result.usage),
                    json!({
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 345,
                            "total": 345
                        },
                        "outputTokens": {
                            "reasoning": 123,
                            "text": 415,
                            "total": 538
                        },
                        "raw": {
                            "input_tokens": 345,
                            "output_tokens": 538,
                            "output_tokens_details": {
                                "reasoning_tokens": 123
                            },
                            "total_tokens": 883
                        }
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › basic text generation › should expose cost_in_usd_ticks in providerMetadata
            #[tokio::test]
            async fn should_expose_cost_in_usd_ticks_in_provider_metadata() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5,
                        "cost_in_usd_ticks": 113500
                    }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &to_json(&result.provider_metadata),
                    json!({ "xai": { "costInUsdTicks": 113500 } }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › basic text generation › should not include providerMetadata when cost_in_usd_ticks is missing
            #[tokio::test]
            async fn should_not_include_provider_metadata_when_cost_in_usd_ticks_is_missing() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert!(result.provider_metadata.is_none());
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › basic text generation › should expose echoed request identifiers in providerMetadata
            #[tokio::test]
            async fn should_expose_echoed_request_identifiers_in_provider_metadata() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [],
                    "usage": { "input_tokens": 10, "output_tokens": 5 },
                    "prompt_cache_key": "conversation-123",
                    "safety_identifier": "hashed-user-123"
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &to_json(&result.provider_metadata),
                    json!({
                        "xai": {
                            "promptCacheKey": "conversation-123",
                            "safetyIdentifier": "hashed-user-123"
                        }
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › basic text generation › should extract finish reason from status
            #[tokio::test]
            async fn should_extract_finish_reason_from_status() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &to_json(&result.finish_reason),
                    json!({ "raw": "completed", "unified": "stop" }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › basic text generation › should return tool-calls finish reason when function_call is present
            #[tokio::test]
            async fn should_return_tool_calls_finish_reason_when_function_call_is_present() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "function_call",
                            "id": "fc_123",
                            "name": "weather",
                            "arguments": "{\"location\":\"sf\"}",
                            "call_id": "call_123"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![function_tool(
                    "weather",
                    "get weather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } }
                    }),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &to_json(&result.finish_reason),
                    json!({ "raw": "completed", "unified": "tool-calls" }),
                );
            }
        }

        mod reasoning_content {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › reasoning content › should extract reasoning with encrypted content when store=false
            #[tokio::test]
            async fn should_extract_reasoning_with_encrypted_content_when_store_false() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "completed",
                            "summary": [
                                {
                                    "type": "summary_text",
                                    "text": "First, analyze the question carefully."
                                }
                            ],
                            "encrypted_content": "abc123encryptedcontent"
                        },
                        {
                            "type": "message",
                            "id": "msg_123",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "The answer is 42.",
                                    "annotations": []
                                }
                            ]
                        }
                    ],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 20,
                        "output_tokens_details": { "reasoning_tokens": 15 }
                    }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "providerMetadata": {
                                "xai": {
                                    "itemId": "rs_456",
                                    "reasoningEncryptedContent": "abc123encryptedcontent"
                                }
                            },
                            "text": "First, analyze the question carefully.",
                            "type": "reasoning"
                        },
                        {
                            "text": "The answer is 42.",
                            "type": "text"
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › reasoning content › should handle reasoning without encrypted content
            #[tokio::test]
            async fn should_handle_reasoning_without_encrypted_content() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "completed",
                            "summary": [
                                {
                                    "type": "summary_text",
                                    "text": "Thinking through the problem."
                                }
                            ]
                        },
                        {
                            "type": "message",
                            "id": "msg_123",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "Solution found.",
                                    "annotations": []
                                }
                            ]
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 15 }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "providerMetadata": { "xai": { "itemId": "rs_456" } },
                            "text": "Thinking through the problem.",
                            "type": "reasoning"
                        },
                        {
                            "text": "Solution found.",
                            "type": "text"
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › reasoning content › should extract reasoning from content when summary is empty
            #[tokio::test]
            async fn should_extract_reasoning_from_content_when_summary_is_empty() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-3-mini",
                    "output": [
                        {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "completed",
                            "summary": [],
                            "content": [
                                {
                                    "type": "reasoning_text",
                                    "text": "Let me think step by step."
                                }
                            ]
                        },
                        {
                            "type": "message",
                            "id": "msg_123",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "The answer is 444.",
                                    "annotations": []
                                }
                            ]
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 15 }
                }));

                let result = create_model(&fetch, "grok-3-mini")
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "providerMetadata": { "xai": { "itemId": "rs_456" } },
                            "text": "Let me think step by step.",
                            "type": "reasoning"
                        },
                        {
                            "text": "The answer is 444.",
                            "type": "text"
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › reasoning content › should extract reasoning with encrypted content but empty summary text
            #[tokio::test]
            async fn should_extract_reasoning_with_encrypted_content_but_empty_summary_text() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "reasoning",
                            "id": "rs_789",
                            "status": "completed",
                            "summary": [],
                            "encrypted_content": "encrypted_zdr_content_xyz"
                        },
                        {
                            "type": "message",
                            "id": "msg_123",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "Here is my response.",
                                    "annotations": []
                                }
                            ]
                        }
                    ],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 20,
                        "output_tokens_details": { "reasoning_tokens": 15 }
                    }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "providerMetadata": {
                                "xai": {
                                    "itemId": "rs_789",
                                    "reasoningEncryptedContent": "encrypted_zdr_content_xyz"
                                }
                            },
                            "text": "",
                            "type": "reasoning"
                        },
                        {
                            "text": "Here is my response.",
                            "type": "text"
                        }
                    ]),
                );
            }
        }

        mod settings_and_options {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › should send model id and settings
            #[tokio::test]
            async fn should_send_model_id_and_settings() {
                let fetch = json_fetch(&empty_response(
                    json!({ "input_tokens": 10, "output_tokens": 5 }),
                ));

                let mut options = CallOptions::new(vec![
                    system_message("you are helpful"),
                    user_message(vec![text_part("hello")]),
                ]);
                options.temperature = Some(0.5);
                options.top_p = Some(0.9);
                options.max_output_tokens = Some(100);
                create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &request_body(&fetch, 0),
                    json!({
                        "input": [
                            { "content": "you are helpful", "role": "system" },
                            {
                                "content": [{ "text": "hello", "type": "input_text" }],
                                "role": "user"
                            }
                        ],
                        "max_output_tokens": 100,
                        "model": "grok-4-fast-non-reasoning",
                        "temperature": 0.5,
                        "top_p": 0.9
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › should send image data in function call output
            #[tokio::test]
            async fn should_send_image_data_in_function_call_output() {
                let fetch = json_fetch(&empty_response(
                    json!({ "input_tokens": 10, "output_tokens": 5 }),
                ));

                let prompt = vec![
                    user_message(vec![text_part("Read the tool image.")]),
                    LanguageModelMessage::Assistant {
                        content: vec![AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id: "call_123".to_string(),
                            tool_name: "inspectImage".to_string(),
                            input: json!({}),
                            provider_executed: None,
                            provider_options: None,
                        })],
                        provider_options: None,
                    },
                    LanguageModelMessage::Tool {
                        content: vec![ToolPart::ToolResult(ToolResultPart {
                            tool_call_id: "call_123".to_string(),
                            tool_name: "inspectImage".to_string(),
                            output: ToolResultOutput::Content {
                                value: vec![
                                    ToolResultContent::Text(TextPart {
                                        text: "The requested image is attached.".to_string(),
                                        provider_options: None,
                                    }),
                                    ToolResultContent::File(png_file(None)),
                                ],
                            },
                            provider_options: None,
                        })],
                        provider_options: None,
                    },
                ];
                create_model(&fetch, "grok-4.5")
                    .do_generate(&CallOptions::new(prompt))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &request_body(&fetch, 0)["input"],
                    json!([
                        {
                            "content": [{ "text": "Read the tool image.", "type": "input_text" }],
                            "role": "user"
                        },
                        {
                            "arguments": "{}",
                            "call_id": "call_123",
                            "id": "call_123",
                            "name": "inspectImage",
                            "status": "completed",
                            "type": "function_call"
                        },
                        {
                            "call_id": "call_123",
                            "output": [
                                { "text": "The requested image is attached.", "type": "input_text" },
                                {
                                    "image_url": "data:image/png;base64,AAECAw==",
                                    "type": "input_image"
                                }
                            ],
                            "type": "function_call_output"
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › should warn about unsupported stopSequences
            #[tokio::test]
            async fn should_warn_about_unsupported_stop_sequences() {
                let fetch = json_fetch(&empty_response(
                    json!({ "input_tokens": 10, "output_tokens": 5 }),
                ));

                let mut options = CallOptions::new(test_prompt());
                options.stop_sequences = Some(vec!["\n\n".to_string(), "STOP".to_string()]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &to_json(&result.warnings),
                    json!([{ "feature": "stopSequences", "type": "unsupported" }]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › should warn about unsupported sampling settings
            #[tokio::test]
            async fn should_warn_about_unsupported_sampling_settings() {
                let fetch = json_fetch(&empty_response(
                    json!({ "input_tokens": 10, "output_tokens": 5 }),
                ));

                let mut options = CallOptions::new(test_prompt());
                options.top_k = Some(10.0);
                options.frequency_penalty = Some(0.5);
                options.presence_penalty = Some(0.5);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &to_json(&result.warnings),
                    json!([
                        { "feature": "frequencyPenalty", "type": "unsupported" },
                        { "feature": "presencePenalty", "type": "unsupported" }
                    ]),
                );
            }

            mod provider_options {
                use super::*;

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › reasoningEffort
                #[tokio::test]
                async fn reasoning_effort() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options =
                        Some(xai_options(json!({ "reasoningEffort": "high" })));
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "high");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › reasoningEffort: "xhigh" for grok-4.6
                #[tokio::test]
                async fn reasoning_effort_xhigh_for_grok_4_6() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options =
                        Some(xai_options(json!({ "reasoningEffort": "xhigh" })));
                    create_model(&fetch, "grok-4.6")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "xhigh");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › reasoningEffort: "none"
                #[tokio::test]
                async fn reasoning_effort_none() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options =
                        Some(xai_options(json!({ "reasoningEffort": "none" })));
                    create_model(&fetch, "grok-4.3")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "none");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › accepts deprecated reasoningSummary without sending it
                #[tokio::test]
                async fn accepts_deprecated_reasoning_summary_without_sending_it() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options =
                        Some(xai_options(json!({ "reasoningSummary": "concise" })));
                    create_model(&fetch, "grok-4.7")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert!(request_body(&fetch, 0).get("reasoning").is_none());
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › logprobs and topLogprobs
                #[tokio::test]
                async fn logprobs_and_top_logprobs() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options = Some(xai_options(json!({
                        "logprobs": true,
                        "topLogprobs": 6
                    })));
                    create_model(&fetch, "grok-4-fast")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    let body = request_body(&fetch, 0);
                    assert_eq!(body["logprobs"], true);
                    assert_js_eq(&body["top_logprobs"], json!(6));
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › topLogprobs enables logprobs
                #[tokio::test]
                async fn top_logprobs_enables_logprobs() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options = Some(xai_options(json!({ "topLogprobs": 2 })));
                    create_model(&fetch, "grok-4-fast")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    let body = request_body(&fetch, 0);
                    assert_eq!(body["logprobs"], true);
                    assert_js_eq(&body["top_logprobs"], json!(2));
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › store:true
                #[tokio::test]
                async fn store_true() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options = Some(xai_options(json!({ "store": true })));
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    let body = request_body(&fetch, 0);
                    assert!(body.get("store").is_none());
                    assert!(body.get("include").is_none());
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › store:false
                #[tokio::test]
                async fn store_false() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options = Some(xai_options(json!({ "store": false })));
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    let body = request_body(&fetch, 0);
                    assert_eq!(body["store"], false);
                    assert_js_eq(&body["include"], json!(["reasoning.encrypted_content"]));
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › previousResponseId
                #[tokio::test]
                async fn previous_response_id() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options =
                        Some(xai_options(json!({ "previousResponseId": "resp_456" })));
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["previous_response_id"], "resp_456");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › additional request options
                #[tokio::test]
                async fn additional_request_options() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.top_k = Some(40.0);
                    options.provider_options = Some(xai_options(json!({
                        "minP": 0.1,
                        "maxTurns": 5,
                        "parallelToolCalls": false,
                        "promptCacheKey": "conversation-123",
                        "safetyIdentifier": "hashed-user-123",
                        "serviceTier": "priority",
                        "user": "user-123"
                    })));
                    create_model(&fetch, "grok-4.7")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_matches_object(
                        &request_body(&fetch, 0),
                        json!({
                            "top_k": 40,
                            "min_p": 0.1,
                            "max_turns": 5,
                            "parallel_tool_calls": false,
                            "prompt_cache_key": "conversation-123",
                            "safety_identifier": "hashed-user-123",
                            "service_tier": "priority",
                            "user": "user-123"
                        }),
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › serviceTier
                #[tokio::test]
                async fn service_tier() {
                    let fetch = json_fetch(&json!({
                        "id": "resp_123",
                        "object": "response",
                        "status": "completed",
                        "model": "grok-4.6",
                        "output": [],
                        "usage": { "input_tokens": 10, "output_tokens": 5 },
                        "service_tier": "priority"
                    }));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options =
                        Some(xai_options(json!({ "serviceTier": "priority" })));
                    let result = create_model(&fetch, "grok-4.6")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["service_tier"], "priority");
                    assert_eq!(
                        to_json(&result.provider_metadata)["xai"]["serviceTier"],
                        "priority"
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › serviceTier reports a downgrade to default
                #[tokio::test]
                async fn service_tier_reports_a_downgrade_to_default() {
                    let fetch = json_fetch(&json!({
                        "id": "resp_123",
                        "object": "response",
                        "status": "completed",
                        "model": "grok-4.6",
                        "output": [],
                        "usage": { "input_tokens": 10, "output_tokens": 5 },
                        "service_tier": "default"
                    }));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options =
                        Some(xai_options(json!({ "serviceTier": "priority" })));
                    let result = create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(
                        to_json(&result.provider_metadata)["xai"]["serviceTier"],
                        "default"
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › include with file_search_call.results
                #[tokio::test]
                async fn include_with_file_search_call_results() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options = Some(xai_options(json!({
                        "include": ["file_search_call.results"]
                    })));
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_js_eq(
                        &request_body(&fetch, 0)["include"],
                        json!(["file_search_call.results"]),
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › include with no_inline_citations
                #[tokio::test]
                async fn include_with_no_inline_citations() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options = Some(xai_options(json!({
                        "include": ["no_inline_citations"]
                    })));
                    create_model(&fetch, "grok-4.7")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_js_eq(
                        &request_body(&fetch, 0)["include"],
                        json!(["no_inline_citations"]),
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › include with server-side tool outputs
                #[tokio::test]
                async fn include_with_server_side_tool_outputs() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options = Some(xai_options(json!({
                        "include": [
                            "web_search_call.action.sources",
                            "code_interpreter_call.outputs"
                        ]
                    })));
                    create_model(&fetch, "grok-4.7")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_js_eq(
                        &request_body(&fetch, 0)["include"],
                        json!([
                            "web_search_call.action.sources",
                            "code_interpreter_call.outputs"
                        ]),
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › provider options › include with file_search_call.results and store:false
                #[tokio::test]
                async fn include_with_file_search_call_results_and_store_false() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options = Some(xai_options(json!({
                        "include": ["file_search_call.results"],
                        "store": false
                    })));
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    let body = request_body(&fetch, 0);
                    assert_eq!(body["store"], false);
                    assert_js_eq(
                        &body["include"],
                        json!(["file_search_call.results", "reasoning.encrypted_content"]),
                    );
                }
            }

            mod top_level_reasoning {
                use super::*;

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › top-level reasoning › should map top-level reasoning to reasoning effort
                #[tokio::test]
                async fn should_map_top_level_reasoning_to_reasoning_effort() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.reasoning = Some(ReasoningEffort::High);
                    create_model(&fetch, "grok-4.3")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "high");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › top-level reasoning › should map top-level reasoning none to reasoning effort "none"
                #[tokio::test]
                async fn should_map_top_level_reasoning_none_to_reasoning_effort_none() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.reasoning = Some(ReasoningEffort::None);
                    create_model(&fetch, "grok-4.3")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "none");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › top-level reasoning › should coerce top-level reasoning xhigh to reasoning effort "high"
                #[tokio::test]
                async fn should_coerce_top_level_reasoning_xhigh_to_reasoning_effort_high() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.reasoning = Some(ReasoningEffort::Xhigh);
                    create_model(&fetch, "grok-4.3")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "high");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › top-level reasoning › should map top-level reasoning xhigh to reasoning effort "xhigh" for grok-4.6
                #[tokio::test]
                async fn should_map_top_level_reasoning_xhigh_to_reasoning_effort_xhigh_for_grok_4_6()
                 {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.reasoning = Some(ReasoningEffort::Xhigh);
                    create_model(&fetch, "grok-4.6")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "xhigh");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › top-level reasoning › should prefer providerOptions reasoningEffort over top-level reasoning
                #[tokio::test]
                async fn should_prefer_provider_options_reasoning_effort_over_top_level_reasoning()
                {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.reasoning = Some(ReasoningEffort::None);
                    options.provider_options =
                        Some(xai_options(json!({ "reasoningEffort": "high" })));
                    create_model(&fetch, "grok-4.3")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "high");
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › top-level reasoning › should omit reasoning effort and warn for models that do not support it
                #[tokio::test]
                async fn should_omit_reasoning_effort_and_warn_for_models_that_do_not_support_it() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.reasoning = Some(ReasoningEffort::None);
                    let result = create_model(&fetch, "grok-4.20-reasoning")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert!(request_body(&fetch, 0).get("reasoning").is_none());
                    assert_contains(
                        &to_json(&result.warnings),
                        json!({
                            "type": "unsupported",
                            "feature": "reasoning",
                            "details": "reasoning \"none\" is not supported by this model."
                        }),
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › top-level reasoning › should still pass providerOptions reasoningEffort for models that do not support top-level reasoning
                #[tokio::test]
                async fn should_still_pass_provider_options_reasoning_effort_without_top_level_reasoning()
                 {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.provider_options =
                        Some(xai_options(json!({ "reasoningEffort": "none" })));
                    create_model(&fetch, "grok-4.20-reasoning")
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(request_body(&fetch, 0)["reasoning"]["effort"], "none");
                }
            }

            mod image_detail {
                use super::*;

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › image detail › should pass detail from the imageDetail provider option on image parts
                #[tokio::test]
                async fn should_pass_detail_from_the_image_detail_provider_option_on_image_parts() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    create_model(&fetch, "grok-4.3")
                        .do_generate(&CallOptions::new(vec![user_message(vec![
                            text_part("What is in this image?"),
                            UserPart::File(png_file(Some(xai_options(
                                json!({ "imageDetail": "high" }),
                            )))),
                        ])]))
                        .await
                        .expect("should succeed");

                    assert_js_eq(
                        &request_body(&fetch, 0)["input"],
                        json!([
                            {
                                "role": "user",
                                "content": [
                                    { "type": "input_text", "text": "What is in this image?" },
                                    {
                                        "type": "input_image",
                                        "image_url": "data:image/png;base64,AAECAw==",
                                        "detail": "high"
                                    }
                                ]
                            }
                        ]),
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › image detail › should not set detail when the imageDetail provider option is not set
                #[tokio::test]
                async fn should_not_set_detail_when_the_image_detail_provider_option_is_not_set() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    create_model(&fetch, "grok-4.3")
                        .do_generate(&CallOptions::new(vec![user_message(vec![
                            text_part("What is in this image?"),
                            UserPart::File(png_file(None)),
                        ])]))
                        .await
                        .expect("should succeed");

                    assert_js_eq(
                        &request_body(&fetch, 0)["input"],
                        json!([
                            {
                                "role": "user",
                                "content": [
                                    { "type": "input_text", "text": "What is in this image?" },
                                    {
                                        "type": "input_image",
                                        "image_url": "data:image/png;base64,AAECAw=="
                                    }
                                ]
                            }
                        ]),
                    );
                }
            }

            mod response_format {
                use super::*;

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › responseFormat › should send response format json schema
                #[tokio::test]
                async fn should_send_response_format_json_schema() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.response_format = Some(ResponseFormat::Json {
                        schema: Some(json!({
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "ingredients": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["name", "ingredients"],
                            "additionalProperties": false
                        })),
                        name: Some("recipe".to_string()),
                        description: Some("A recipe object".to_string()),
                    });
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_js_eq(
                        &request_body(&fetch, 0),
                        json!({
                            "input": [
                                {
                                    "content": [{ "text": "hello", "type": "input_text" }],
                                    "role": "user"
                                }
                            ],
                            "model": "grok-4-fast-non-reasoning",
                            "text": {
                                "format": {
                                    "description": "A recipe object",
                                    "name": "recipe",
                                    "schema": {
                                        "additionalProperties": false,
                                        "properties": {
                                            "ingredients": {
                                                "items": { "type": "string" },
                                                "type": "array"
                                            },
                                            "name": { "type": "string" }
                                        },
                                        "required": ["name", "ingredients"],
                                        "type": "object"
                                    },
                                    "strict": true,
                                    "type": "json_schema"
                                }
                            }
                        }),
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › responseFormat › should send response format json object when no schema provided
                #[tokio::test]
                async fn should_send_response_format_json_object_when_no_schema_provided() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.response_format = Some(ResponseFormat::Json {
                        schema: None,
                        name: None,
                        description: None,
                    });
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_js_eq(
                        &request_body(&fetch, 0),
                        json!({
                            "input": [
                                {
                                    "content": [{ "text": "hello", "type": "input_text" }],
                                    "role": "user"
                                }
                            ],
                            "model": "grok-4-fast-non-reasoning",
                            "text": { "format": { "type": "json_object" } }
                        }),
                    );
                }

                /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › settings and options › responseFormat › should use default name when responseFormat.name is not provided
                #[tokio::test]
                async fn should_use_default_name_when_response_format_name_is_not_provided() {
                    let fetch = json_fetch(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    ));

                    let mut options = CallOptions::new(test_prompt());
                    options.response_format = Some(ResponseFormat::Json {
                        schema: Some(json!({
                            "type": "object",
                            "properties": { "value": { "type": "string" } }
                        })),
                        name: None,
                        description: None,
                    });
                    create_model(&fetch, DEFAULT_MODEL_ID)
                        .do_generate(&options)
                        .await
                        .expect("should succeed");

                    assert_eq!(
                        request_body(&fetch, 0)["text"]["format"]["name"],
                        "response"
                    );
                }
            }
        }

        mod web_search_tool {
            use super::*;

            /// The `beforeEach` of the describe: the fixture's response to a
            /// `xai.web_search` request, made with the model.
            async fn web_search_before_each(fetch: &Arc<MockFetch>) -> GenerateResult {
                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                create_model(fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed")
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › web_search tool › should include web_search tool call with providerExecuted true
            #[tokio::test]
            async fn should_include_web_search_tool_call_with_provider_executed_true() {
                let fetch = json_fetch(&json_fixture(include_str!(
                    "fixtures/xai-web-search-tool.1.json"
                )));

                let result = web_search_before_each(&fetch).await;

                assert_js_eq(
                    &content_json(&result),
                    snapshot(include_str!(
                        "snapshots/should_include_web_search_tool_call_with_provider_executed_true.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › web_search tool › should send web_search tool with args in request
            #[tokio::test]
            async fn should_send_web_search_tool_with_args_in_request() {
                // The `beforeEach` request is the first call; this test's is the second.
                let fetch = MockFetch::new(vec![
                    Canned::json(&json_fixture(include_str!(
                        "fixtures/xai-web-search-tool.1.json"
                    ))),
                    Canned::json(&empty_response(
                        json!({ "input_tokens": 10, "output_tokens": 5 }),
                    )),
                ]);
                web_search_before_each(&fetch).await;

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({
                        "allowedDomains": ["wikipedia.org"],
                        "enableImageSearch": true,
                        "enableImageUnderstanding": true
                    }),
                )]);
                create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &request_body(&fetch, 1),
                    json!({
                        "input": [
                            {
                                "content": [{ "text": "hello", "type": "input_text" }],
                                "role": "user"
                            }
                        ],
                        "model": "grok-4-fast-non-reasoning",
                        "tools": [
                            {
                                "allowed_domains": ["wikipedia.org"],
                                "enable_image_search": true,
                                "enable_image_understanding": true,
                                "type": "web_search"
                            }
                        ]
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › web_search tool › should forward web_search_call action and sources to tool-result
            #[tokio::test]
            async fn should_forward_web_search_call_action_and_sources_to_tool_result() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "web_search_call",
                            "id": "ws_action_1",
                            "name": "web_search",
                            "call_id": "",
                            "status": "completed",
                            "action": {
                                "type": "search",
                                "query": "latest AI news",
                                "sources": [
                                    { "type": "url", "url": "https://example.com/a" },
                                    { "type": "url", "url": "https://example.com/b" }
                                ]
                            }
                        },
                        {
                            "type": "web_search_call",
                            "id": "ws_action_2",
                            "name": "web_search",
                            "call_id": "",
                            "status": "completed",
                            "action": { "type": "open_page", "url": "https://example.com/a" }
                        },
                        {
                            "type": "web_search_call",
                            "id": "ws_action_3",
                            "name": "web_search",
                            "call_id": "",
                            "status": "completed",
                            "action": {
                                "type": "find_in_page",
                                "url": "https://example.com/a",
                                "pattern": "climate"
                            }
                        },
                        {
                            "type": "web_search_call",
                            "id": "ws_action_4",
                            "name": "web_search",
                            "call_id": "",
                            "status": "completed",
                            "action": { "type": "search", "query": null, "sources": null }
                        },
                        {
                            "type": "web_search_call",
                            "id": "ws_action_5",
                            "name": "web_search",
                            "call_id": "",
                            "status": "completed",
                            "action": { "type": "open_page", "url": null }
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                let content = content_json(&result);
                assert_contains(
                    &content,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_action_1",
                        "toolName": "web_search",
                        "result": {
                            "action": { "type": "search", "query": "latest AI news" },
                            "sources": [
                                { "type": "url", "url": "https://example.com/a" },
                                { "type": "url", "url": "https://example.com/b" }
                            ]
                        }
                    }),
                );
                assert_contains(
                    &content,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_action_2",
                        "toolName": "web_search",
                        "result": {
                            "action": { "type": "openPage", "url": "https://example.com/a" }
                        }
                    }),
                );
                assert_contains(
                    &content,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_action_3",
                        "toolName": "web_search",
                        "result": {
                            "action": {
                                "type": "findInPage",
                                "url": "https://example.com/a",
                                "pattern": "climate"
                            }
                        }
                    }),
                );
                assert_contains(
                    &content,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_action_4",
                        "toolName": "web_search",
                        "result": { "action": { "type": "search" } }
                    }),
                );
                assert_contains(
                    &content,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_action_5",
                        "toolName": "web_search",
                        "result": { "action": { "type": "openPage", "url": null } }
                    }),
                );
            }
        }

        mod x_search_tool {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › x_search tool › should include x_search tool call with real response
            #[tokio::test]
            async fn should_include_x_search_tool_call_with_real_response() {
                let fetch = json_fetch(&json_fixture(include_str!(
                    "fixtures/xai-x-search-tool.1.json"
                )));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool("xai.x_search", "x_search", json!({}))]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    snapshot(include_str!(
                        "snapshots/should_include_x_search_tool_call_with_real_response.json"
                    )),
                );
            }
        }

        mod code_interpreter_tool {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › code_interpreter tool › should include code_interpreter tool call with real response
            #[tokio::test]
            async fn should_include_code_interpreter_tool_call_with_real_response() {
                let fetch = json_fetch(&json_fixture(include_str!(
                    "fixtures/xai-code-execution-tool.1.json"
                )));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.code_execution",
                    "code_execution",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    snapshot(include_str!(
                        "snapshots/should_include_code_interpreter_tool_call_with_real_response.json"
                    )),
                );
            }
        }

        mod file_search_tool {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › file_search tool › should include file_search tool call and result with providerExecuted true
            #[tokio::test]
            async fn should_include_file_search_tool_call_and_result_with_provider_executed_true() {
                let fetch = json_fetch(&json_fixture(include_str!(
                    "fixtures/xai-file-search-tool.1.json"
                )));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.file_search",
                    "file_search",
                    json!({ "vectorStoreIds": ["collection_test123"] }),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "input": "",
                            "providerExecuted": true,
                            "toolCallId": "fs_123",
                            "toolName": "file_search",
                            "type": "tool-call"
                        },
                        {
                            "result": {
                                "queries": ["AI safety research"],
                                "results": [
                                    {
                                        "fileId": "file_abc123",
                                        "filename": "ai-safety-paper.pdf",
                                        "score": 0.95,
                                        "text": "Recent advances in AI safety research have focused on alignment techniques..."
                                    },
                                    {
                                        "fileId": "file_def456",
                                        "filename": "research-notes.md",
                                        "score": 0.82,
                                        "text": "Key findings from the AI safety workshop include recommendations for..."
                                    }
                                ]
                            },
                            "toolCallId": "fs_123",
                            "toolName": "file_search",
                            "type": "tool-result"
                        },
                        {
                            "text": "Based on the documents in your collection, I found relevant information about AI safety research. The main findings indicate that recent advances have focused on alignment techniques and the workshop recommendations emphasize the importance of interpretability.",
                            "type": "text"
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › file_search tool › should handle file_search with null results
            #[tokio::test]
            async fn should_handle_file_search_with_null_results() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "file_search_call",
                            "id": "fs_456",
                            "status": "completed",
                            "queries": ["nonexistent topic"],
                            "results": null
                        },
                        {
                            "type": "message",
                            "id": "msg_456",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "No relevant documents found.",
                                    "annotations": []
                                }
                            ]
                        }
                    ],
                    "usage": { "input_tokens": 100, "output_tokens": 20 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.file_search",
                    "file_search",
                    json!({ "vectorStoreIds": ["collection_test123"] }),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "input": "",
                            "providerExecuted": true,
                            "toolCallId": "fs_456",
                            "toolName": "file_search",
                            "type": "tool-call"
                        },
                        {
                            "result": {
                                "queries": ["nonexistent topic"],
                                "results": null
                            },
                            "toolCallId": "fs_456",
                            "toolName": "file_search",
                            "type": "tool-result"
                        },
                        {
                            "text": "No relevant documents found.",
                            "type": "text"
                        }
                    ]),
                );
            }
        }

        mod image_generation_tool {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › image_generation tool › should handle image_generation tool call from response fixture
            #[tokio::test]
            async fn should_handle_image_generation_tool_call_from_response_fixture() {
                let fetch = json_fetch(&json_fixture(include_str!(
                    "fixtures/xai-image-generation-tool.1.json"
                )));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "input": "{}",
                            "providerExecuted": true,
                            "toolCallId": "ig_00000000-0000-4000-8000-000000000001_call-00000000-0000-4000-8000-00000000000a-0",
                            "toolName": "image_generation",
                            "type": "tool-call"
                        },
                        {
                            "result": {
                                "prompt": "A cute robot holding a flower, adorable design with big friendly eyes, shiny metallic body with soft rounded edges, holding a colorful flower in its hand, cheerful expression, simple clean background, high quality digital art",
                                "result": "ZmFrZUltYWdlQmFzZTY0RGF0YQ=="
                            },
                            "toolCallId": "ig_00000000-0000-4000-8000-000000000001_call-00000000-0000-4000-8000-00000000000a-0",
                            "toolName": "image_generation",
                            "type": "tool-result"
                        },
                        {
                            "text": "Here's a cute robot holding a flower! 🌸🤖",
                            "type": "text"
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › image_generation tool › should include image_generation tool call and result with providerExecuted true
            #[tokio::test]
            async fn should_include_image_generation_tool_call_and_result_with_provider_executed_true()
             {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "image_generation_call",
                            "id": "ig_123",
                            "status": "completed",
                            "prompt": "A corgi surfing a big wave, Japanese woodblock print style",
                            "result": "base64imagedata"
                        },
                        {
                            "type": "message",
                            "id": "msg_123",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "Here is your image of a corgi surfing.",
                                    "annotations": []
                                }
                            ]
                        }
                    ],
                    "usage": { "input_tokens": 100, "output_tokens": 20 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "input": "{}",
                            "providerExecuted": true,
                            "toolCallId": "ig_123",
                            "toolName": "image_generation",
                            "type": "tool-call"
                        },
                        {
                            "result": {
                                "prompt": "A corgi surfing a big wave, Japanese woodblock print style",
                                "result": "base64imagedata"
                            },
                            "toolCallId": "ig_123",
                            "toolName": "image_generation",
                            "type": "tool-result"
                        },
                        {
                            "text": "Here is your image of a corgi surfing.",
                            "type": "text"
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › image_generation tool › should emit an error tool result for image_generation calls without result
            #[tokio::test]
            async fn should_emit_an_error_tool_result_for_image_generation_calls_without_result() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "image_generation_call",
                            "id": "ig_456",
                            "status": "failed"
                        }
                    ],
                    "usage": { "input_tokens": 100, "output_tokens": 20 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "input": "{}",
                            "providerExecuted": true,
                            "toolCallId": "ig_456",
                            "toolName": "image_generation",
                            "type": "tool-call"
                        },
                        {
                            "isError": true,
                            "result": "Image generation failed (status: failed).",
                            "toolCallId": "ig_456",
                            "toolName": "image_generation",
                            "type": "tool-result"
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › image_generation tool › should use the custom tool name for image_generation calls
            #[tokio::test]
            async fn should_use_the_custom_tool_name_for_image_generation_calls() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "image_generation_call",
                            "id": "ig_123",
                            "status": "completed",
                            "prompt": "A hot air balloon over the desert",
                            "result": "base64imagedata"
                        }
                    ],
                    "usage": { "input_tokens": 100, "output_tokens": 20 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.image_generation",
                    "generateImage",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "input": "{}",
                            "providerExecuted": true,
                            "toolCallId": "ig_123",
                            "toolName": "generateImage",
                            "type": "tool-call"
                        },
                        {
                            "result": {
                                "prompt": "A hot air balloon over the desert",
                                "result": "base64imagedata"
                            },
                            "toolCallId": "ig_123",
                            "toolName": "generateImage",
                            "type": "tool-result"
                        }
                    ]),
                );
            }
        }

        mod function_tools {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › function tools › should include function tool calls without providerExecuted
            #[tokio::test]
            async fn should_include_function_tool_calls_without_provider_executed() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "function_call",
                            "id": "fc_123",
                            "name": "weather",
                            "arguments": "{\"location\":\"sf\"}",
                            "call_id": "call_123"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![function_tool(
                    "weather",
                    "get weather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } }
                    }),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "input": "{\"location\":\"sf\"}",
                            "toolCallId": "call_123",
                            "toolName": "weather",
                            "type": "tool-call"
                        }
                    ]),
                );
            }
        }

        mod citations {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › citations › should extract citations from annotations
            #[tokio::test]
            async fn should_extract_citations_from_annotations() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "message",
                            "id": "msg_123",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "based on research",
                                    "annotations": [
                                        {
                                            "type": "url_citation",
                                            "url": "https://example.com",
                                            "title": "example title"
                                        },
                                        {
                                            "type": "url_citation",
                                            "url": "https://test.com"
                                        }
                                    ]
                                }
                            ]
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&CallOptions::new(test_prompt()))
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        { "text": "based on research", "type": "text" },
                        {
                            "id": "id-0",
                            "sourceType": "url",
                            "title": "example title",
                            "type": "source",
                            "url": "https://example.com"
                        },
                        {
                            "id": "id-1",
                            "sourceType": "url",
                            "title": "https://test.com",
                            "type": "source",
                            "url": "https://test.com"
                        }
                    ]),
                );
            }
        }

        mod multiple_tools {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › multiple tools › should handle multiple server-side tools
            #[tokio::test]
            async fn should_handle_multiple_server_side_tools() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "web_search_call",
                            "id": "ws_123",
                            "name": "web_search",
                            "arguments": "{}",
                            "call_id": "",
                            "status": "completed"
                        },
                        {
                            "type": "code_interpreter_call",
                            "id": "code_123",
                            "name": "code_execution",
                            "arguments": "{}",
                            "call_id": "",
                            "status": "completed"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![
                    provider_tool("xai.web_search", "web_search", json!({})),
                    provider_tool("xai.code_execution", "code_execution", json!({})),
                ]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                let content = content_json(&result);
                assert_eq!(content.as_array().expect("an array").len(), 3);
                assert_eq!(content[0]["type"], "tool-call");
                assert_eq!(content[1]["type"], "tool-result");
                assert_eq!(content[2]["type"], "tool-call");
            }
        }

        mod tool_name_mapping_by_type {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › tool name mapping by type › should map web_search_call type to web_search tool name when name is empty
            #[tokio::test]
            async fn should_map_web_search_call_type_to_web_search_tool_name_when_name_is_empty() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "web_search_call",
                            "id": "ws_123",
                            "name": "",
                            "arguments": "{\"query\":\"test\"}",
                            "call_id": "",
                            "status": "completed"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "type": "tool-call",
                            "toolCallId": "ws_123",
                            "toolName": "web_search",
                            "input": "{\"query\":\"test\"}",
                            "providerExecuted": true
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "ws_123",
                            "toolName": "web_search",
                            "result": {}
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › tool name mapping by type › should map x_search_call type to x_search tool name when name is empty
            #[tokio::test]
            async fn should_map_x_search_call_type_to_x_search_tool_name_when_name_is_empty() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "x_search_call",
                            "id": "xs_123",
                            "name": "",
                            "arguments": "{\"query\":\"test\"}",
                            "call_id": "",
                            "status": "completed"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool("xai.x_search", "x_search", json!({}))]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "type": "tool-call",
                            "toolCallId": "xs_123",
                            "toolName": "x_search",
                            "input": "{\"query\":\"test\"}",
                            "providerExecuted": true
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › tool name mapping by type › should map code_interpreter_call type to code_execution tool name when name is empty
            #[tokio::test]
            async fn should_map_code_interpreter_call_type_to_code_execution_when_name_empty() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "code_interpreter_call",
                            "id": "ci_123",
                            "name": "",
                            "arguments": "{}",
                            "call_id": "",
                            "status": "completed"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.code_execution",
                    "code_execution",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "type": "tool-call",
                            "toolCallId": "ci_123",
                            "toolName": "code_execution",
                            "input": "{}",
                            "providerExecuted": true
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › tool name mapping by type › should map code_execution_call type to code_execution tool name when name is empty
            #[tokio::test]
            async fn should_map_code_execution_call_type_to_code_execution_when_name_empty() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "code_execution_call",
                            "id": "ce_123",
                            "name": "",
                            "arguments": "{}",
                            "call_id": "",
                            "status": "completed"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.code_execution",
                    "code_execution",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "type": "tool-call",
                            "toolCallId": "ce_123",
                            "toolName": "code_execution",
                            "input": "{}",
                            "providerExecuted": true
                        }
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doGenerate › tool name mapping by type › should use custom tool name from provider tool when type matches
            #[tokio::test]
            async fn should_use_custom_tool_name_from_provider_tool_when_type_matches() {
                let fetch = json_fetch(&json!({
                    "id": "resp_123",
                    "object": "response",
                    "status": "completed",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [
                        {
                            "type": "web_search_call",
                            "id": "ws_123",
                            "name": "",
                            "arguments": "{}",
                            "call_id": "",
                            "status": "completed"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "my_custom_search",
                    json!({}),
                )]);
                let result = create_model(&fetch, DEFAULT_MODEL_ID)
                    .do_generate(&options)
                    .await
                    .expect("should succeed");

                assert_js_eq(
                    &content_json(&result),
                    json!([
                        {
                            "type": "tool-call",
                            "toolCallId": "ws_123",
                            "toolName": "my_custom_search",
                            "input": "{}",
                            "providerExecuted": true
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "ws_123",
                            "toolName": "my_custom_search",
                            "result": {}
                        }
                    ]),
                );
            }
        }
    }

    mod do_stream {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › should warn about unsupported sampling settings
        #[tokio::test]
        async fn should_warn_about_unsupported_sampling_settings() {
            let fetch =
                chunks_fixture_fetch(include_str!("fixtures/xai-text-streaming.1.chunks.txt"));

            let mut options = CallOptions::new(test_prompt());
            options.top_k = Some(10.0);
            options.frequency_penalty = Some(0.5);
            options.presence_penalty = Some(0.5);
            let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

            assert_js_eq(
                &first_part(&parts, "stream-start")["warnings"],
                json!([
                    { "feature": "frequencyPenalty", "type": "unsupported" },
                    { "feature": "presencePenalty", "type": "unsupported" }
                ]),
            );
        }

        mod text_streaming {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should stream web search with real response
            #[tokio::test]
            async fn should_stream_web_search_with_real_response() {
                let fetch =
                    chunks_fixture_fetch(include_str!("fixtures/xai-web-search-tool.1.chunks.txt"));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_js_eq(
                    &parts,
                    snapshot(include_str!(
                        "snapshots/should_stream_web_search_with_real_response.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should stream text deltas
            #[tokio::test]
            async fn should_stream_text_deltas() {
                let fetch =
                    chunks_fixture_fetch(include_str!("fixtures/xai-text-streaming.1.chunks.txt"));

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_js_eq(
                    &parts,
                    snapshot(include_str!("snapshots/should_stream_text_deltas.json")),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should stream text deltas with reasoning
            #[tokio::test]
            async fn should_stream_text_deltas_with_reasoning() {
                let fetch = chunks_fixture_fetch(include_str!(
                    "fixtures/xai-text-with-reasoning-streaming.1.chunks.txt"
                ));

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_js_eq(
                    &parts,
                    snapshot(include_str!(
                        "snapshots/should_stream_text_deltas_with_reasoning.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should stream text deltas with encrypted reasoning
            #[tokio::test]
            async fn should_stream_text_deltas_with_encrypted_reasoning() {
                let fetch = chunks_fixture_fetch(include_str!(
                    "fixtures/xai-text-with-reasoning-streaming-store-false.1.chunks.txt"
                ));

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_js_eq(
                    &parts,
                    snapshot(include_str!(
                        "snapshots/should_stream_text_deltas_with_encrypted_reasoning.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should include encrypted content in reasoning-end providerMetadata
            #[tokio::test]
            async fn should_include_encrypted_content_in_reasoning_end_provider_metadata() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "in_progress",
                            "summary": []
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.reasoning_summary_part.added",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "summary_index": 0,
                        "part": { "type": "summary_text", "text": "" }
                    }),
                    json!({
                        "type": "response.reasoning_summary_text.delta",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "summary_index": 0,
                        "delta": "Analyzing..."
                    }),
                    json!({
                        "type": "response.reasoning_summary_text.done",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "summary_index": 0,
                        "text": "Analyzing..."
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "completed",
                            "summary": [{ "type": "summary_text", "text": "Analyzing..." }],
                            "encrypted_content": "encrypted_data_abc123"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "message",
                            "id": "msg_789",
                            "role": "assistant",
                            "status": "in_progress",
                            "content": []
                        },
                        "output_index": 1
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": "msg_789",
                        "output_index": 1,
                        "content_index": 0,
                        "delta": "Result."
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 20 }
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_js_eq(
                    &first_part(&parts, "reasoning-end"),
                    json!({
                        "id": "reasoning-rs_456",
                        "providerMetadata": {
                            "xai": {
                                "itemId": "rs_456",
                                "reasoningEncryptedContent": "encrypted_data_abc123"
                            }
                        },
                        "type": "reasoning-end"
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should emit reasoning-start before reasoning-end when reasoning_summary_part.added is not sent
            #[tokio::test]
            async fn should_emit_reasoning_start_before_end_without_summary_part_added() {
                // Encrypted reasoning without any reasoning summary text: no
                // `reasoning_summary_part.added` event.
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "in_progress",
                            "summary": []
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "completed",
                            "summary": [],
                            "encrypted_content": "encrypted_reasoning_content_xyz"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "message",
                            "id": "msg_789",
                            "role": "assistant",
                            "status": "in_progress",
                            "content": []
                        },
                        "output_index": 1
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": "msg_789",
                        "output_index": 1,
                        "content_index": 0,
                        "delta": "The answer is 42."
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 20 }
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_js_eq(
                    &first_part(&parts, "reasoning-start"),
                    json!({
                        "id": "reasoning-rs_456",
                        "providerMetadata": { "xai": { "itemId": "rs_456" } },
                        "type": "reasoning-start"
                    }),
                );
                assert_js_eq(
                    &first_part(&parts, "reasoning-end"),
                    json!({
                        "id": "reasoning-rs_456",
                        "providerMetadata": {
                            "xai": {
                                "itemId": "rs_456",
                                "reasoningEncryptedContent": "encrypted_reasoning_content_xyz"
                            }
                        },
                        "type": "reasoning-end"
                    }),
                );
                assert!(index_of(&parts, "reasoning-start") < index_of(&parts, "reasoning-end"));
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should stream reasoning text deltas (response.reasoning_text.delta)
            #[tokio::test]
            async fn should_stream_reasoning_text_deltas_response_reasoning_text_delta() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-code-fast-1",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "in_progress",
                            "summary": []
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.reasoning_text.delta",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": "First"
                    }),
                    json!({
                        "type": "response.reasoning_text.delta",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": ", analyze the question."
                    }),
                    json!({
                        "type": "response.reasoning_text.done",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "content_index": 0,
                        "text": "First, analyze the question."
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "completed",
                            "summary": [
                                { "type": "summary_text", "text": "First, analyze the question." }
                            ]
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "message",
                            "id": "msg_789",
                            "role": "assistant",
                            "status": "in_progress",
                            "content": []
                        },
                        "output_index": 1
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": "msg_789",
                        "output_index": 1,
                        "content_index": 0,
                        "delta": "The answer."
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-code-fast-1",
                            "status": "completed",
                            "output": [],
                            "usage": {
                                "input_tokens": 10,
                                "output_tokens": 20,
                                "output_tokens_details": { "reasoning_tokens": 15 }
                            }
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, "grok-code-fast-1", &CallOptions::new(test_prompt())).await;

                assert_js_eq(
                    &first_part(&parts, "reasoning-start"),
                    json!({
                        "id": "reasoning-rs_456",
                        "providerMetadata": { "xai": { "itemId": "rs_456" } },
                        "type": "reasoning-start"
                    }),
                );
                let deltas = parts_of_type(&parts, "reasoning-delta");
                assert_eq!(deltas.len(), 2);
                assert_eq!(deltas[0]["delta"], "First");
                assert_eq!(deltas[1]["delta"], ", analyze the question.");
                assert_js_eq(
                    &first_part(&parts, "reasoning-end"),
                    json!({
                        "id": "reasoning-rs_456",
                        "providerMetadata": { "xai": { "itemId": "rs_456" } },
                        "type": "reasoning-end"
                    }),
                );

                // reasoning-start < reasoning-deltas < reasoning-end < text
                let start = index_of(&parts, "reasoning-start");
                let first_delta = index_of(&parts, "reasoning-delta");
                let end = index_of(&parts, "reasoning-end");
                let text = index_of(&parts, "text-delta");
                assert!(start < first_delta);
                assert!(first_delta < end);
                assert!(end < text);
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should emit only one reasoning-start when multiple reasoning_summary_part.added events share an item_id
            #[tokio::test]
            async fn should_emit_one_reasoning_start_for_shared_item_id_summary_parts() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4.3",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "in_progress",
                            "summary": []
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.reasoning_summary_part.added",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "summary_index": 0,
                        "part": { "type": "summary_text", "text": "" }
                    }),
                    json!({
                        "type": "response.reasoning_summary_text.delta",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "summary_index": 0,
                        "delta": "First summary part."
                    }),
                    json!({
                        "type": "response.reasoning_summary_part.added",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "summary_index": 1,
                        "part": { "type": "summary_text", "text": "" }
                    }),
                    json!({
                        "type": "response.reasoning_summary_text.delta",
                        "item_id": "rs_456",
                        "output_index": 0,
                        "summary_index": 1,
                        "delta": " Second summary part."
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "reasoning",
                            "id": "rs_456",
                            "status": "completed",
                            "summary": [
                                { "type": "summary_text", "text": "First summary part." },
                                { "type": "summary_text", "text": " Second summary part." }
                            ]
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4.3",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 20 }
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                let starts = parts_of_type(&parts, "reasoning-start");
                let ends = parts_of_type(&parts, "reasoning-end");
                assert_eq!(starts.len(), 1);
                assert_eq!(ends.len(), 1);
                assert_matches_object(
                    &starts[0],
                    json!({
                        "type": "reasoning-start",
                        "id": "reasoning-rs_456",
                        "providerMetadata": { "xai": { "itemId": "rs_456" } }
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should stream x_search tool call
            #[tokio::test]
            async fn should_stream_x_search_tool_call() {
                let fetch =
                    chunks_fixture_fetch(include_str!("fixtures/xai-x-search-tool.chunks.txt"));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool("xai.x_search", "x_search", json!({}))]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_js_eq(
                    &parts,
                    snapshot(include_str!(
                        "snapshots/should_stream_x_search_tool_call.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › text streaming › should not emit duplicate text-delta from response.output_item.done after streaming
            #[tokio::test]
            async fn should_not_emit_duplicate_text_delta_after_output_item_done() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "created_at": 1700000000,
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "message",
                            "id": "msg_123",
                            "status": "in_progress",
                            "role": "assistant",
                            "content": []
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": "msg_123",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": "Hello"
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": "msg_123",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": " "
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": "msg_123",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": "world"
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "message",
                            "id": "msg_123",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "Hello world",
                                    "annotations": []
                                }
                            ]
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                // Only the three streamed deltas, not the full text again.
                let deltas = parts_of_type(&parts, "text-delta");
                assert_eq!(deltas.len(), 3);
                let texts: Vec<&Value> = deltas.iter().map(|delta| &delta["delta"]).collect();
                assert_js_eq(&json!(texts), json!(["Hello", " ", "world"]));
                assert!(deltas.iter().all(|delta| delta["delta"] != "Hello world"));
            }
        }

        mod tool_call_streaming {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should stream web_search tool calls
            #[tokio::test]
            async fn should_stream_web_search_tool_calls() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "web_search_call",
                            "id": "ws_123",
                            "name": "web_search",
                            "arguments": "{\"query\":\"test\"}",
                            "call_id": "",
                            "status": "completed"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "ws_123",
                        "toolName": "web_search",
                        "input": "{\"query\":\"test\"}",
                        "providerExecuted": true
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should emit a tool result when provider-executed tool calls finish
            #[tokio::test]
            async fn should_emit_a_tool_result_when_provider_executed_tool_calls_finish() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "web_search_call",
                            "id": "ws_123",
                            "name": "web_search",
                            "arguments": "{\"query\":\"test\"}",
                            "call_id": "",
                            "status": "in_progress"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "web_search_call",
                            "id": "ws_123",
                            "name": "web_search",
                            "arguments": "{\"query\":\"test\"}",
                            "call_id": "",
                            "status": "completed"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "ws_123",
                        "toolName": "web_search",
                        "input": "{\"query\":\"test\"}",
                        "providerExecuted": true
                    }),
                );
                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_123",
                        "toolName": "web_search",
                        "result": {}
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should forward web_search_call action and sources to tool-result
            #[tokio::test]
            async fn should_forward_web_search_call_action_and_sources_to_tool_result() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "web_search_call",
                            "id": "ws_action",
                            "name": "web_search",
                            "call_id": "",
                            "status": "in_progress"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "web_search_call",
                            "id": "ws_action",
                            "name": "web_search",
                            "call_id": "",
                            "status": "completed",
                            "action": {
                                "type": "search",
                                "query": "latest AI news",
                                "sources": [{ "type": "url", "url": "https://example.com/a" }]
                            }
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_action",
                        "toolName": "web_search",
                        "result": {
                            "action": { "type": "search", "query": "latest AI news" },
                            "sources": [{ "type": "url", "url": "https://example.com/a" }]
                        }
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should map open_page and find_in_page actions in stream tool-results
            #[tokio::test]
            async fn should_map_open_page_and_find_in_page_actions_in_stream_tool_results() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "web_search_call",
                            "id": "ws_open",
                            "name": "web_search",
                            "call_id": "",
                            "status": "completed",
                            "action": { "type": "open_page", "url": "https://example.com/a" }
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "web_search_call",
                            "id": "ws_find",
                            "name": "web_search",
                            "call_id": "",
                            "status": "completed",
                            "action": {
                                "type": "find_in_page",
                                "url": "https://example.com/a",
                                "pattern": "climate"
                            }
                        },
                        "output_index": 1
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_open",
                        "toolName": "web_search",
                        "result": {
                            "action": { "type": "openPage", "url": "https://example.com/a" }
                        }
                    }),
                );
                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ws_find",
                        "toolName": "web_search",
                        "result": {
                            "action": {
                                "type": "findInPage",
                                "url": "https://example.com/a",
                                "pattern": "climate"
                            }
                        }
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should stream image_generation tool calls from chunks fixture
            #[tokio::test]
            async fn should_stream_image_generation_tool_calls_from_chunks_fixture() {
                let fetch = chunks_fixture_fetch(include_str!(
                    "fixtures/xai-image-generation-tool.1.chunks.txt"
                ));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_js_eq(
                    &parts,
                    Value::Array(vec![
                        json!({ "type": "stream-start", "warnings": [] }),
                        json!({
                            "id": "00000000-0000-4000-8000-000000000002",
                            "modelId": "grok-4.5",
                            "timestamp": "1970-01-01T00:00:00.000Z",
                            "type": "response-metadata"
                        }),
                        json!({
                            "id": "ig_00000000-0000-4000-8000-000000000002_call-00000000-0000-4000-8000-00000000000b-0",
                            "toolName": "image_generation",
                            "type": "tool-input-start"
                        }),
                        json!({
                            "delta": "{}",
                            "id": "ig_00000000-0000-4000-8000-000000000002_call-00000000-0000-4000-8000-00000000000b-0",
                            "type": "tool-input-delta"
                        }),
                        json!({
                            "id": "ig_00000000-0000-4000-8000-000000000002_call-00000000-0000-4000-8000-00000000000b-0",
                            "type": "tool-input-end"
                        }),
                        json!({
                            "input": "{}",
                            "providerExecuted": true,
                            "toolCallId": "ig_00000000-0000-4000-8000-000000000002_call-00000000-0000-4000-8000-00000000000b-0",
                            "toolName": "image_generation",
                            "type": "tool-call"
                        }),
                        json!({
                            "result": {
                                "prompt": "A delicate origami paper boat sailing gently on a tranquil pond covered with floating green lily pads and autumn leaves, soft sunlight filtering through trees, peaceful nature scene, highly detailed, realistic style",
                                "result": "ZmFrZUltYWdlQmFzZTY0RGF0YQ=="
                            },
                            "toolCallId": "ig_00000000-0000-4000-8000-000000000002_call-00000000-0000-4000-8000-00000000000b-0",
                            "toolName": "image_generation",
                            "type": "tool-result"
                        }),
                        json!({ "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-start" }),
                        json!({ "delta": "Here's", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " a", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " serene", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " image", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " of", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " a", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " paper", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " boat", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " sailing", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " on", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " a", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " leaf", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": "-", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": "covered", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": " pond", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "delta": ":", "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-delta" }),
                        json!({ "id": "text-msg_00000000-0000-4000-8000-000000000002", "type": "text-end" }),
                        json!({
                            "finishReason": { "raw": "completed", "unified": "stop" },
                            "providerMetadata": {
                                "xai": { "costInUsdTicks": 123456, "serviceTier": "default" }
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": { "cacheRead": 20, "noCache": 80, "total": 100 },
                                "outputTokens": { "reasoning": 30, "text": 20, "total": 50 },
                                "raw": {
                                    "context_details": { "input_tokens": 80, "output_tokens": 20 },
                                    "cost_in_usd_ticks": 123456,
                                    "input_tokens": 100,
                                    "input_tokens_details": { "cached_tokens": 20 },
                                    "num_server_side_tools_used": 1,
                                    "num_sources_used": 0,
                                    "output_tokens": 50,
                                    "output_tokens_details": { "reasoning_tokens": 30 },
                                    "server_side_tool_usage_details": {
                                        "code_interpreter_calls": 0,
                                        "document_search_calls": 0,
                                        "file_search_calls": 0,
                                        "image_generation_calls": 1,
                                        "mcp_calls": 0,
                                        "web_search_calls": 0,
                                        "x_search_calls": 0
                                    },
                                    "total_tokens": 150
                                }
                            }
                        }),
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should stream image_generation tool calls with progress events
            #[tokio::test]
            async fn should_stream_image_generation_tool_calls_with_progress_events() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.image_generation_call.in_progress",
                        "item_id": "ig_123",
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.image_generation_call.generating",
                        "item_id": "ig_123",
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.image_generation_call.completed",
                        "item_id": "ig_123",
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "image_generation_call",
                            "id": "ig_123",
                            "status": "completed",
                            "prompt": "An origami fox in a paper forest",
                            "result": "base64imagedata"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": "msg_123",
                        "output_index": 1,
                        "content_index": 0,
                        "delta": "Here is your origami fox."
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_js_eq(
                    &parts,
                    Value::Array(vec![
                        json!({ "type": "stream-start", "warnings": [] }),
                        json!({
                            "id": "resp_123",
                            "modelId": "grok-4-fast-non-reasoning",
                            "type": "response-metadata"
                        }),
                        json!({ "id": "ig_123", "toolName": "image_generation", "type": "tool-input-start" }),
                        json!({ "delta": "{}", "id": "ig_123", "type": "tool-input-delta" }),
                        json!({ "id": "ig_123", "type": "tool-input-end" }),
                        json!({
                            "input": "{}",
                            "providerExecuted": true,
                            "toolCallId": "ig_123",
                            "toolName": "image_generation",
                            "type": "tool-call"
                        }),
                        json!({
                            "result": {
                                "prompt": "An origami fox in a paper forest",
                                "result": "base64imagedata"
                            },
                            "toolCallId": "ig_123",
                            "toolName": "image_generation",
                            "type": "tool-result"
                        }),
                        json!({ "id": "text-msg_123", "type": "text-start" }),
                        json!({
                            "delta": "Here is your origami fox.",
                            "id": "text-msg_123",
                            "type": "text-delta"
                        }),
                        json!({ "id": "text-msg_123", "type": "text-end" }),
                        json!({
                            "finishReason": { "raw": "completed", "unified": "stop" },
                            "type": "finish",
                            "usage": {
                                "inputTokens": { "cacheRead": 0, "noCache": 10, "total": 10 },
                                "outputTokens": { "reasoning": 0, "text": 5, "total": 5 },
                                "raw": { "input_tokens": 10, "output_tokens": 5 }
                            }
                        }),
                    ]),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should stream image_generation tool calls when only output items are emitted
            #[tokio::test]
            async fn should_stream_image_generation_tool_calls_when_only_output_items_are_emitted()
            {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "image_generation_call",
                            "id": "ig_123",
                            "status": "in_progress"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "image_generation_call",
                            "id": "ig_123",
                            "status": "completed",
                            "prompt": "An origami fox in a paper forest",
                            "result": "base64imagedata"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({ "type": "tool-input-start", "id": "ig_123", "toolName": "image_generation" }),
                );
                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "ig_123",
                        "toolName": "image_generation",
                        "input": "{}",
                        "providerExecuted": true
                    }),
                );
                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ig_123",
                        "toolName": "image_generation",
                        "result": {
                            "result": "base64imagedata",
                            "prompt": "An origami fox in a paper forest"
                        }
                    }),
                );
                assert_eq!(parts_of_type(&parts, "tool-input-start").len(), 1);
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should stream an error tool result for failed image_generation calls
            #[tokio::test]
            async fn should_stream_an_error_tool_result_for_failed_image_generation_calls() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.image_generation_call.in_progress",
                        "item_id": "ig_123",
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "image_generation_call",
                            "id": "ig_123",
                            "status": "failed"
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.image_generation",
                    "image_generation",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "ig_123",
                        "toolName": "image_generation",
                        "input": "{}",
                        "providerExecuted": true
                    }),
                );
                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "ig_123",
                        "toolName": "image_generation",
                        "isError": true,
                        "result": "Image generation failed (status: failed)."
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should stream function tool call arguments
            #[tokio::test]
            async fn should_stream_function_tool_call_arguments() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 0,
                        "item": {
                            "type": "function_call",
                            "id": "fc_123",
                            "call_id": "call_123",
                            "name": "weather",
                            "arguments": "",
                            "status": "in_progress"
                        }
                    }),
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": "fc_123",
                        "output_index": 0,
                        "delta": "{\"location\""
                    }),
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": "fc_123",
                        "output_index": 0,
                        "delta": ":\"sf\"}"
                    }),
                    json!({
                        "type": "response.function_call_arguments.done",
                        "item_id": "fc_123",
                        "output_index": 0,
                        "arguments": "{\"location\":\"sf\"}"
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 0,
                        "item": {
                            "type": "function_call",
                            "id": "fc_123",
                            "call_id": "call_123",
                            "name": "weather",
                            "arguments": "{\"location\":\"sf\"}",
                            "status": "completed"
                        }
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![function_tool(
                    "weather",
                    "get weather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } }
                    }),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({ "type": "tool-input-start", "id": "call_123", "toolName": "weather" }),
                );
                assert_contains(
                    &parts,
                    json!({ "type": "tool-input-delta", "id": "call_123", "delta": "{\"location\"" }),
                );
                assert_contains(
                    &parts,
                    json!({ "type": "tool-input-delta", "id": "call_123", "delta": ":\"sf\"}" }),
                );
                assert_contains(
                    &parts,
                    json!({ "type": "tool-input-end", "id": "call_123" }),
                );
                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "call_123",
                        "toolName": "weather",
                        "input": "{\"location\":\"sf\"}"
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should return tool-calls finish reason when function_call is streamed
            #[tokio::test]
            async fn should_return_tool_calls_finish_reason_when_function_call_is_streamed() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 0,
                        "item": {
                            "type": "function_call",
                            "id": "fc_123",
                            "call_id": "call_123",
                            "name": "weather",
                            "arguments": "",
                            "status": "in_progress"
                        }
                    }),
                    json!({
                        "type": "response.function_call_arguments.done",
                        "item_id": "fc_123",
                        "output_index": 0,
                        "arguments": "{\"location\":\"sf\"}"
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 0,
                        "item": {
                            "type": "function_call",
                            "id": "fc_123",
                            "call_id": "call_123",
                            "name": "weather",
                            "arguments": "{\"location\":\"sf\"}",
                            "status": "completed"
                        }
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![function_tool(
                    "weather",
                    "get weather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } }
                    }),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_matches_object(
                    &first_part(&parts, "finish"),
                    json!({
                        "type": "finish",
                        "finishReason": { "unified": "tool-calls", "raw": "completed" }
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool call streaming › should stream file_search tool call and result
            #[tokio::test]
            async fn should_stream_file_search_tool_call_and_result() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "file_search_call",
                            "id": "fs_stream_123",
                            "status": "in_progress",
                            "queries": ["search query"],
                            "results": null
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.output_item.done",
                        "item": {
                            "type": "file_search_call",
                            "id": "fs_stream_123",
                            "status": "completed",
                            "queries": ["search query"],
                            "results": [
                                {
                                    "file_id": "file_abc",
                                    "filename": "doc.txt",
                                    "score": 0.9,
                                    "text": "Found text content"
                                }
                            ]
                        },
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.file_search",
                    "file_search",
                    json!({ "vectorStoreIds": ["collection_123"] }),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({ "type": "tool-input-start", "id": "fs_stream_123", "toolName": "file_search" }),
                );
                assert_contains(
                    &parts,
                    json!({ "type": "tool-input-delta", "id": "fs_stream_123", "delta": "" }),
                );
                assert_contains(
                    &parts,
                    json!({ "type": "tool-input-end", "id": "fs_stream_123" }),
                );
                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "fs_stream_123",
                        "toolName": "file_search",
                        "input": "",
                        "providerExecuted": true
                    }),
                );
                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-result",
                        "toolCallId": "fs_stream_123",
                        "toolName": "file_search",
                        "result": {
                            "queries": ["search query"],
                            "results": [
                                {
                                    "fileId": "file_abc",
                                    "filename": "doc.txt",
                                    "score": 0.9,
                                    "text": "Found text content"
                                }
                            ]
                        }
                    }),
                );
            }
        }

        mod tool_name_mapping_by_type_in_streaming {
            use super::*;

            /// Fixture of the streamed `output_item.added` for one server-side
            /// item with an empty `name`: `response.created`, the item, `response.done`.
            fn single_item_stream(item: Value) -> Arc<MockFetch> {
                stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_item.added",
                        "item": item,
                        "output_index": 0
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ])
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool name mapping by type in streaming › should map web_search_call type to web_search tool name when name is empty
            #[tokio::test]
            async fn should_map_web_search_call_type_to_web_search_tool_name_when_name_is_empty() {
                let fetch = single_item_stream(json!({
                    "type": "web_search_call",
                    "id": "ws_123",
                    "name": "",
                    "arguments": "{\"query\":\"test\"}",
                    "call_id": "",
                    "status": "completed"
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "web_search",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "ws_123",
                        "toolName": "web_search",
                        "input": "{\"query\":\"test\"}",
                        "providerExecuted": true
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool name mapping by type in streaming › should map x_search_call type to x_search tool name when name is empty
            #[tokio::test]
            async fn should_map_x_search_call_type_to_x_search_tool_name_when_name_is_empty() {
                let fetch = single_item_stream(json!({
                    "type": "x_search_call",
                    "id": "xs_123",
                    "name": "",
                    "arguments": "{\"query\":\"test\"}",
                    "call_id": "",
                    "status": "completed"
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool("xai.x_search", "x_search", json!({}))]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "xs_123",
                        "toolName": "x_search",
                        "input": "{\"query\":\"test\"}",
                        "providerExecuted": true
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool name mapping by type in streaming › should map code_interpreter_call type to code_execution tool name when name is empty
            #[tokio::test]
            async fn should_map_code_interpreter_call_type_to_code_execution_when_name_empty() {
                let fetch = single_item_stream(json!({
                    "type": "code_interpreter_call",
                    "id": "ci_123",
                    "name": "",
                    "arguments": "{}",
                    "call_id": "",
                    "status": "completed"
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.code_execution",
                    "code_execution",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "ci_123",
                        "toolName": "code_execution",
                        "input": "{}",
                        "providerExecuted": true
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool name mapping by type in streaming › should map code_execution_call type to code_execution tool name when name is empty
            #[tokio::test]
            async fn should_map_code_execution_call_type_to_code_execution_when_name_empty() {
                let fetch = single_item_stream(json!({
                    "type": "code_execution_call",
                    "id": "ce_123",
                    "name": "",
                    "arguments": "{}",
                    "call_id": "",
                    "status": "completed"
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.code_execution",
                    "code_execution",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "ce_123",
                        "toolName": "code_execution",
                        "input": "{}",
                        "providerExecuted": true
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › tool name mapping by type in streaming › should use custom tool name from provider tool when type matches
            #[tokio::test]
            async fn should_use_custom_tool_name_from_provider_tool_when_type_matches() {
                let fetch = single_item_stream(json!({
                    "type": "web_search_call",
                    "id": "ws_123",
                    "name": "",
                    "arguments": "{}",
                    "call_id": "",
                    "status": "completed"
                }));

                let mut options = CallOptions::new(test_prompt());
                options.tools = Some(vec![provider_tool(
                    "xai.web_search",
                    "my_custom_search",
                    json!({}),
                )]);
                let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &options).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "tool-call",
                        "toolCallId": "ws_123",
                        "toolName": "my_custom_search",
                        "input": "{}",
                        "providerExecuted": true
                    }),
                );
            }
        }

        mod citation_streaming {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › citation streaming › should stream citations as sources
            #[tokio::test]
            async fn should_stream_citations_as_sources() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_text.annotation.added",
                        "item_id": "msg_123",
                        "output_index": 0,
                        "content_index": 0,
                        "annotation_index": 0,
                        "annotation": {
                            "type": "url_citation",
                            "url": "https://example.com",
                            "title": "example"
                        }
                    }),
                    json!({
                        "type": "response.done",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 }
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_contains(
                    &parts,
                    json!({
                        "type": "source",
                        "sourceType": "url",
                        "id": "id-0",
                        "url": "https://example.com",
                        "title": "example"
                    }),
                );
            }
        }

        mod cost_in_usd_ticks_streaming {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › cost_in_usd_ticks streaming › should expose cost_in_usd_ticks in finish providerMetadata
            #[tokio::test]
            async fn should_expose_cost_in_usd_ticks_in_finish_provider_metadata() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": "Hello"
                    }),
                    json!({
                        "type": "response.completed",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "completed",
                            "output": [
                                {
                                    "type": "message",
                                    "id": "msg_001",
                                    "role": "assistant",
                                    "status": "completed",
                                    "content": [{ "type": "output_text", "text": "Hello" }]
                                }
                            ],
                            "usage": {
                                "input_tokens": 10,
                                "output_tokens": 5,
                                "cost_in_usd_ticks": 113500
                            }
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_matches_object(
                    &first_part(&parts, "finish"),
                    json!({
                        "type": "finish",
                        "providerMetadata": { "xai": { "costInUsdTicks": 113500 } }
                    }),
                );
            }

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › cost_in_usd_ticks streaming › should expose echoed request identifiers in finish providerMetadata
            #[tokio::test]
            async fn should_expose_echoed_request_identifiers_in_finish_provider_metadata() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.completed",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "status": "completed",
                            "output": [],
                            "usage": { "input_tokens": 10, "output_tokens": 5 },
                            "prompt_cache_key": "conversation-123",
                            "safety_identifier": "hashed-user-123"
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_matches_object(
                    &first_part(&parts, "finish"),
                    json!({
                        "type": "finish",
                        "providerMetadata": {
                            "xai": {
                                "promptCacheKey": "conversation-123",
                                "safetyIdentifier": "hashed-user-123"
                            }
                        }
                    }),
                );
            }
        }

        mod missing_usage_handling {
            use super::*;

            /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › doStream › missing usage handling › should handle missing usage in streaming response
            #[tokio::test]
            async fn should_handle_missing_usage_in_streaming_response() {
                let fetch = stream_fetch(vec![
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "created_at": 1700000000,
                            "status": "in_progress",
                            "output": []
                        }
                    }),
                    json!({
                        "type": "response.output_text.delta",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": "Hello"
                    }),
                    // The response has no `usage` field.
                    json!({
                        "type": "response.completed",
                        "response": {
                            "id": "resp_123",
                            "object": "response",
                            "model": "grok-4-fast-non-reasoning",
                            "created_at": 1700000000,
                            "status": "completed",
                            "output": [
                                {
                                    "type": "message",
                                    "id": "msg_001",
                                    "role": "assistant",
                                    "status": "completed",
                                    "content": [{ "type": "output_text", "text": "Hello" }]
                                }
                            ]
                        }
                    }),
                ]);

                let parts =
                    run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

                assert_matches_object(
                    &first_part(&parts, "finish"),
                    json!({
                        "type": "finish",
                        "finishReason": { "unified": "stop", "raw": "completed" },
                        "usage": {
                            "inputTokens": { "total": 0, "noCache": 0, "cacheRead": 0, "cacheWrite": 0 },
                            "outputTokens": { "total": 0, "text": 0, "reasoning": 0 }
                        }
                    }),
                );
            }
        }
    }

    mod schema_validation {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › schema validation › should accept response.created with usage: null
        #[tokio::test]
        async fn should_accept_response_created_with_usage_null() {
            let fetch = stream_fetch(vec![
                json!({
                    "type": "response.created",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "created_at": 1700000000,
                        "status": "in_progress",
                        "output": [],
                        "usage": null
                    }
                }),
                json!({
                    "type": "response.output_item.added",
                    "item": {
                        "id": "msg_001",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "status": "in_progress"
                    },
                    "output_index": 0
                }),
                json!({
                    "type": "response.content_part.added",
                    "item_id": "msg_001",
                    "output_index": 0,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": "" }
                }),
                json!({
                    "type": "response.output_text.delta",
                    "item_id": "msg_001",
                    "output_index": 0,
                    "content_index": 0,
                    "delta": "Hello"
                }),
                json!({
                    "type": "response.completed",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "created_at": 1700000000,
                        "status": "completed",
                        "output": [
                            {
                                "id": "msg_001",
                                "type": "message",
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": "Hello" }],
                                "status": "completed"
                            }
                        ],
                        "usage": {
                            "input_tokens": 10,
                            "output_tokens": 5,
                            "total_tokens": 15
                        }
                    }
                }),
            ]);

            let parts =
                run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

            assert_matches_object(
                &first_part(&parts, "text-delta"),
                json!({ "type": "text-delta", "delta": "Hello" }),
            );
            assert_matches_object(&first_part(&parts, "finish"), json!({ "type": "finish" }));
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › schema validation › should accept response.in_progress with usage: null
        #[tokio::test]
        async fn should_accept_response_in_progress_with_usage_null() {
            let fetch = stream_fetch(vec![
                json!({
                    "type": "response.created",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "created_at": 1700000000,
                        "status": "in_progress",
                        "output": [],
                        "usage": null
                    }
                }),
                json!({
                    "type": "response.in_progress",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "created_at": 1700000000,
                        "status": "in_progress",
                        "output": [],
                        "usage": null
                    }
                }),
                json!({
                    "type": "response.output_item.added",
                    "item": {
                        "id": "msg_001",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "status": "in_progress"
                    },
                    "output_index": 0
                }),
                json!({
                    "type": "response.content_part.added",
                    "item_id": "msg_001",
                    "output_index": 0,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": "" }
                }),
                json!({
                    "type": "response.output_text.delta",
                    "item_id": "msg_001",
                    "output_index": 0,
                    "content_index": 0,
                    "delta": "Hi"
                }),
                json!({
                    "type": "response.completed",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "created_at": 1700000000,
                        "status": "completed",
                        "output": [
                            {
                                "id": "msg_001",
                                "type": "message",
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": "Hi" }],
                                "status": "completed"
                            }
                        ],
                        "usage": {
                            "input_tokens": 5,
                            "output_tokens": 1,
                            "total_tokens": 6
                        }
                    }
                }),
            ]);

            let parts =
                run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

            assert_matches_object(
                &first_part(&parts, "text-delta"),
                json!({ "type": "text-delta", "delta": "Hi" }),
            );
        }
    }

    mod response_incomplete_handling {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › response.incomplete handling › should set finish reason to length for max_output_tokens
        #[tokio::test]
        async fn should_set_finish_reason_to_length_for_max_output_tokens() {
            let fetch = stream_fetch(vec![
                json!({
                    "type": "response.created",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "output": []
                    }
                }),
                json!({
                    "type": "response.output_text.delta",
                    "item_id": "msg_456",
                    "output_index": 0,
                    "content_index": 0,
                    "delta": "partial output"
                }),
                json!({
                    "type": "response.incomplete",
                    "response": {
                        "incomplete_details": { "reason": "max_output_tokens" },
                        "usage": { "input_tokens": 100, "output_tokens": 4096 }
                    }
                }),
            ]);

            let parts =
                run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

            assert_matches_object(
                &first_part(&parts, "finish"),
                json!({
                    "type": "finish",
                    "finishReason": { "unified": "length", "raw": "max_output_tokens" },
                    "usage": {
                        "inputTokens": { "total": 100 },
                        "outputTokens": { "total": 4096 }
                    }
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › response.incomplete handling › should default to length when incomplete_details is missing
        #[tokio::test]
        async fn should_default_to_length_when_incomplete_details_is_missing() {
            let fetch = stream_fetch(vec![
                json!({
                    "type": "response.created",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "output": []
                    }
                }),
                json!({
                    "type": "response.incomplete",
                    "response": {
                        "usage": { "input_tokens": 50, "output_tokens": 200 }
                    }
                }),
            ]);

            let parts =
                run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

            assert_matches_object(
                &first_part(&parts, "finish"),
                json!({
                    "type": "finish",
                    "finishReason": { "unified": "other", "raw": "incomplete" }
                }),
            );
        }
    }

    mod response_failed_handling {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › response.failed handling › should set finish reason to error
        #[tokio::test]
        async fn should_set_finish_reason_to_error() {
            let response_failed_event = json!({
                "type": "response.failed",
                "response": {
                    "error": {
                        "code": "server_error",
                        "message": "Internal server error"
                    },
                    "usage": { "input_tokens": 50, "output_tokens": 0 }
                }
            });
            let fetch = stream_fetch(vec![
                json!({
                    "type": "response.created",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "output": []
                    }
                }),
                response_failed_event.clone(),
            ]);

            let parts =
                run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

            assert_matches_object(
                &first_part(&parts, "finish"),
                json!({
                    "type": "finish",
                    "finishReason": { "unified": "error", "raw": "error" },
                    "usage": {
                        "inputTokens": { "total": 50 },
                        "outputTokens": { "total": 0 }
                    }
                }),
            );
            assert_matches_object(
                &stream_provider_error_view(&first_part(&parts, "error")),
                json!({
                    "type": "error",
                    "error": {
                        "message": response_failed_event["response"]["error"]["message"],
                        "type": response_failed_event["type"],
                        "code": response_failed_event["response"]["error"]["code"],
                        "statusCode": 500,
                        "isRetryable": true,
                        "data": response_failed_event
                    }
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › response.failed handling › should classify insufficient quota as non-retryable
        #[tokio::test]
        async fn should_classify_insufficient_quota_as_non_retryable() {
            let response_failed_event = json!({
                "type": "response.failed",
                "response": {
                    "error": {
                        "code": "insufficient_quota",
                        "message": "You exceeded your current quota."
                    },
                    "usage": { "input_tokens": 50, "output_tokens": 0 }
                }
            });
            let fetch = stream_fetch(vec![
                json!({
                    "type": "response.created",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "output": []
                    }
                }),
                response_failed_event.clone(),
            ]);

            let parts =
                run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

            assert_matches_object(
                &stream_provider_error_view(&first_part(&parts, "error")),
                json!({
                    "type": "error",
                    "error": {
                        "message": response_failed_event["response"]["error"]["message"],
                        "type": response_failed_event["type"],
                        "code": response_failed_event["response"]["error"]["code"],
                        "statusCode": 429,
                        "isRetryable": false,
                        "data": response_failed_event
                    }
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › response.failed handling › should map incomplete_details reason on failed response
        #[tokio::test]
        async fn should_map_incomplete_details_reason_on_failed_response() {
            let fetch = stream_fetch(vec![
                json!({
                    "type": "response.created",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "output": []
                    }
                }),
                json!({
                    "type": "response.failed",
                    "response": {
                        "error": {
                            "code": "server_error",
                            "message": "response failed"
                        },
                        "incomplete_details": { "reason": "max_output_tokens" },
                        "usage": null
                    }
                }),
            ]);

            let parts =
                run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

            assert_matches_object(
                &first_part(&parts, "finish"),
                json!({
                    "type": "finish",
                    "finishReason": { "unified": "length", "raw": "max_output_tokens" }
                }),
            );
        }
    }

    mod error_event_handling {
        use super::*;

        /// upstream: @ai-sdk/xai src/responses/xai-responses-language-model.test.ts › XaiResponsesLanguageModel › error event handling › should emit error chunk for server error events
        #[tokio::test]
        async fn should_emit_error_chunk_for_server_error_events() {
            let error_event = json!({
                "type": "error",
                "code": "server_error",
                "message": "Internal server error",
                "param": null
            });
            let fetch = stream_fetch(vec![
                json!({
                    "type": "response.created",
                    "response": {
                        "id": "resp_123",
                        "object": "response",
                        "model": "grok-4-fast-non-reasoning",
                        "output": []
                    }
                }),
                error_event.clone(),
            ]);

            let parts =
                run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

            assert_matches_object(
                &stream_provider_error_view(&first_part(&parts, "error")),
                json!({
                    "type": "error",
                    "error": {
                        "message": error_event["message"],
                        "type": error_event["type"],
                        "code": error_event["code"],
                        "statusCode": 500,
                        "isRetryable": true,
                        "data": error_event
                    }
                }),
            );
        }
    }
}
