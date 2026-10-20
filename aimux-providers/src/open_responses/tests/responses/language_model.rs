//! Port of `@ai-sdk/open-responses@2.0.58`
//! `src/responses/open-responses-language-model.test.ts`.
//!
//! The upstream `createTestServer` is replaced by the scripted
//! [`MockFetch`] transport; `server.calls[n].requestBodyJson` is
//! `mock.seen()[n].json_body()`. Upstream builds `OpenResponsesLanguageModel`
//! directly with provider `lmstudio`; here the model comes from
//! `create_open_responses` with the provider name `lmstudio`, which gives the
//! same providerOptions key and the same request URL.
//!
//! The older end-to-end ports (previously `tests/open_responses_test.rs`)
//! follow, on the wiremock harness in `support.rs`.

use super::support::*;
use crate::open_responses::{
    OpenResponsesModel, OpenResponsesProviderSettings, create_open_responses,
};
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart,
    ToolPart, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice, ResponseFormat, Tool};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::shared::FileData;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, RawToolCall};
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::Arc;
use wiremock::MockServer;

/// Upstream `URL` is `https://localhost:1234/v1/responses`.
const BASE_URL: &str = "https://localhost:1234/v1";

/// An upstream `__fixtures__` file, unchanged.
macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("fixtures/", $name))
    };
}

/// An upstream `toMatchSnapshot()` value from `__snapshots__`.
macro_rules! snapshot {
    ($name:literal) => {
        serde_json::from_str::<Value>(include_str!(concat!("snapshots/", $name, ".json")))
            .expect("snapshot is JSON")
    };
}

/// The upstream `createModel(modelId)` (no extensions), answering the
/// requests with `responses` in order.
fn create_model(model_id: &str, responses: Vec<Canned>) -> (OpenResponsesModel, Arc<MockFetch>) {
    let mock = MockFetch::new(responses);
    let mut settings = OpenResponsesProviderSettings::new("lmstudio", BASE_URL);
    settings.fetch = Some(mock.transport());
    let provider = create_open_responses(settings).expect("valid settings");
    (provider.responses(model_id), mock)
}

/// The upstream `prepareJsonFixtureResponse(filename)`.
fn json_fixture(text: &str) -> Canned {
    Canned::json(&serde_json::from_str(text).expect("fixture is JSON"))
}

/// An SSE response of the given `data:` chunks (each already framed).
fn stream_chunks(chunks: Vec<String>) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: chunks.concat().into_bytes(),
    }
}

/// One `data: <json>\n\n` chunk.
fn data(event: &Value) -> String {
    format!("data: {event}\n\n")
}

/// The upstream `prepareChunksFixtureResponse(filename)`: every non-empty
/// line becomes a `data:` chunk, then `data: [DONE]`.
fn chunks_fixture(text: &str) -> Canned {
    let mut chunks: Vec<String> = text
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("data: {line}\n\n"))
        .collect();
    chunks.push("data: [DONE]\n\n".into());
    stream_chunks(chunks)
}

/// The upstream `prepareOutputResponse(output)`.
fn output_response(output: Value) -> Canned {
    Canned::json(&json!({
        "id": "resp_1",
        "object": "response",
        "created_at": 0,
        "model": "test-model",
        "status": "completed",
        "output": output,
        "usage": {
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0,
        },
    }))
}

/// The upstream `TEST_PROMPT`.
fn test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

/// Call options written in the AI SDK's JSON shape.
fn call(options: Value) -> CallOptions {
    serde_json::from_value(options).expect("valid call options")
}

/// `result.content` as JSON.
fn content_json(result: &GenerateResult) -> Value {
    serde_json::to_value(&result.content).unwrap()
}

/// `first.content as AssistantContent`: upstream feeds the generated parts
/// back as prompt parts, and its converter reads `providerOptions ??
/// providerMetadata`. The Rust prompt part has only `providerOptions`, so
/// the metadata moves there.
fn as_assistant_content(content: &Value) -> Value {
    Value::Array(
        content
            .as_array()
            .expect("content array")
            .iter()
            .map(|part| {
                let mut part = part.clone();
                if let Some(metadata) = part.as_object_mut().unwrap().remove("providerMetadata") {
                    part["providerOptions"] = metadata;
                }
                part
            })
            .collect(),
    )
}

/// `convertReadableStreamToArray(result.stream)`; a stream error fails the
/// test as the rejected promise does upstream.
async fn collect_parts(result: StreamResult) -> Vec<StreamPart> {
    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(part) = stream.next().await {
        parts.push(part.unwrap_or_else(|error| panic!("stream failed: {error}")));
    }
    parts
}

/// Stream parts as JSON.
fn parts_json(parts: &[StreamPart]) -> Vec<Value> {
    parts
        .iter()
        .map(|part| serde_json::to_value(part).unwrap())
        .collect()
}

/// vitest `toMatchObject`: objects match on the expected keys, arrays
/// element by element with the same length.
fn matches_object(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected.iter().all(|(key, value)| {
            actual
                .get(key)
                .is_some_and(|actual| matches_object(actual, value))
        }),
        (Value::Array(actual), Value::Array(expected)) => {
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| matches_object(actual, expected))
        }
        _ => actual == expected,
    }
}

fn assert_match_object(actual: &Value, expected: &Value) {
    assert!(
        matches_object(actual, expected),
        "expected {actual:#} to match object {expected:#}"
    );
}

/// vitest `toContainEqual`.
fn assert_contains_equal(parts: &[Value], expected: &Value) {
    assert!(
        parts.contains(expected),
        "expected {expected:#} in {:#}",
        Value::Array(parts.to_vec())
    );
}

/// The paths where `actual` and `expected` differ, for a readable failure
/// of a large snapshot comparison.
fn json_diff(actual: &Value, expected: &Value, path: &str, out: &mut Vec<String>) {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            for (key, value) in expected {
                match actual.get(key) {
                    Some(actual) => json_diff(actual, value, &format!("{path}.{key}"), out),
                    None => out.push(format!("{path}.{key}: missing, expected {value}")),
                }
            }
            for (key, value) in actual {
                if !expected.contains_key(key) {
                    out.push(format!("{path}.{key}: unexpected {value}"));
                }
            }
        }
        (Value::Array(actual_items), Value::Array(expected_items)) => {
            if actual_items.len() != expected_items.len() {
                out.push(format!(
                    "{path}: length {} != expected {}",
                    actual_items.len(),
                    expected_items.len()
                ));
            }
            for (index, (actual, expected)) in actual_items.iter().zip(expected_items).enumerate() {
                json_diff(actual, expected, &format!("{path}[{index}]"), out);
            }
        }
        _ if actual != expected => out.push(format!("{path}: {actual} != expected {expected}")),
        _ => {}
    }
}

/// `toEqual` / `toMatchSnapshot` of a large value, reporting the differing
/// paths.
fn assert_json_eq(actual: &Value, expected: &Value) {
    let mut diff = Vec::new();
    json_diff(actual, expected, "$", &mut diff);
    assert!(diff.is_empty(), "values differ:\n{}", diff.join("\n"));
}

mod open_responses_language_model {
    use super::*;

    mod do_generate {
        use super::*;

        mod basic_generation {
            use super::*;

