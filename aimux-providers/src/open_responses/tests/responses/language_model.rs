//! Port of `@ai-sdk/open-responses@2.0.58`
//! `src/responses/open-responses-language-model.test.ts`.
//!
//! The upstream `createTestServer` is replaced by the scripted
//! [`MockFetch`] transport; `server.calls[n].requestBodyJson` is
//! `mock.seen()[n].json_body()`. Upstream builds `OpenResponsesLanguageModel`
//! directly with provider `lmstudio`; here the model comes from
//! `create_open_responses` with the provider name `lmstudio`, which gives the
//! same providerOptions key and the same request URL.

use std::sync::Arc;

use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;

use crate::open_responses::{
    OpenResponsesModel, OpenResponsesProviderSettings, create_open_responses,
};
use crate::test_support::mock_fetch::{Canned, MockFetch};

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