            /// The `beforeEach`: `createModel().doGenerate({ prompt: TEST_PROMPT })`.
            async fn generate() -> GenerateResult {
                let (model, _mock) = create_model(
                    "gemma-7b-it",
                    vec![json_fixture(fixture!("lmstudio-basic.1.json"))],
                );
                model
                    .do_generate(&call(json!({ "prompt": test_prompt() })))
                    .await
                    .unwrap()
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › basic generation › should produce correct content
            #[tokio::test]
            async fn should_produce_correct_content() {
                let result = generate().await;
                assert_eq!(content_json(&result), snapshot!("basic_generation_content"));
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › basic generation › should extract usage correctly
            #[tokio::test]
            async fn should_extract_usage_correctly() {
                let result = generate().await;
                assert_eq!(
                    serde_json::to_value(&result.usage).unwrap(),
                    snapshot!("basic_generation_usage")
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › basic generation › should send correct request body
            #[tokio::test]
            async fn basic_generation_request_body() {
                let server = MockServer::start().await;
                mock_json(&server, lmstudio_basic_json()).await;

                let model = make_model(&server, "gemma-7b-it");
                model
                    .do_generate(&default_options(wiremock_test_prompt()))
                    .await
                    .unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(
                    body,
                    json!({
                        "model": "gemma-7b-it",
                        "input": [{
                            "type": "message",
                            "role": "user",
                            "content": [{"type": "input_text", "text": "Hello"}]
                        }]
                    })
                );
            }
        }

        mod manual_history_replay {
            use super::*;

            /// Generates from `output`, then replays `first.content` as an
            /// assistant message; returns the first result and the second
            /// request's `input`.
            async fn generate_and_replay(output: Value) -> (Value, Value) {
                let (model, mock) = create_model(
                    "gemma-7b-it",
                    vec![output_response(output.clone()), output_response(output)],
                );
                let first = model
                    .do_generate(&call(json!({ "prompt": test_prompt() })))
                    .await
                    .unwrap();
                let content = content_json(&first);
                model
                    .do_generate(&call(json!({
                        "prompt": [{
                            "role": "assistant",
                            "content": as_assistant_content(&content),
                        }],
                    })))
                    .await
                    .unwrap();
                (content, mock.seen()[1].json_body()["input"].clone())
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › manual history replay › should preserve output item order and ids
            #[tokio::test]
            async fn should_preserve_output_item_order_and_ids() {
                let (_, input) = generate_and_replay(json!([
                    {
                        "id": "rs_1",
                        "type": "reasoning",
                        "status": "completed",
                        "summary": [],
                        "content": [{ "type": "reasoning_text", "text": "reasoning" }],
                    },
                    {
                        "id": "fc_1",
                        "type": "function_call",
                        "status": "completed",
                        "call_id": "call_1",
                        "name": "search",
                        "arguments": "{}",
                    },
                    {
                        "id": "msg_1",
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": [{
                            "type": "output_text",
                            "text": "answer after the call",
                            "annotations": [],
                        }],
                    },
                ]))
                .await;

                assert_eq!(
                    input,
                    json!([
                        {
                            "id": "rs_1",
                            "type": "reasoning",
                            "summary": [],
                            "content": [{ "type": "reasoning_text", "text": "reasoning" }],
                        },
                        {
                            "id": "fc_1",
                            "type": "function_call",
                            "call_id": "call_1",
                            "name": "search",
                            "arguments": "{}",
                        },
                        {
                            "id": "msg_1",
                            "type": "message",
                            "role": "assistant",
                            "content": [{
                                "type": "output_text",
                                "text": "answer after the call",
                            }],
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › manual history replay › should preserve summary and encrypted-only reasoning items
            #[tokio::test]
            async fn should_preserve_summary_and_encrypted_only_reasoning_items() {
                let (content, input) = generate_and_replay(json!([{
                    "id": "rs_2",
                    "type": "reasoning",
                    "status": "completed",
                    "summary": [{ "type": "summary_text", "text": "safe summary" }],
                    "encrypted_content": "opaque-provider-state",
                }]))
                .await;

                assert_eq!(
                    content,
                    json!([{
                        "type": "reasoning",
                        "text": "safe summary",
                        "providerMetadata": {
                            "lmstudio": {
                                "itemId": "rs_2",
                                "reasoningContent": null,
                                "reasoningSummary": [
                                    { "type": "summary_text", "text": "safe summary" },
                                ],
                                "reasoningEncryptedContent": "opaque-provider-state",
                            },
                        },
                    }])
                );
                assert_eq!(
                    input,
                    json!([{
                        "id": "rs_2",
                        "type": "reasoning",
                        "summary": [{ "type": "summary_text", "text": "safe summary" }],
                        "encrypted_content": "opaque-provider-state",
                    }])
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › manual history replay › should preserve output text annotations
            #[tokio::test]
            async fn should_preserve_output_text_annotations() {
                let annotation = json!({
                    "type": "url_citation",
                    "start_index": 0,
                    "end_index": 7,
                    "url": "https://example.com/source",
                    "title": "Example source",
                });
                let (content, input) = generate_and_replay(json!([{
                    "id": "msg_annotated",
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{
                        "type": "output_text",
                        "text": "Sourced answer",
                        "annotations": [annotation],
                    }],
                }]))
                .await;

                assert_eq!(
                    content,
                    json!([{
                        "type": "text",
                        "text": "Sourced answer",
                        "providerMetadata": {
                            "lmstudio": {
                                "itemId": "msg_annotated",
                                "annotations": [annotation],
                            },
                        },
                    }])
                );
                assert_eq!(
                    input,
                    json!([{
                        "id": "msg_annotated",
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": "Sourced answer",
                            "annotations": [annotation],
                        }],
                    }])
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › manual history replay › should preserve reasoning content part boundaries
            #[tokio::test]
            async fn should_preserve_reasoning_content_part_boundaries() {
                let (content, input) = generate_and_replay(json!([{
                    "id": "rs_multiple",
                    "type": "reasoning",
                    "status": "completed",
                    "summary": [],
                    "content": [
                        { "type": "reasoning_text", "text": "First thought. " },
                        { "type": "reasoning_text", "text": "Second thought." },
                    ],
                }]))
                .await;

                let content = content.as_array().unwrap();
                assert_eq!(content.len(), 2);
                assert_eq!(
                    content.iter().map(|part| &part["type"]).collect::<Vec<_>>(),
                    ["reasoning", "reasoning"]
                );
                assert_eq!(
                    content
                        .iter()
                        .map(|part| part["text"].as_str())
                        .collect::<Vec<_>>(),
                    [Some("First thought. "), Some("Second thought.")]
                );
                assert_eq!(
                    input,
                    json!([{
                        "id": "rs_multiple",
                        "type": "reasoning",
                        "summary": [],
                        "content": [
                            { "type": "reasoning_text", "text": "First thought. " },
                            { "type": "reasoning_text", "text": "Second thought." },
                        ],
                    }])
                );
            }
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › should send schema-less JSON as a JSON object format
        #[tokio::test]
        async fn should_send_schema_less_json_as_a_json_object_format() {
            let (model, mock) = create_model(
                "gemma-7b-it",
                vec![json_fixture(fixture!("lmstudio-basic.1.json"))],
            );
            model
                .do_generate(&call(json!({
                    "prompt": test_prompt(),
                    "responseFormat": { "type": "json" },
                })))
                .await
                .unwrap();

            assert_match_object(
                &mock.seen()[0].json_body(),
                &json!({ "text": { "format": { "type": "json_object" } } }),
            );
        }

        mod tools {
            use super::*;

            /// The `beforeEach`: two function tools and two provider tools.
            async fn generate() -> (GenerateResult, Arc<MockFetch>) {
                let (model, mock) = create_model(
                    "gemma-7b-it",
                    vec![json_fixture(fixture!("lmstudio-basic.1.json"))],
                );
                let result = model
                    .do_generate(&call(json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "function",
                                "name": "get_weather",
                                "description": "Get the current weather for a location",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "location": {
                                            "type": "string",
                                            "description": "The city and state",
                                        },
                                    },
                                    "required": ["location"],
                                },
                            },
                            {
                                "type": "function",
                                "name": "search",
                                "description": "Search for information",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": { "query": { "type": "string" } },
                                    "required": ["query"],
                                },
                                "strict": true,
                            },
                            {
                                "type": "provider",
                                "id": "openai.web_search",
                                "name": "web_search",
                                "args": {},
                            },
                            {
                                "type": "provider",
                                "id": "openai.file_search",
                                "name": "file_search",
                                "args": { "vectorStoreIds": ["vs_123"] },
                            },
                        ],
                    })))
                    .await
                    .unwrap();
                (result, mock)
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tools › should send correct request body with tools
            #[tokio::test]
            async fn should_send_correct_request_body_with_tools() {
                let (_, mock) = generate().await;
                assert_eq!(mock.seen()[0].json_body(), snapshot!("tools_request_body"));
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tools › should warn for each unsupported provider-defined tool
            #[tokio::test]
            async fn should_warn_for_each_unsupported_provider_defined_tool() {
                let (result, _) = generate().await;
                assert_eq!(
                    serde_json::to_value(&result.warnings).unwrap(),
                    json!([
                        {
                            "feature": "provider-defined tool openai.web_search",
                            "type": "unsupported",
                        },
                        {
                            "feature": "provider-defined tool openai.file_search",
                            "type": "unsupported",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tools › should omit a selected unregistered provider tool choice
            #[tokio::test]
            async fn should_omit_a_selected_unregistered_provider_tool_choice() {
                let (model, mock) = create_model(
                    "gemma-7b-it",
                    vec![json_fixture(fixture!("lmstudio-basic.1.json"))],
                );
                let result = model
                    .do_generate(&call(json!({
                        "prompt": test_prompt(),
                        "tools": [{
                            "type": "provider",
                            "id": "acme.unregistered",
                            "name": "unregistered",
                            "args": {},
                        }],
                        "toolChoice": { "type": "tool", "toolName": "unregistered" },
                    })))
                    .await
                    .unwrap();

                let request_body = mock.seen().last().unwrap().json_body();
                assert_eq!(request_body.get("tools"), None);
                assert_eq!(request_body.get("tool_choice"), None);
                assert_eq!(
                    serde_json::to_value(&result.warnings).unwrap(),
                    json!([{
                        "type": "unsupported",
                        "feature": "provider-defined tool acme.unregistered",
                    }])
                );
            }
        }

        mod provider_options_reasoning {
            use super::*;

            /// `createModel().doGenerate(options)` against the
            /// `lmstudio-basic.1` fixture; returns the result and the request
            /// body.
            async fn generate(options: Value) -> (GenerateResult, Value) {
                let (model, mock) = create_model(
                    "gemma-7b-it",
                    vec![json_fixture(fixture!("lmstudio-basic.1.json"))],
                );
                let result = model.do_generate(&call(options)).await.unwrap();
                let body = mock.seen()[0].json_body();
                (result, body)
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › providerOptions reasoning › should send a provider-native reasoning effort
            #[tokio::test]
            async fn should_send_a_provider_native_reasoning_effort() {
                let (_, body) = generate(json!({
                    "prompt": test_prompt(),
                    "providerOptions": { "lmstudio": { "reasoningEffort": "max" } },
                }))
                .await;
                assert_eq!(body.get("reasoning"), Some(&json!({ "effort": "max" })));
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › providerOptions reasoning › should prefer providerOptions reasoning effort over top-level reasoning
            #[tokio::test]
            async fn should_prefer_provider_options_reasoning_effort_over_top_level_reasoning() {
                let (result, body) = generate(json!({
                    "prompt": test_prompt(),
                    "reasoning": "low",
                    "providerOptions": {
                        "lmstudio": {
                            "reasoningEffort": "max",
                            "reasoningSummary": "detailed",
                        },
                    },
                }))
                .await;
                assert_eq!(
                    body.get("reasoning"),
                    Some(&json!({ "effort": "max", "summary": "detailed" }))
                );
                assert_eq!(result.warnings, []);
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › providerOptions reasoning › should send reasoning.summary via providerOptions
            #[tokio::test]
            async fn should_send_reasoning_summary_via_provider_options() {
                let (_, body) = generate(json!({
                    "prompt": test_prompt(),
                    "providerOptions": { "lmstudio": { "reasoningSummary": "detailed" } },
                }))
                .await;
                assert_eq!(
                    body,
                    json!({
                        "input": [{
                            "content": [{ "text": "Hello", "type": "input_text" }],
                            "role": "user",
                            "type": "message",
                        }],
                        "model": "gemma-7b-it",
                        "reasoning": { "summary": "detailed" },
                    })
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › providerOptions reasoning › should combine top-level reasoning effort with providerOptions summary
            #[tokio::test]
            async fn should_combine_top_level_reasoning_effort_with_provider_options_summary() {
                let (_, body) = generate(json!({
                    "prompt": test_prompt(),
                    "reasoning": "high",
                    "providerOptions": { "lmstudio": { "reasoningSummary": "auto" } },
                }))
                .await;
                assert_eq!(
                    body,
                    json!({
                        "input": [{
                            "content": [{ "text": "Hello", "type": "input_text" }],
                            "role": "user",
                            "type": "message",
                        }],
                        "model": "gemma-7b-it",
                        "reasoning": { "effort": "high", "summary": "auto" },
                    })
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › providerOptions reasoning › should send reasoning.summary concise via providerOptions
            #[tokio::test]
            async fn should_send_reasoning_summary_concise_via_provider_options() {
                let (_, body) = generate(json!({
                    "prompt": test_prompt(),
                    "providerOptions": { "lmstudio": { "reasoningSummary": "concise" } },
                }))
                .await;
                assert_eq!(
                    body,
                    json!({
                        "input": [{
                            "content": [{ "text": "Hello", "type": "input_text" }],
                            "role": "user",
                            "type": "message",
                        }],
                        "model": "gemma-7b-it",
                        "reasoning": { "summary": "concise" },
                    })
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › providerOptions reasoning › should not set reasoning when providerOptions has no reasoning fields
            #[tokio::test]
            async fn should_not_set_reasoning_when_provider_options_has_no_reasoning_fields() {
                let (_, body) = generate(json!({
                    "prompt": test_prompt(),
                    "providerOptions": { "lmstudio": {} },
                }))
                .await;
                assert_eq!(
                    body,
                    json!({
                        "input": [{
                            "content": [{ "text": "Hello", "type": "input_text" }],
                            "role": "user",
                            "type": "message",
                        }],
                        "model": "gemma-7b-it",
                    })
                );
            }
        }

        mod tool_call_parsing {
            use super::*;

            /// The `beforeEach`: the `weather` tool with `toolChoice: required`.
            async fn generate() -> GenerateResult {
                let (model, _mock) = create_model(
                    "gemma-7b-it",
                    vec![json_fixture(fixture!("lmstudio-tool-call.1.json"))],
                );
                model
                    .do_generate(&call(json!({
                        "prompt": test_prompt(),
                        "tools": [{
                            "type": "function",
                            "name": "weather",
                            "description": "Get the weather in a location",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "location": {
                                        "type": "string",
                                        "description": "The location to get the weather for",
                                    },
                                },
                                "required": ["location"],
                            },
                        }],
                        "toolChoice": { "type": "required" },
                    })))
                    .await
                    .unwrap()
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tool call parsing › should parse tool call from response
            #[tokio::test]
            async fn should_parse_tool_call_from_response() {
                let result = generate().await;
                assert_eq!(
                    content_json(&result),
                    snapshot!("tool_call_parsing_content")
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tool call parsing › should return tool-calls finish reason
            #[tokio::test]
            async fn should_return_tool_calls_finish_reason() {
                let result = generate().await;
                assert_eq!(
                    serde_json::to_value(&result.finish_reason).unwrap(),
                    json!({ "unified": "tool-calls" })
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tool call parsing › should extract usage correctly
            #[tokio::test]
            async fn should_extract_usage_correctly() {
                let result = generate().await;
                assert_eq!(
                    serde_json::to_value(&result.usage).unwrap(),
                    snapshot!("tool_call_parsing_usage")
                );
            }
        }

        mod tool_choice {
            use super::*;

            /// The upstream `TEST_TOOL`.
            fn test_tool() -> Value {
                json!({
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get the current weather",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "location": { "type": "string" } },
                        "required": ["location"],
                    },
                })
            }

            /// The request body sent for `toolChoice`.
            async fn request_body(tool_choice: Value) -> Value {
                let (model, mock) = create_model(
                    "gemma-7b-it",
                    vec![json_fixture(fixture!("lmstudio-basic.1.json"))],
                );
                model
                    .do_generate(&call(json!({
                        "prompt": test_prompt(),
                        "tools": [test_tool()],
                        "toolChoice": tool_choice,
                    })))
                    .await
                    .unwrap();
                mock.seen()[0].json_body()
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tool choice › should send tool_choice auto
            #[tokio::test]
            async fn should_send_tool_choice_auto() {
                assert_eq!(
                    request_body(json!({ "type": "auto" })).await,
                    snapshot!("tool_choice_auto")
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tool choice › should send tool_choice none
            #[tokio::test]
            async fn should_send_tool_choice_none() {
                assert_eq!(
                    request_body(json!({ "type": "none" })).await,
                    snapshot!("tool_choice_none")
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tool choice › should send tool_choice required
            #[tokio::test]
            async fn should_send_tool_choice_required() {
                assert_eq!(
                    request_body(json!({ "type": "required" })).await,
                    snapshot!("tool_choice_required")
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › tool choice › should send tool_choice with specific tool
            #[tokio::test]
            async fn should_send_tool_choice_with_specific_tool() {
                assert_eq!(
                    request_body(json!({ "type": "tool", "toolName": "get_weather" })).await,
                    snapshot!("tool_choice_specific_tool")
                );
            }
        }

        mod system_messages {
            use super::*;

            /// The request body sent for `prompt`.
            async fn request_body(prompt: Value) -> Value {
                let (model, mock) = create_model(
                    "gemma-7b-it",
                    vec![json_fixture(fixture!("lmstudio-basic.1.json"))],
                );
                model
                    .do_generate(&call(json!({ "prompt": prompt })))
                    .await
                    .unwrap();
                mock.seen()[0].json_body()
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › system messages › should send instructions from system message
            #[tokio::test]
            async fn should_send_instructions_from_system_message() {
                let body = request_body(json!([
                    { "role": "system", "content": "You are a helpful assistant." },
                    { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                ]))
                .await;
                assert_eq!(body, snapshot!("system_messages_instructions"));
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › system messages › should join multiple system messages with newlines
            #[tokio::test]
            async fn should_join_multiple_system_messages_with_newlines() {
                let body = request_body(json!([
                    { "role": "system", "content": "You are a helpful assistant." },
                    { "role": "system", "content": "Always be concise." },
                    { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                ]))
                .await;
                assert_eq!(body, snapshot!("system_messages_joined"));
            }
        }

        mod pdf_input_file {
            use super::*;

            /// The upstream `getPdfPrompt()`.
            fn pdf_prompt() -> Value {
                json!([{
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "What text does this PDF contain? Reply with just the text content, nothing else.",
                        },
                        {
                            "type": "file",
                            "data": {
                                "type": "url",
                                "url": "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf",
                            },
                            "mediaType": "application/pdf",
                        },
                    ],
                }])
            }

            /// The `beforeEach`.
            async fn generate() -> GenerateResult {
                let (model, _mock) = create_model(
                    "gpt-4.1-nano",
                    vec![json_fixture(fixture!("openai-pdf-input-file.1.json"))],
                );
                model
                    .do_generate(&call(json!({ "prompt": pdf_prompt() })))
                    .await
                    .unwrap()
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › pdf input file › should produce correct content
            #[tokio::test]
            async fn should_produce_correct_content() {
                let result = generate().await;
                assert_eq!(
                    content_json(&result),
                    json!([{
                        "providerMetadata": {
                            "lmstudio": {
                                "itemId": "msg_048edf44633e41ae0069d4fea0d1a08194af1e491c093df1d9",
                            },
                        },
                        "text": "Dummy PDF file",
                        "type": "text",
                    }])
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › pdf input file › should extract usage correctly
            #[tokio::test]
            async fn should_extract_usage_correctly() {
                let result = generate().await;
                assert_eq!(
                    serde_json::to_value(&result.usage).unwrap(),
                    json!({
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 44,
                            "total": 44,
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 4,
                            "total": 4,
                        },
                        "raw": {
                            "input_tokens": 44,
                            "input_tokens_details": { "cached_tokens": 0 },
                            "output_tokens": 4,
                            "output_tokens_details": { "reasoning_tokens": 0 },
                            "total_tokens": 48,
                        },
                    })
                );
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › pdf input file › should send input_file in request body
            #[tokio::test]
            async fn pdf_input_file_request_body() {
                let server = MockServer::start().await;
                mock_json(&server, openai_pdf_json()).await;

                let model = make_model(&server, "gpt-4.1-nano");
                let prompt = vec![LanguageModelMessage::User {
                    content: vec![
            UserPart::Text(TextPart {
            text: "What text does this PDF contain? Reply with just the text content, nothing else.".into(),
            provider_options: None
            }),
            UserPart::File(FilePart {
            data: FileData::Url {
            original_url: None,
            url: "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf".into()
            },
            media_type: "application/pdf".into(),
            filename: None,
            provider_options: None
            })
            ],
                    provider_options: None,
                }];
                model.do_generate(&default_options(prompt)).await.unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(
                    body,
                    json!({
                        "model": "gpt-4.1-nano",
                        "input": [{
                            "type": "message",
                            "role": "user",
                            "content": [
                                {"type": "input_text", "text": "What text does this PDF contain? Reply with just the text content, nothing else."},
                                {"type": "input_file", "file_url": "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf"}
                            ]
                        }]
                    })
                );
            }
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › should throw a descriptive error when the response has no output
        #[tokio::test]
        async fn throws_when_response_has_no_output() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "resp_no_output",
                    "created_at": 1741257730,
                    "model": "gemma-7b-it",
                    "status": "incomplete",
                    "incomplete_details": {"reason": "content_filter"},
                    "usage": {"input_tokens": 10, "output_tokens": 0}
                }),
            )
            .await;

            let model = make_model(&server, "gemma-7b-it");
            let result = model
                .do_generate(&default_options(wiremock_test_prompt()))
                .await;
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("Responses API returned no output (content_filter)"),
                "got: {msg}"
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › should surface response.error message before the no-output fallback
        #[tokio::test]
        async fn surfaces_response_error_before_no_output() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "resp_error",
                    "created_at": 1741257730,
                    "model": "gemma-7b-it",
                    "status": "failed",
                    "error": {
                        "code": "server_error",
                        "message": "The upstream provider failed to generate a response."
                    },
                    "usage": {"input_tokens": 10, "output_tokens": 0}
                }),
            )
            .await;

            let model = make_model(&server, "gemma-7b-it");
            let result = model
                .do_generate(&default_options(wiremock_test_prompt()))
                .await;
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("The upstream provider failed to generate a response."),
                "got: {msg}"
            );
        }

        mod request_parameters {
            use super::*;

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › request parameters › should send correct request body
            #[tokio::test]
            async fn request_parameters_request_body() {
                let server = MockServer::start().await;
                mock_json(&server, lmstudio_basic_json()).await;

                let model = make_model(&server, "gemma-7b-it");
                let options = CallOptions {
                    max_output_tokens: Some(100),
                    temperature: Some(0.5),
                    top_p: Some(0.9),
                    presence_penalty: Some(0.1),
                    frequency_penalty: Some(0.2),
                    response_format: Some(ResponseFormat::Json {
                        schema: Some(json!({
                            "type": "object",
                            "properties": {"status": {"type": "string"}},
                            "required": ["status"]
                        })),
                        name: Some("response".to_string()),
                        description: Some("Example response schema".to_string()),
                    }),
                    ..CallOptions::new(wiremock_test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(
                    body,
                    json!({
                        "model": "gemma-7b-it",
                        "input": [{
                            "type": "message",
                            "role": "user",
                            "content": [{"type": "input_text", "text": "Hello"}]
                        }],
                        "max_output_tokens": 100,
                        "temperature": 0.5,
                        "top_p": 0.9,
                        "presence_penalty": 0.1,
                        "frequency_penalty": 0.2,
                        "text": {
                            "format": {
                                "type": "json_schema",
                                "strict": true,
                                "name": "response",
                                "description": "Example response schema",
                                "schema": {
                                    "type": "object",
                                    "properties": {"status": {"type": "string"}},
                                    "required": ["status"]
                                }
                            }
                        }
                    })
                );
            }
        }

        mod top_level_reasoning {
            use super::*;

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › top-level reasoning › should map top-level reasoning to reasoning effort
            #[tokio::test]
            async fn reasoning_high_maps_to_effort() {
                let server = MockServer::start().await;
                mock_json(&server, lmstudio_basic_json()).await;

                let model = make_model(&server, "gemma-7b-it");
                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::High),
                    ..CallOptions::new(wiremock_test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["reasoning"], json!({"effort": "high"}));
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › top-level reasoning › should coerce top-level reasoning minimal to low
            #[tokio::test]
            async fn reasoning_minimal_coerced_to_low() {
                let server = MockServer::start().await;
                mock_json(&server, lmstudio_basic_json()).await;

                let model = make_model(&server, "gemma-7b-it");
                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::Minimal),
                    ..CallOptions::new(wiremock_test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["reasoning"], json!({"effort": "low"}));
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › top-level reasoning › should map top-level reasoning none to none
            #[tokio::test]
            async fn reasoning_none_maps_to_none() {
                let server = MockServer::start().await;
                mock_json(&server, lmstudio_basic_json()).await;

                let model = make_model(&server, "gemma-7b-it");
                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::None),
                    ..CallOptions::new(wiremock_test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["reasoning"], json!({"effort": "none"}));
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › top-level reasoning › should pass xhigh directly
            #[tokio::test]
            async fn reasoning_xhigh_passed_directly() {
                let server = MockServer::start().await;
                mock_json(&server, lmstudio_basic_json()).await;

                let model = make_model(&server, "gemma-7b-it");
                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::Xhigh),
                    ..CallOptions::new(wiremock_test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["reasoning"], json!({"effort": "xhigh"}));
            }

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › top-level reasoning › should not set reasoning when not specified
            #[tokio::test]
            async fn reasoning_not_set_when_not_specified() {
                let server = MockServer::start().await;
                mock_json(&server, lmstudio_basic_json()).await;

                let model = make_model(&server, "gemma-7b-it");
                model
                    .do_generate(&default_options(wiremock_test_prompt()))
                    .await
                    .unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert!(body.get("reasoning").is_none());
            }
        }

        mod multi_turn_tool_conversation {
            use super::*;

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doGenerate › multi-turn tool conversation › should send correct request body with user, assistant tool-call, and tool result
            #[tokio::test]
            async fn multi_turn_tool_conversation_request_body() {
                let server = MockServer::start().await;
                mock_json(&server, lmstudio_basic_json()).await;

                let model = make_model(&server, "gemma-7b-it");
                let prompt = vec![
                    LanguageModelMessage::user_text("What is the weather in Tokyo?"),
                    LanguageModelMessage::Assistant {
                        content: vec![AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id: "call_weather_123".into(),
                            tool_name: "get_weather".into(),
                            input: json!({"location": "Tokyo"}),
                            provider_executed: None,
                            provider_options: None,
                        })],
                        provider_options: None,
                    },
                    LanguageModelMessage::Tool {
                        content: vec![ToolPart::ToolResult(ToolResultPart {
                            tool_call_id: "call_weather_123".into(),
                            output: serde_json::from_value(json!({
                                "type": "json",
                                "value": {"temperature": 22, "condition": "sunny", "humidity": 65}
                            }))
                            .unwrap(),
                            tool_name: "get_weather".into(),
                            provider_options: None,
                        })],
                        provider_options: None,
                    },
                ];
                let options = CallOptions {
                    tools: Some(vec![Tool::from(
                        FunctionTool::new(
                            "get_weather",
                            json!({
                                "type": "object",
                                "properties": {"location": {"type": "string"}},
                                "required": ["location"]
                            }),
                        )
                        .with_description("Get the current weather for a location"),
                    )]),
                    ..CallOptions::new(prompt)
                };
                model.do_generate(&options).await.unwrap();

                let request = &server.received_requests().await.unwrap()[0];
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(
                    body,
                    json!({
                        "model": "gemma-7b-it",
                        "input": [
                            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What is the weather in Tokyo?"}]},
                            {"type": "function_call", "call_id": "call_weather_123", "name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"},
                            {"type": "function_call_output", "call_id": "call_weather_123", "output": "{\"temperature\":22,\"condition\":\"sunny\",\"humidity\":65}"}
                        ],
                        "tools": [{
                            "type": "function",
                            "name": "get_weather",
                            "parameters": {"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"]},
                            "description": "Get the current weather for a location"
                        }]
                    })
                );
            }
        }
    }

    mod do_stream {
        use super::*;

        /// `createModel(model_id).doStream(options)` against `response`;
        /// returns the stream parts and the transport.
        async fn stream(
            model_id: &str,
            response: Canned,
            options: Value,
        ) -> (Vec<StreamPart>, Arc<MockFetch>) {
            let (model, mock) = create_model(model_id, vec![response]);
            let result = model.do_stream(&call(options)).await.unwrap();
            (collect_parts(result).await, mock)
        }

        mod basic_generation {
            use super::*;

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › basic generation › should stream content
            #[tokio::test]
            async fn should_stream_content() {
                let (parts, _) = stream(
                    "gemma-7b-it",
                    chunks_fixture(fixture!("lmstudio-basic.1.chunks.txt")),
                    json!({ "prompt": test_prompt() }),
                )
                .await;
                assert_json_eq(
                    &Value::Array(parts_json(&parts)),
                    &snapshot!("stream_basic_generation"),
                );
            }
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › should stream reasoning summary text deltas
        #[tokio::test]
        async fn should_stream_reasoning_summary_text_deltas() {
            let (parts, _) = stream(
                "gemma-7b-it",
                chunks_fixture(fixture!("openai-reasoning-summary-text.1.chunks.txt")),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            let reasoning: Vec<Value> = parts_json(&parts)
                .into_iter()
                .filter(|part| part["type"].as_str().unwrap().starts_with("reasoning"))
                .collect();
            assert_eq!(
                Value::Array(reasoning),
                json!([
                    { "type": "reasoning-start", "id": "rs_1" },
                    { "type": "reasoning-delta", "id": "rs_1", "delta": "Think" },
                    { "type": "reasoning-delta", "id": "rs_1", "delta": "ing." },
                    {
                        "type": "reasoning-end",
                        "id": "rs_1",
                        "providerMetadata": {
                            "lmstudio": {
                                "itemId": "rs_1",
                                "reasoningSummary": [
                                    { "type": "summary_text", "text": "Thinking." },
                                ],
                                "reasoningContent": null,
                            },
                        },
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › should use complete function arguments from the completed item when argument events are omitted
        #[tokio::test]
        async fn should_use_complete_function_arguments_from_the_completed_item() {
            let (parts, _) = stream(
                "gemma-7b-it",
                stream_chunks(vec![
                    data(&json!({
                        "type": "response.output_item.added",
                        "sequence_number": 0,
                        "output_index": 0,
                        "item": {
                            "id": "fc_1",
                            "type": "function_call",
                            "status": "in_progress",
                            "call_id": "call_1",
                            "name": "write_svg",
                            "arguments": "",
                        },
                    })),
                    data(&json!({
                        "type": "response.output_item.done",
                        "sequence_number": 1,
                        "output_index": 0,
                        "item": {
                            "id": "fc_1",
                            "type": "function_call",
                            "status": "completed",
                            "call_id": "call_1",
                            "name": "write_svg",
                            "arguments": "{\"path\":\"icon.svg\"}",
                        },
                    })),
                    "data: [DONE]\n\n".into(),
                ]),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            assert_contains_equal(
                &parts_json(&parts),
                &json!({
                    "type": "tool-call",
                    "toolCallId": "call_1",
                    "toolName": "write_svg",
                    "input": "{\"path\":\"icon.svg\"}",
                    "providerMetadata": { "lmstudio": { "itemId": "fc_1" } },
                }),
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › should preserve output text annotations in stream metadata
        #[tokio::test]
        async fn should_preserve_output_text_annotations_in_stream_metadata() {
            let annotation = json!({
                "type": "url_citation",
                "start_index": 0,
                "end_index": 7,
                "url": "https://example.com/source",
                "title": "Example source",
            });
            let (parts, _) = stream(
                "gemma-7b-it",
                stream_chunks(vec![
                    data(&json!({
                        "type": "response.output_item.added",
                        "sequence_number": 0,
                        "output_index": 0,
                        "item": {
                            "id": "msg_annotated",
                            "type": "message",
                            "role": "assistant",
                            "status": "in_progress",
                            "content": [],
                        },
                    })),
                    data(&json!({
                        "type": "response.output_text.delta",
                        "sequence_number": 1,
                        "item_id": "msg_annotated",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": "Sourced answer",
                    })),
                    data(&json!({
                        "type": "response.output_item.done",
                        "sequence_number": 2,
                        "output_index": 0,
                        "item": {
                            "id": "msg_annotated",
                            "type": "message",
                            "role": "assistant",
                            "status": "completed",
                            "content": [{
                                "type": "output_text",
                                "text": "Sourced answer",
                                "annotations": [annotation],
                            }],
                        },
                    })),
                    "data: [DONE]\n\n".into(),
                ]),
                json!({ "prompt": test_prompt() }),
            )
            .await;

            assert_contains_equal(
                &parts_json(&parts),
                &json!({
                    "type": "text-end",
                    "id": "msg_annotated",
                    "providerMetadata": {
                        "lmstudio": {
                            "itemId": "msg_annotated",
                            "annotations": [annotation],
                        },
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › should send provider-native reasoning effort when streaming
        #[tokio::test]
        async fn should_send_provider_native_reasoning_effort_when_streaming() {
            let (_, mock) = stream(
                "gemma-7b-it",
                chunks_fixture(fixture!("lmstudio-basic.1.chunks.txt")),
                json!({
                    "prompt": test_prompt(),
                    "providerOptions": { "lmstudio": { "reasoningEffort": "max" } },
                }),
            )
            .await;
            assert_eq!(
                mock.seen()[0].json_body().get("reasoning"),
                Some(&json!({ "effort": "max" }))
            );
        }

        mod reasoning_with_tool_call {
            use super::*;

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › reasoning with tool call › should stream reasoning and tool call content
            #[tokio::test]
            async fn should_stream_reasoning_and_tool_call_content() {
                let (parts, _) = stream(
                    "gemma-7b-it",
                    chunks_fixture(fixture!("lmstudio-tool-call.2.chunks.txt")),
                    json!({ "prompt": test_prompt() }),
                )
                .await;
                assert_json_eq(
                    &Value::Array(parts_json(&parts)),
                    &snapshot!("stream_reasoning_with_tool_call"),
                );
            }
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › should close unfinished reasoning items with their original ids
        #[tokio::test]
        async fn should_close_unfinished_reasoning_items_with_their_original_ids() {
            let (parts, _) = stream(
                "gemma-7b-it",
                stream_chunks(vec![
                    data(&json!({
                        "type": "response.output_item.added",
                        "sequence_number": 0,
                        "output_index": 0,
                        "item": { "id": "rs_reasoning_item", "type": "reasoning", "summary": [] },
                    })),
                    data(&json!({
                        "type": "response.incomplete",
                        "sequence_number": 1,
                        "response": {
                            "status": "incomplete",
                            "incomplete_details": { "reason": "max_output_tokens" },
                            "usage": {
                                "input_tokens": 1,
                                "input_tokens_details": { "cached_tokens": 0 },
                                "output_tokens": 1,
                                "output_tokens_details": { "reasoning_tokens": 1 },
                                "total_tokens": 2,
                            },
                        },
                    })),
                    "data: [DONE]\n\n".into(),
                ]),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            let parts = parts_json(&parts);

            assert_contains_equal(
                &parts,
                &json!({ "type": "reasoning-start", "id": "rs_reasoning_item" }),
            );
            assert_contains_equal(
                &parts,
                &json!({ "type": "reasoning-end", "id": "rs_reasoning_item" }),
            );
            assert_match_object(
                parts.last().unwrap(),
                &json!({
                    "type": "finish",
                    "finishReason": { "unified": "length", "raw": "max_output_tokens" },
                }),
            );
        }

        /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › should not pollute Object.prototype from tool call item ids
        ///
        /// Rust has no `Object.prototype`; what carries over is that a
        /// `__proto__` item id is an ordinary key, so the stream completes
        /// without failing (`collect_parts` fails the test on a stream error).
        #[tokio::test]
        async fn should_not_pollute_object_prototype_from_tool_call_item_ids() {
            let (parts, _) = stream(
                "gemma-7b-it",
                stream_chunks(vec![
                    data(&json!({
                        "type": "response.function_call_arguments.done",
                        "item_id": "__proto__",
                        "output_index": 0,
                        "arguments": "polluted",
                        "sequence_number": 0,
                    })),
                    data(&json!({
                        "type": "response.completed",
                        "response": {
                            "incomplete_details": null,
                            "status": "completed",
                            "usage": {
                                "input_tokens": 0,
                                "input_tokens_details": { "cached_tokens": 0 },
                                "output_tokens": 0,
                                "output_tokens_details": { "reasoning_tokens": 0 },
                                "total_tokens": 0,
                            },
                        },
                        "sequence_number": 1,
                    })),
                    "data: [DONE]\n\n".into(),
                ]),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            assert!(matches!(parts.last(), Some(StreamPart::Finish { .. })));
        }

        mod pdf_input_file {
            use super::*;

            /// upstream: @ai-sdk/open-responses src/responses/open-responses-language-model.test.ts › OpenResponsesLanguageModel › doStream › pdf input file › should stream content from pdf input
            #[tokio::test]
            async fn should_stream_content_from_pdf_input() {
                let (parts, _) = stream(
                    "gpt-4.1-nano",
                    chunks_fixture(fixture!("openai-pdf-input-file.1.chunks.txt")),
                    json!({
                        "prompt": [{
                            "role": "user",
                            "content": [
                                { "type": "text", "text": "What text does this PDF contain?" },
                                {
                                    "type": "file",
                                    "data": {
                                        "type": "url",
                                        "url": "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf",
                                    },
                                    "mediaType": "application/pdf",
                                },
                            ],
                        }],
                    }),
                )
                .await;

                let id = "msg_051ebd7ab60063870069d4fe8c1b7c8194b701e22f1ef094dd";
                assert_json_eq(
                    &Value::Array(parts_json(&parts)),
                    &json!([
                        { "type": "stream-start", "warnings": [] },
                        { "id": id, "type": "text-start" },
                        { "delta": "Dummy", "id": id, "type": "text-delta" },
                        { "delta": " PDF", "id": id, "type": "text-delta" },
                        { "delta": " file", "id": id, "type": "text-delta" },
                        {
                            "id": id,
                            "providerMetadata": { "lmstudio": { "itemId": id } },
                            "type": "text-end",
                        },
                        {
                            "finishReason": { "unified": "stop" },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "noCache": 44,
                                    "total": 44,
                                },
                                "outputTokens": {
                                    "reasoning": 0,
                                    "text": 4,
                                    "total": 4,
                                },
                                "raw": {
                                    "input_tokens": 44,
                                    "input_tokens_details": { "cached_tokens": 0 },
                                    "output_tokens": 4,
                                    "output_tokens_details": { "reasoning_tokens": 0 },
                                    "total_tokens": 48,
                                },
                            },
                        },
                    ]),
                );
            }
        }
    }
}

mod do_generate_tests {
    use super::*;

    /// `generate_text` always sends `{type:"auto"}`, and open-responses
    /// forwards `toolChoice?.type` with or without tools
    /// (`@ai-sdk/open-responses@2.0.58
    /// src/responses/open-responses-language-model.ts:310, :379-380`).
    #[tokio::test]
    async fn tool_choice_is_sent_without_tools() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tool_choice: Some(LanguageModelV4ToolChoice::Auto),
            ..default_options(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["tool_choice"], "auto");
        assert!(body.get("tools").is_none());
    }

    /// partial: `should produce correct content`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn basic_generation_content() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_generate(&default_options(wiremock_test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.content.len(), 2);
        assert_eq!(
            result.content[0],
            GenerateContent::Reasoning(ReasoningOutput {
                text: "reasoning content".to_string(),
                provider_metadata: serde_json::from_value(json!({ "lmstudio": {
                    "itemId": "rs_3l1z5wpifxkwxhj459ya7",
                    "reasoningSummary": [],
                    "reasoningContent": [{ "type": "reasoning_text", "text": "reasoning content" }],
                } }))
                .unwrap(),
            })
        );
        assert_eq!(
            result.content[1],
            GenerateContent::Text {
                text: "text content".to_string(),
                provider_metadata: serde_json::from_value(
                    json!({ "lmstudio": { "itemId": "msg_p1y190hl7hj1xyfqr1cir" } })
                )
                .unwrap(),
            }
        );
    }

    /// partial: `should extract usage correctly`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn basic_generation_usage() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_generate(&default_options(wiremock_test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(136));
        assert_eq!(result.usage.input_tokens.no_cache, Some(136));
        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(3677));
        assert_eq!(result.usage.output_tokens.reasoning, Some(2456));
        assert_eq!(result.usage.output_tokens.text, Some(1221));
    }

    /// partial: `should send correct request body with tools`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn tools_request_body() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![
                Tool::from(
                    FunctionTool::new(
                        "get_weather",
                        json!({
                            "type": "object",
                            "properties": {
                                "location": {"type": "string", "description": "The city and state"}
                            },
                            "required": ["location"]
                        }),
                    )
                    .with_description("Get the current weather for a location"),
                ),
                Tool::from(
                    FunctionTool::new(
                        "search",
                        json!({
                            "type": "object",
                            "properties": {"query": {"type": "string"}},
                            "required": ["query"]
                        }),
                    )
                    .with_description("Search for information")
                    .with_strict(true),
                ),
            ]),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gemma-7b-it",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Hello"}]
                }],
                "tools": [
                    {
                        "type": "function",
                        "name": "get_weather",
                        "parameters": {
                            "type": "object",
                            "properties": {"location": {"type": "string", "description": "The city and state"}},
                            "required": ["location"]
                        },
                        "description": "Get the current weather for a location"
                    },
                    {
                        "type": "function",
                        "name": "search",
                        "parameters": {
                            "type": "object",
                            "properties": {"query": {"type": "string"}},
                            "required": ["query"]
                        },
                        "description": "Search for information",
                        "strict": true
                    }
                ]
            })
        );
    }

    // -- ProviderOptions reasoning tests --

    fn lmstudio_opts(value: Value) -> Option<aimux_core::shared::SharedProviderOptions> {
        Some(aimux_core::shared::provider_namespace("lmstudio", value).unwrap())
    }

    /// partial: `should send reasoning.summary via providerOptions`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn provider_options_reasoning_summary_detailed() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            provider_options: lmstudio_opts(json!({"reasoningSummary": "detailed"})),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reasoning"], json!({"summary": "detailed"}));
    }

    /// partial: `should combine top-level reasoning effort with providerOptions summary`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn provider_options_combines_effort_with_summary() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::High),
            provider_options: lmstudio_opts(json!({"reasoningSummary": "auto"})),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["reasoning"],
            json!({"effort": "high", "summary": "auto"})
        );
    }

    /// partial: `should send reasoning.summary concise via providerOptions`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn provider_options_reasoning_summary_concise() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            provider_options: lmstudio_opts(json!({"reasoningSummary": "concise"})),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reasoning"], json!({"summary": "concise"}));
    }

    /// partial: `should not set reasoning when providerOptions has no reasoning fields`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn provider_options_no_reasoning_fields() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            provider_options: lmstudio_opts(json!({})),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert!(body.get("reasoning").is_none());
    }

    // -- Tool call parsing tests --

    /// partial: `should parse tool call from response`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn parse_tool_call_from_response() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_tool_call_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![Tool::from(
                FunctionTool::new(
                    "weather",
                    json!({
                        "type": "object",
                        "properties": {
                            "location": {"type": "string", "description": "The location to get the weather for"}
                        },
                        "required": ["location"]
                    }),
                )
                .with_description("Get the weather in a location"),
            )]),
            tool_choice: Some(LanguageModelV4ToolChoice::Required),
            ..CallOptions::new(wiremock_test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => {
                assert_eq!(tool_call_id, "call_2866856768160095");
                assert_eq!(tool_name, "weather");
                assert_eq!(
                    input,
                    &Value::String(r#"{"location":"San Francisco"}"#.into())
                );
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// partial: `should return tool-calls finish reason`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn tool_call_finish_reason() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_tool_call_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![Tool::from(
                FunctionTool::new("weather", json!({"type": "object"}))
                    .with_description("Get the weather in a location"),
            )]),
            tool_choice: Some(LanguageModelV4ToolChoice::Required),
            ..CallOptions::new(wiremock_test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(result.finish_reason.raw, None);
    }

    /// partial: `should extract usage correctly`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn tool_call_usage() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_tool_call_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![Tool::from(
                FunctionTool::new("weather", json!({"type": "object"}))
                    .with_description("Get the weather in a location"),
            )]),
            tool_choice: Some(LanguageModelV4ToolChoice::Required),
            ..CallOptions::new(wiremock_test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(1189));
        assert_eq!(result.usage.input_tokens.cache_read, Some(891));
        assert_eq!(result.usage.input_tokens.no_cache, Some(298));
        assert_eq!(result.usage.output_tokens.total, Some(11));
        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(11));
    }

    // -- Tool choice tests --

    fn test_tool() -> Tool {
        Tool::from(
            FunctionTool::new(
                "get_weather",
                json!({
                    "type": "object",
                    "properties": {"location": {"type": "string"}},
                    "required": ["location"]
                }),
            )
            .with_description("Get the current weather"),
        )
    }

    /// partial: `should send tool_choice auto`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn tool_choice_auto() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![test_tool()]),
            tool_choice: Some(LanguageModelV4ToolChoice::Auto),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["tool_choice"], json!("auto"));
        assert!(body.get("tools").is_some());
    }

    /// partial: `should send tool_choice none`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn tool_choice_none() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![test_tool()]),
            tool_choice: Some(LanguageModelV4ToolChoice::None),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["tool_choice"], json!("none"));
    }

    /// partial: `should send tool_choice required`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn tool_choice_required() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![test_tool()]),
            tool_choice: Some(LanguageModelV4ToolChoice::Required),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["tool_choice"], json!("required"));
    }

    /// partial: `should send tool_choice with specific tool`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn tool_choice_specific_tool() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![test_tool()]),
            tool_choice: Some(LanguageModelV4ToolChoice::Tool {
                tool_name: "get_weather".to_string(),
            }),
            ..CallOptions::new(wiremock_test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["tool_choice"],
            json!({"type": "function", "name": "get_weather"})
        );
    }

    // -- System messages tests --

    /// partial: `should send instructions from system message`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn system_message_instructions() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let prompt = vec![
            LanguageModelMessage::System {
                content: "You are a helpful assistant.".into(),
                provider_options: None,
            },
            LanguageModelMessage::user_text("Hello"),
        ];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["instructions"], json!("You are a helpful assistant."));
    }

    /// partial: `should join multiple system messages with newlines`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn multiple_system_messages_joined() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let prompt = vec![
            LanguageModelMessage::System {
                content: "You are a helpful assistant.".into(),
                provider_options: None,
            },
            LanguageModelMessage::System {
                content: "Always be concise.".into(),
                provider_options: None,
            },
            LanguageModelMessage::user_text("Hello"),
        ];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["instructions"],
            json!("You are a helpful assistant.\nAlways be concise.")
        );
    }

    // -- PDF input file --

    /// partial: `should produce correct content`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn pdf_input_file_content() {
        let server = MockServer::start().await;
        mock_json(&server, openai_pdf_json()).await;

        let model = make_model(&server, "gpt-4.1-nano");
        let prompt = vec![LanguageModelMessage::User {
            content: vec![
UserPart::Text(TextPart {
text: "What text does this PDF contain?".into(),
provider_options: None
}),
UserPart::File(FilePart {
data: FileData::Url {
original_url: None,
url: "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf".into()
},
media_type: "application/pdf".into(),
filename: None,
provider_options: None
})
],
            provider_options: None,
        }];
        let result = model.do_generate(&default_options(prompt)).await.unwrap();

        assert_eq!(result.content.len(), 1);
        assert_eq!(
            result.content[0],
            GenerateContent::Text {
                text: "Dummy PDF file".to_string(),
                provider_metadata: serde_json::from_value(json!({ "lmstudio": {
                    "itemId": "msg_048edf44633e41ae0069d4fea0d1a08194af1e491c093df1d9"
                } }))
                .unwrap(),
            }
        );
    }

    /// partial: `should extract usage correctly`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn pdf_input_file_usage() {
        let server = MockServer::start().await;
        mock_json(&server, openai_pdf_json()).await;

        let model = make_model(&server, "gpt-4.1-nano");
        let prompt = vec![LanguageModelMessage::User {
            content: vec![
UserPart::Text(TextPart {
text: "What text does this PDF contain?".into(),
provider_options: None
}),
UserPart::File(FilePart {
data: FileData::Url {
original_url: None,
url: "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf".into()
},
media_type: "application/pdf".into(),
filename: None,
provider_options: None
})
],
            provider_options: None,
        }];
        let result = model.do_generate(&default_options(prompt)).await.unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(44));
        assert_eq!(result.usage.input_tokens.no_cache, Some(44));
        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(4));
        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(4));
    }
}

mod do_stream_tests {
    use super::*;

    /// Build SSE chunks for a basic text streaming response (simplified).
    fn basic_stream_chunks() -> Vec<String> {
        vec![
            sse_event(
                r#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress","incomplete_details":null,"usage":null},"sequence_number":0}"#,
            ),
            sse_event(
                r#"{"type":"response.in_progress","response":{"id":"resp_1","status":"in_progress","incomplete_details":null,"usage":null},"sequence_number":1}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","content":[],"role":"assistant"},"sequence_number":2}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello","sequence_number":3}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":" world","sequence_number":4}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","status":"completed","content":[{"type":"output_text","text":"Hello world"}],"role":"assistant"},"sequence_number":5}"#,
            ),
            sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","incomplete_details":null,"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}}},"sequence_number":6}"#,
            ),
        ]
    }

    /// partial: `should stream content`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn stream_basic_generation() {
        let server = MockServer::start().await;
        let chunks = basic_stream_chunks();
        let body = sse_body(
            &chunks
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        mock_sse(&server, body).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_stream(&default_options(wiremock_test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        // StreamStart
        assert!(matches!(
            &parts[0],
            StreamPart::StreamStart { warnings } if warnings.is_empty()
        ));

        // TextStart
        assert!(matches!(
            &parts[1],
            StreamPart::TextStart { id, .. } if id == "msg_1"
        ));

        // TextDelta "Hello"
        assert!(matches!(
            &parts[2],
            StreamPart::TextDelta { id, delta, .. } if id == "msg_1" && delta == "Hello"
        ));

        // TextDelta " world"
        assert!(matches!(
            &parts[3],
            StreamPart::TextDelta { id, delta, .. } if id == "msg_1" && delta == " world"
        ));

        // TextEnd
        assert!(matches!(
            &parts[4],
            StreamPart::TextEnd { id, .. } if id == "msg_1"
        ));

        // Finish
        assert!(
            matches!(&parts[5], StreamPart::Finish { finish_reason, usage, .. } if
                finish_reason.unified == FinishReasonUnified::Stop &&
                usage.input_tokens.total == Some(10) &&
                usage.output_tokens.total == Some(5)
            )
        );
    }

    /// Build SSE chunks for a reasoning + tool call streaming response.
    fn reasoning_tool_call_chunks() -> Vec<String> {
        vec![
            sse_event(
                r#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress","incomplete_details":null,"usage":null},"sequence_number":0}"#,
            ),
            // Reasoning item added
            sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","status":"in_progress","summary":[],"content":[]},"sequence_number":1}"#,
            ),
            // Reasoning text deltas
            sse_event(
                r#"{"type":"response.reasoning_text.delta","item_id":"rs_1","output_index":0,"content_index":0,"delta":"Thinking","sequence_number":2}"#,
            ),
            sse_event(
                r#"{"type":"response.reasoning_text.delta","item_id":"rs_1","output_index":0,"content_index":0,"delta":" about weather","sequence_number":3}"#,
            ),
            // Reasoning item done
            sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","status":"completed","summary":[],"content":[{"type":"reasoning_text","text":"Thinking about weather"}]},"sequence_number":4}"#,
            ),
            // Message item added
            sse_event(
                r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message","status":"in_progress","content":[],"role":"assistant"},"sequence_number":5}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"content_index":0,"delta":"I'll check.","sequence_number":6}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.done","output_index":1,"item":{"id":"msg_1","type":"message","status":"completed","content":[{"type":"output_text","text":"I'll check."}],"role":"assistant"},"sequence_number":7}"#,
            ),
            // Function call
            sse_event(
                r#"{"type":"response.output_item.added","output_index":2,"item":{"id":"fc_1","type":"function_call","status":"in_progress","arguments":"","call_id":"call_1","name":"weather"},"sequence_number":8}"#,
            ),
            sse_event(
                r#"{"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":2,"arguments":"{\"location\":\"SF\"}","sequence_number":9}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.done","output_index":2,"item":{"id":"fc_1","type":"function_call","status":"completed","arguments":"{\"location\":\"SF\"}","call_id":"call_1","name":"weather"},"sequence_number":10}"#,
            ),
            // Response completed
            sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","incomplete_details":null,"usage":{"input_tokens":100,"output_tokens":50,"total_tokens":150,"input_tokens_details":{"cached_tokens":20},"output_tokens_details":{"reasoning_tokens":10}}},"sequence_number":11}"#,
            ),
        ]
    }

    /// partial: `should stream reasoning and tool call content`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn stream_reasoning_with_tool_call() {
        let server = MockServer::start().await;
        let chunks = reasoning_tool_call_chunks();
        let body = sse_body(
            &chunks
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        mock_sse(&server, body).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_stream(&default_options(wiremock_test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        // Find the key parts.
        let has_reasoning_start = parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningStart { id, .. } if id == "rs_1"));
        assert!(has_reasoning_start, "should have ReasoningStart");

        let reasoning_deltas: Vec<&str> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ReasoningDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(reasoning_deltas, vec!["Thinking", " about weather"]);

        let has_reasoning_end = parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { id, .. } if id == "rs_1"));
        assert!(has_reasoning_end, "should have ReasoningEnd");

        // ToolCall
        let tool_call = parts.iter().find_map(|p| match p {
            StreamPart::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => Some((tool_call_id, tool_name, input)),
            _ => None,
        });
        assert!(tool_call.is_some(), "should have ToolCall");
        let (tc_id, tc_name, tc_input) = tool_call.unwrap();
        assert_eq!(tc_id, "call_1");
        assert_eq!(tc_name, "weather");
        assert_eq!(tc_input, &Value::String(r#"{"location":"SF"}"#.into()));

        // Finish with tool-calls reason
        let finish = parts.iter().find_map(|p| match p {
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => Some((finish_reason, usage)),
            _ => None,
        });
        assert!(finish.is_some(), "should have Finish");
        let (fr, usage) = finish.unwrap();
        assert_eq!(fr.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(usage.input_tokens.total, Some(100));
        assert_eq!(usage.input_tokens.cache_read, Some(20));
        assert_eq!(usage.output_tokens.total, Some(50));
        assert_eq!(usage.output_tokens.reasoning, Some(10));
    }

    /// Build SSE chunks for a PDF input streaming response.
    fn pdf_stream_chunks() -> Vec<String> {
        vec![
            sse_event(
                r#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress","incomplete_details":null,"usage":null},"sequence_number":0}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.added","item":{"id":"msg_051","type":"message","status":"in_progress","content":[],"role":"assistant"},"output_index":0,"sequence_number":2}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","content_index":0,"delta":"Dummy","item_id":"msg_051","output_index":0,"sequence_number":4}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","content_index":0,"delta":" PDF","item_id":"msg_051","output_index":0,"sequence_number":5}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","content_index":0,"delta":" file","item_id":"msg_051","output_index":0,"sequence_number":6}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.done","item":{"id":"msg_051","type":"message","status":"completed","content":[{"type":"output_text","text":"Dummy PDF file"}],"role":"assistant"},"output_index":0,"sequence_number":9}"#,
            ),
            sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","incomplete_details":null,"usage":{"input_tokens":44,"input_tokens_details":{"cached_tokens":0},"output_tokens":4,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":48}},"sequence_number":10}"#,
            ),
        ]
    }

    /// partial: `should stream content from pdf input`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn stream_pdf_input() {
        let server = MockServer::start().await;
        let chunks = pdf_stream_chunks();
        let body = sse_body(
            &chunks
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        mock_sse(&server, body).await;

        let model = make_model(&server, "gpt-4.1-nano");
        let prompt = vec![LanguageModelMessage::User {
            content: vec![
UserPart::Text(TextPart {
text: "What text does this PDF contain?".into(),
provider_options: None
}),
UserPart::File(FilePart {
data: FileData::Url {
original_url: None,
url: "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf".into()
},
media_type: "application/pdf".into(),
filename: None,
provider_options: None
})
],
            provider_options: None,
        }];
        let result = model.do_stream(&default_options(prompt)).await.unwrap();
        let parts = collect_stream(result).await;

        // Collect all text deltas
        let text: String = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Dummy PDF file");

        // Verify finish
        let finish = parts.iter().find_map(|p| match p {
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => Some((finish_reason, usage)),
            _ => None,
        });
        assert!(finish.is_some());
        let (fr, usage) = finish.unwrap();
        assert_eq!(fr.unified, FinishReasonUnified::Stop);
        assert_eq!(usage.input_tokens.total, Some(44));
        assert_eq!(usage.output_tokens.total, Some(4));
    }

    /// partial: `should not pollute Object.prototype from tool call item ids`; full port in src/open_responses/tests/responses/language_model.rs.
    #[tokio::test]
    async fn stream_tool_call_with_proto_item_id() {
        // Test that "__proto__" as item_id doesn't cause issues (Rust HashMap is safe).
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"type":"response.function_call_arguments.done","item_id":"__proto__","output_index":0,"arguments":"polluted","sequence_number":0}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"incomplete_details":null,"status":"completed","usage":{"input_tokens":0,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":0}},"sequence_number":1}"#,
            ),
        ]);
        mock_sse(&server, body).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_stream(&default_options(wiremock_test_prompt()))
            .await
            .unwrap();

        // Should not panic or error.
        let _parts = collect_stream(result).await;
    }
}

/// TS: `supportedUrls` (open-responses-language-model.ts:101-103): image URLs
/// go to the API as they are; other URLs are downloaded by the operation.
#[tokio::test]
async fn supports_http_image_urls() {
    let server = MockServer::start().await;
    let urls = make_model(&server, "gpt-4o").supported_urls().await;
    assert_eq!(urls.0.len(), 1);
    assert!(urls.0["image/*"][0].is_match("https://example.com/a.png"));
    assert!(!urls.0["image/*"][0].is_match("ftp://example.com/a.png"));
}

/// TS: `resolveFullMediaType` throws for an inline user file whose subtype
/// cannot be detected, so the call fails before anything is sent.
#[tokio::test]
async fn an_undetectable_user_file_media_type_fails_before_sending() {
    let server = MockServer::start().await;
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "user",
        "content": [{ "type": "file", "mediaType": "image/*",
                      "data": { "type": "data", "data": "AAEC" } }]
    }]))
    .unwrap();
    let error = make_model(&server, "gpt-4o")
        .do_generate(&default_options(prompt))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            aimux_core::error::AiMuxError::UnsupportedFunctionality(_)
        ),
        "{error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// TS: `convertToOpenResponsesInput` (convert-to-open-responses-input.ts:75-84)
/// throws `UnsupportedFunctionalityError` for a user file given as a provider
/// reference or as text, so the call fails before anything is sent.
#[tokio::test]
async fn reference_and_text_user_files_are_unsupported() {
    let server = MockServer::start().await;
    for (data, functionality) in [
        (
            json!({ "type": "reference", "reference": { "lmstudio": "file-1" } }),
            "file parts with provider references",
        ),
        (
            json!({ "type": "text", "text": "hello" }),
            "text file parts",
        ),
    ] {
        let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
            "role": "user",
            "content": [{ "type": "file", "mediaType": "text/plain", "data": data }]
        }]))
        .unwrap();
        let error = make_model(&server, "gpt-4o")
            .do_generate(&default_options(prompt))
            .await
            .unwrap_err();
        assert!(
            matches!(
                &error,
                aimux_core::error::AiMuxError::UnsupportedFunctionality(f) if f == functionality
            ),
            "{error:?}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// TS: `OpenResponsesLanguageModel` (open-responses-language-model.ts:446-505,
/// :757-779, :876-917): generated and streamed output carries
/// `{ <name>: { itemId } }`, text adds its `url_citation` annotations, and
/// reasoning adds its summary, content and encrypted content.
#[tokio::test]
async fn output_parts_carry_the_item_metadata() {
    let citation = json!({
        "type": "url_citation", "start_index": 0, "end_index": 4,
        "url": "https://example.com", "title": "Example"
    });
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_1", "created_at": 1, "model": "m", "status": "completed",
            "output": [
                { "type": "reasoning", "id": "rs_1", "summary": [],
                  "content": [{ "type": "reasoning_text", "text": "Think" }],
                  "encrypted_content": "enc" },
                { "type": "message", "id": "msg_1", "role": "assistant",
                  "content": [{ "type": "output_text", "text": "Done", "annotations": [citation] }] },
                { "type": "function_call", "id": "fc_1", "call_id": "call_1",
                  "name": "weather", "arguments": "{}" }
            ]
        }),
    )
    .await;
    let result = make_model(&server, "m")
        .do_generate(&default_options(wiremock_test_prompt()))
        .await
        .unwrap();
    let metadata: Vec<Value> = result
        .content
        .iter()
        .map(|part| match part {
            GenerateContent::Reasoning(part) => json!(part.provider_metadata),
            GenerateContent::Text {
                provider_metadata, ..
            } => json!(provider_metadata),
            GenerateContent::ToolCall(call) => json!(call.provider_metadata),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        metadata,
        vec![
            json!({ "lmstudio": { "itemId": "rs_1", "reasoningSummary": [],
                "reasoningContent": [{ "type": "reasoning_text", "text": "Think" }],
                "reasoningEncryptedContent": "enc" } }),
            json!({ "lmstudio": { "itemId": "msg_1", "annotations": [citation] } }),
            json!({ "lmstudio": { "itemId": "fc_1" } }),
        ]
    );

    let server = MockServer::start().await;
    let events = [
        json!({ "type": "response.output_item.added", "item": { "type": "message", "id": "msg_1" } }),
        json!({ "type": "response.output_item.done", "item": { "type": "message", "id": "msg_1",
                "content": [{ "type": "output_text", "text": "Done", "annotations": [citation] }] } }),
        json!({ "type": "response.output_item.added", "item": { "type": "reasoning", "id": "rs_1" } }),
        json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_1",
                "summary": [{ "type": "summary_text", "text": "Sum" }] } }),
        json!({ "type": "response.output_item.done", "item": { "type": "function_call", "id": "fc_1",
                "call_id": "call_1", "name": "weather", "arguments": "{}" } }),
    ];
    mock_sse(
        &server,
        events
            .iter()
            .map(|event| sse_event(&event.to_string()))
            .collect(),
    )
    .await;
    let result = make_model(&server, "m")
        .do_stream(&default_options(wiremock_test_prompt()))
        .await
        .unwrap();
    let metadata: Vec<Value> = collect_stream(result)
        .await
        .into_iter()
        .filter_map(|part| match part {
            StreamPart::TextEnd {
                provider_metadata, ..
            }
            | StreamPart::ReasoningEnd {
                provider_metadata, ..
            } => Some(json!(provider_metadata)),
            StreamPart::ToolCall(call) => Some(json!(call.provider_metadata)),
            _ => None,
        })
        .collect();
    assert_eq!(
        metadata,
        vec![
            json!({ "lmstudio": { "itemId": "msg_1", "annotations": [citation] } }),
            json!({ "lmstudio": { "itemId": "rs_1",
                "reasoningSummary": [{ "type": "summary_text", "text": "Sum" }],
                "reasoningContent": null } }),
            json!({ "lmstudio": { "itemId": "fc_1" } }),
        ]
    );
}

/// TS: `doGenerate` (open-responses-language-model.ts:448-473): a reasoning
/// item without content yields one reasoning part with the joined summary
/// text, so summary and encrypted-only items are kept.
#[tokio::test]
async fn summary_only_reasoning_items_yield_reasoning() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "resp_1", "created_at": 1, "model": "m", "status": "completed",
            "output": [
                { "type": "reasoning", "id": "rs_1",
                  "summary": [{ "type": "summary_text", "text": "First. " },
                              { "type": "summary_text", "text": "Second." }] },
                { "type": "reasoning", "id": "rs_2", "summary": [], "content": [],
                  "encrypted_content": "enc" }
            ]
        }),
    )
    .await;
    let result = make_model(&server, "m")
        .do_generate(&default_options(wiremock_test_prompt()))
        .await
        .unwrap();
    let reasoning: Vec<Value> = result
        .content
        .iter()
        .map(|part| match part {
            GenerateContent::Reasoning(part) => {
                json!({ "text": part.text, "metadata": part.provider_metadata })
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        reasoning,
        vec![
            json!({ "text": "First. Second.", "metadata": { "lmstudio": { "itemId": "rs_1",
                "reasoningSummary": [{ "type": "summary_text", "text": "First. " },
                                     { "type": "summary_text", "text": "Second." }],
                "reasoningContent": null } } }),
            json!({ "text": "", "metadata": { "lmstudio": { "itemId": "rs_2",
                "reasoningSummary": [], "reasoningContent": [],
                "reasoningEncryptedContent": "enc" } } }),
        ]
    );
}

/// TS: `doStream` (open-responses-language-model.ts:878-892): reasoning
/// summary text deltas stream as reasoning deltas, like LM Studio's
/// `response.reasoning_text.delta`.
#[tokio::test]
async fn streams_reasoning_summary_deltas() {
    let server = MockServer::start().await;
    let events = [
        json!({ "type": "response.output_item.added", "item": { "type": "reasoning", "id": "rs_1" } }),
        json!({ "type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "delta": "Think" }),
        json!({ "type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "delta": "ing." }),
    ];
    mock_sse(
        &server,
        events
            .iter()
            .map(|event| sse_event(&event.to_string()))
            .collect(),
    )
    .await;
    let result = make_model(&server, "m")
        .do_stream(&default_options(wiremock_test_prompt()))
        .await
        .unwrap();
    let deltas: Vec<(String, String)> = collect_stream(result)
        .await
        .into_iter()
        .filter_map(|part| match part {
            StreamPart::ReasoningDelta { id, delta, .. } => Some((id, delta)),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        [
            ("rs_1".into(), "Think".into()),
            ("rs_1".into(), "ing.".into())
        ]
    );
}

/// TS: `doStream` flush (open-responses-language-model.ts:997-1003): a
/// reasoning item that never got `response.output_item.done` ends with its
/// own item id.
#[tokio::test]
async fn unfinished_reasoning_ends_with_its_item_id() {
    let server = MockServer::start().await;
    let added = json!({ "type": "response.output_item.added",
                        "item": { "type": "reasoning", "id": "rs_open" } });
    mock_sse(&server, sse_event(&added.to_string())).await;
    let result = make_model(&server, "m")
        .do_stream(&default_options(wiremock_test_prompt()))
        .await
        .unwrap();
    let ends: Vec<String> = collect_stream(result)
        .await
        .into_iter()
        .filter_map(|part| match part {
            StreamPart::ReasoningEnd { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(ends, ["rs_open"]);
}

/// TS: `doStream` (open-responses-language-model.ts:758-781): a streamed tool
/// call takes the completed item's `arguments` unless they are empty, so a
/// stream without argument events still has the input.
#[tokio::test]
async fn stream_tool_calls_take_the_completed_arguments() {
    let server = MockServer::start().await;
    let events = [
        json!({ "type": "response.output_item.added", "item": { "type": "function_call",
                "id": "fc_1", "call_id": "call_1", "name": "write_svg", "arguments": "" } }),
        json!({ "type": "response.output_item.done", "item": { "type": "function_call",
                "id": "fc_1", "call_id": "call_1", "name": "write_svg",
                "arguments": "{\"path\":\"icon.svg\"}" } }),
    ];
    mock_sse(
        &server,
        events
            .iter()
            .map(|event| sse_event(&event.to_string()))
            .collect(),
    )
    .await;
    let result = make_model(&server, "m")
        .do_stream(&default_options(wiremock_test_prompt()))
        .await
        .unwrap();
    let inputs: Vec<String> = collect_stream(result)
        .await
        .into_iter()
        .filter_map(|part| match part {
            StreamPart::ToolCall(call) => Some(call.input),
            _ => None,
        })
        .collect();
    assert_eq!(inputs, [r#"{"path":"icon.svg"}"#]);
}

/// TS: `doStream` (open-responses-language-model.ts:957-990): `response.failed`
/// with an error and in-stream `error` events become error parts carrying
/// the event as `data`, and the stream finishes with `error`.
#[tokio::test]
async fn stream_errors_become_error_parts() {
    let failed = json!({ "type": "response.failed", "response": { "status": "failed",
                         "error": { "code": "429", "message": "Rate limit reached" } } });
    let error =
        json!({ "type": "error", "error": { "code": "503", "message": "Service unavailable" } });
    let text = json!({ "type": "response.output_item.added", "item": { "type": "message", "id": "msg_1" } });
    for (events, message, code) in [
        (vec![failed.clone()], "Rate limit reached", "429"),
        (vec![text, error.clone()], "Service unavailable", "503"),
    ] {
        let server = MockServer::start().await;
        mock_sse(
            &server,
            events
                .iter()
                .map(|event| sse_event(&event.to_string()))
                .collect(),
        )
        .await;
        let result = make_model(&server, "m")
            .do_stream(&default_options(wiremock_test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;
        let error = parts
            .iter()
            .find_map(|part| match part {
                StreamPart::Error {
                    error: aimux_core::error::AiMuxError::ApiCall(error),
                } => Some(error),
                _ => None,
            })
            .expect("an error part");
        assert_eq!(error.message, message);
        assert_eq!(error.provider_code.as_deref(), Some(code));
        assert_eq!(error.data.as_ref(), events.last());
        let Some(StreamPart::Finish { finish_reason, .. }) = parts.last() else {
            panic!("no finish");
        };
        assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
        assert_eq!(finish_reason.raw.as_deref(), Some(code));
    }
}

/// TS: `getArgs` (open-responses-language-model.ts:304-315): a JSON response
/// format without a schema is sent as `{ type: "json_object" }`.
#[tokio::test]
async fn schema_less_json_is_a_json_object() {
    let server = MockServer::start().await;
    mock_json(&server, lmstudio_basic_json()).await;
    let options = CallOptions {
        response_format: Some(ResponseFormat::Json {
            schema: None,
            name: None,
            description: None,
        }),
        ..default_options(wiremock_test_prompt())
    };
    make_model(&server, "m")
        .do_generate(&options)
        .await
        .unwrap();
    let request = &server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["text"], json!({ "format": { "type": "json_object" } }));
}

/// TS: `getArgs` (open-responses-language-model.ts:166-300): with no
/// extension to encode them, provider-defined tools are left out with an
/// `unsupported` warning each, and a tool choice naming one is omitted.
#[tokio::test]
async fn provider_defined_tools_are_unsupported() {
    let server = MockServer::start().await;
    mock_json(&server, lmstudio_basic_json()).await;
    let options: CallOptions = serde_json::from_value(json!({
        "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }],
        "tools": [
            { "type": "provider", "id": "openai.web_search", "name": "web_search", "args": {} },
            { "type": "provider", "id": "openai.file_search", "name": "file_search", "args": {} }
        ],
        "toolChoice": { "type": "tool", "toolName": "web_search" }
    }))
    .unwrap();
    let result = make_model(&server, "m")
        .do_generate(&options)
        .await
        .unwrap();
    assert_eq!(
        json!(result.warnings),
        json!([
            { "type": "unsupported", "feature": "provider-defined tool openai.web_search" },
            { "type": "unsupported", "feature": "provider-defined tool openai.file_search" }
        ])
    );
    let request = &server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
}

/// TS: `getArgs` (open-responses-language-model.ts:330-347):
/// `providerOptions.<name>.reasoningEffort` is sent as is and wins over the
/// top-level `reasoning`.
#[tokio::test]
async fn reads_the_reasoning_effort_provider_option() {
    let server = MockServer::start().await;
    mock_json(&server, lmstudio_basic_json()).await;
    let options: CallOptions = serde_json::from_value(json!({
        "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }],
        "reasoning": "low",
        "providerOptions": { "lmstudio": { "reasoningEffort": "max", "reasoningSummary": "detailed" } }
    }))
    .unwrap();
    make_model(&server, "m")
        .do_generate(&options)
        .await
        .unwrap();
    let request = &server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        body["reasoning"],
        json!({ "effort": "max", "summary": "detailed" })
    );
}

/// TS: `doGenerate` (open-responses-language-model.ts:411-426): an `error` in
/// a successful response body fails with status 400, not retryable, and the
/// error object as `data`.
#[tokio::test]
async fn response_errors_report_status_400() {
    let server = MockServer::start().await;
    let error = json!({ "code": "busy", "message": "Try again later.", "status_code": 503 });
    mock_json(
        &server,
        json!({ "error": error, "status": "failed", "output": [] }),
    )
    .await;
    let result = make_model(&server, "m")
        .do_generate(&default_options(wiremock_test_prompt()))
        .await;
    let Err(aimux_core::error::AiMuxError::ApiCall(api_error)) = result else {
        panic!("expected an API call error, got {result:?}");
    };
    assert_eq!(api_error.message, "Try again later.");
    assert_eq!(api_error.status_code, Some(400));
    assert!(!api_error.is_retryable);
    assert_eq!(api_error.data, Some(error));
}

/// TS: `doGenerate` (open-responses-language-model.ts:428-441): a response
/// without `output` fails as an API call error with status 500, not
/// retryable.
#[tokio::test]
async fn missing_output_is_an_api_call_error() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({ "id": "resp_1", "status": "incomplete",
                "incomplete_details": { "reason": "content_filter" } }),
    )
    .await;
    let result = make_model(&server, "m")
        .do_generate(&default_options(wiremock_test_prompt()))
        .await;
    let Err(aimux_core::error::AiMuxError::ApiCall(api_error)) = result else {
        panic!("expected an API call error, got {result:?}");
    };
    assert_eq!(
        api_error.message,
        "Responses API returned no output (content_filter)"
    );
    assert_eq!(api_error.status_code, Some(500));
    assert!(!api_error.is_retryable);
}
