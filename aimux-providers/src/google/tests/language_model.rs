//! `google-language-model.test.ts` of `@ai-sdk/google` 4.0.87.
//!
//! Upstream drives `GoogleLanguageModel` against `createTestServer`; here the
//! same models come from [`create_google`] (or `GoogleModel::from_config` where
//! upstream passes a custom provider string) with a scripted transport
//! ([`MockFetch`]). Results, stream parts and errors are compared in upstream's
//! JSON shape (`undefined` values are omitted keys).

use std::collections::BTreeMap;
use std::sync::Arc;

use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::google::api_types::Candidate;
use crate::google::options::Namespace;
use crate::google::{GoogleModel, GoogleProvider, GoogleProviderSettings, create_google};
use crate::shared::{AuthScheme, Credential, EndpointConfig, EndpointHeaders, ProviderHeaders};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

// ── doGenerate helpers ───────────────────────────────────────────────────────

/// The default base URL every upstream `doGenerate` request goes to.
const GENERATE_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// `TEST_PROMPT` of the upstream file.
fn generate_test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

/// `SAFETY_RATINGS` of the upstream file.
fn generate_safety_ratings() -> Value {
    json!([
        { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "probability": "NEGLIGIBLE" },
        { "category": "HARM_CATEGORY_HATE_SPEECH", "probability": "NEGLIGIBLE" },
        { "category": "HARM_CATEGORY_HARASSMENT", "probability": "NEGLIGIBLE" },
        { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "probability": "NEGLIGIBLE" },
    ])
}

/// `COMPLETE_USAGE_METADATA` of the upstream file.
fn generate_complete_usage_metadata() -> Value {
    json!({
        "promptTokenCount": 12,
        "cachedContentTokenCount": 4,
        "candidatesTokenCount": 71,
        "toolUsePromptTokenCount": 65,
        "thoughtsTokenCount": 89,
        "totalTokenCount": 237,
        "promptTokensDetails": [{ "modality": "TEXT", "tokenCount": 12, "nestedSentinel": "prompt" }],
        "cacheTokensDetails": [{ "modality": "TEXT", "tokenCount": 4, "nestedSentinel": "cache" }],
        "candidatesTokensDetails": [{ "modality": "TEXT", "tokenCount": 71, "nestedSentinel": "candidate" }],
        "toolUsePromptTokensDetails": [{ "modality": "TEXT", "tokenCount": 65, "nestedSentinel": "tool" }],
        "serviceTier": "standard",
        "topLevelSentinel": "preserve-me",
    })
}

/// An upstream `__fixtures__/<name>.json` response body.
fn generate_fixture(name: &str) -> Value {
    let text = match name {
        "google-text" => include_str!("fixtures/google-text.json"),
        "google-tool-call" => include_str!("fixtures/google-tool-call.json"),
        "google-reasoning-gemini3" => include_str!("fixtures/google-reasoning-gemini3.json"),
        "google-tool-call-gemini3" => include_str!("fixtures/google-tool-call-gemini3.json"),
        other => panic!("unknown fixture {other}"),
    };
    serde_json::from_str(text).expect("fixture is JSON")
}

/// A `toMatchSnapshot()` value of the upstream `doGenerate` tests.
fn generate_snapshot(name: &str) -> Value {
    let text = match name {
        "do_generate_text_extract_text_response" => {
            include_str!("snapshots/do_generate_text_extract_text_response.json")
        }
        "do_generate_text_extract_usage" => {
            include_str!("snapshots/do_generate_text_extract_usage.json")
        }
        "do_generate_tool_call_extract_tool_calls" => {
            include_str!("snapshots/do_generate_tool_call_extract_tool_calls.json")
        }
        "do_generate_reasoning_gemini3_extract_reasoning" => {
            include_str!("snapshots/do_generate_reasoning_gemini3_extract_reasoning.json")
        }
        "do_generate_tool_call_gemini3_extract_tool_call" => {
            include_str!("snapshots/do_generate_tool_call_gemini3_extract_tool_call.json")
        }
        other => panic!("unknown snapshot {other}"),
    };
    serde_json::from_str(text).expect("snapshot is JSON")
}

/// The `json-value` response of the upstream test server: the body as
/// compact JSON with its `content-type` and `content-length`, plus `headers`.
fn generate_json_value_response_with_headers(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let bytes = serde_json::to_vec(body).expect("body serializes");
    let mut response_headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), bytes.len().to_string()),
    ];
    response_headers.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    Canned {
        status: 200,
        headers: response_headers,
        body: bytes,
    }
}

/// [`generate_json_value_response_with_headers`] without extra headers.
fn generate_json_value_response(body: &Value) -> Canned {
    generate_json_value_response_with_headers(body, &[])
}

/// A mock transport answering one request with `body` as JSON.
fn generate_mock_with_body(body: &Value) -> Arc<MockFetch> {
    MockFetch::new(vec![generate_json_value_response(body)])
}

/// A mock transport answering one request with an upstream fixture.
fn generate_mock_with_fixture(name: &str) -> Arc<MockFetch> {
    generate_mock_with_body(&generate_fixture(name))
}

/// The body `prepareJsonResponse({ content, groundingMetadata })` serves.
fn generate_prepared_json_body(content: &str, grounding_metadata: Option<Value>) -> Value {
    let mut candidate = json!({
        "content": { "parts": [{ "text": content }], "role": "model" },
        "finishReason": "STOP",
        "index": 0,
        "safetyRatings": generate_safety_ratings(),
    });
    if let Some(grounding_metadata) = grounding_metadata {
        candidate["groundingMetadata"] = grounding_metadata;
    }
    json!({
        "candidates": [candidate],
        "promptFeedback": { "safetyRatings": generate_safety_ratings() },
        "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3 },
    })
}

/// `createGoogle({ apiKey: 'test-api-key', generateId: () => 'test-id' })`
/// with the mock transport (the upstream `provider`).
fn generate_test_provider(mock: &Arc<MockFetch>) -> GoogleProvider {
    create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(mock.transport()),
        generate_id: Some(Arc::new(|| "test-id".into())),
        ..Default::default()
    })
    .expect("valid settings")
}

/// `new GoogleLanguageModel(modelId, { provider, baseURL, headers,
/// generateId: () => 'test-id' })`: Vertex AI behavior when `provider`
/// contains `vertex`, as upstream's `isVertexProvider`.
fn generate_direct_model(
    model_id: &str,
    provider: &str,
    headers: &[(&str, &str)],
    mock: &Arc<MockFetch>,
) -> GoogleModel {
    let provider_headers = ProviderHeaders::new(
        Credential::None,
        AuthScheme::Bearer,
        headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect(),
        None,
    );
    generate_direct_model_with_endpoint_headers(
        model_id,
        provider,
        EndpointHeaders::Provider(provider_headers),
        mock,
    )
}

/// [`generate_direct_model`] whose config `headers` is an async function.
fn generate_direct_model_with_async_headers(
    model_id: &str,
    provider: &str,
    headers: &[(&str, &str)],
    mock: &Arc<MockFetch>,
) -> GoogleModel {
    let produced: HeaderMapOpt = headers
        .iter()
        .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
        .collect();
    let endpoint_headers = EndpointHeaders::WithUserHeaders {
        provider: ProviderHeaders::new(Credential::None, AuthScheme::Bearer, Vec::new(), None),
        user: Some(Resolvable::AsyncFn(Arc::new(move || {
            let produced = produced.clone();
            Box::pin(async move { Ok(produced) })
        }))),
        user_agent_suffix: None,
    };
    generate_direct_model_with_endpoint_headers(model_id, provider, endpoint_headers, mock)
}

fn generate_direct_model_with_endpoint_headers(
    model_id: &str,
    provider: &str,
    headers: EndpointHeaders,
    mock: &Arc<MockFetch>,
) -> GoogleModel {
    let namespace = if provider.contains("vertex") {
        Namespace::Vertex
    } else {
        Namespace::Google
    };
    let config = EndpointConfig::fixed(
        provider.to_string(),
        GENERATE_BASE_URL.to_string(),
        headers,
        Some(mock.transport()),
        None,
    );
    GoogleModel::from_config(model_id.to_string(), config, namespace)
        .with_generate_id(Some(Arc::new(|| "test-id".into())))
}

/// `LanguageModelV4CallOptions` written as upstream JSON.
fn generate_call_options(options: Value) -> CallOptions {
    serde_json::from_value(options).expect("call options deserialize")
}

/// `model.doGenerate(options)`, which must succeed.
async fn generate_with(model: &impl LanguageModel, options: Value) -> GenerateResult {
    model
        .do_generate(&generate_call_options(options))
        .await
        .expect("doGenerate succeeds")
}

/// `model.doGenerate({ prompt: TEST_PROMPT })`.
async fn generate_with_test_prompt(model: &impl LanguageModel) -> GenerateResult {
    generate_with(model, json!({ "prompt": generate_test_prompt() })).await
}

/// The JSON of any serializable value.
fn generate_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("value serializes")
}

/// The single request the mock saw.
fn generate_only_call(mock: &MockFetch) -> Seen {
    let seen = mock.seen();
    assert_eq!(seen.len(), 1, "expected exactly one request");
    seen[0].clone()
}

/// The request body of the single request the mock saw.
fn generate_only_body(mock: &MockFetch) -> Value {
    generate_only_call(mock).json_body()
}

/// `server.calls[0].requestHeaders`: the headers without `user-agent`, which
/// the test server reports separately as `requestUserAgent`.
fn generate_request_headers(seen: &Seen) -> BTreeMap<String, String> {
    seen.headers
        .iter()
        .filter(|(name, _)| name.as_str() != "user-agent")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// `expect(actual).toMatchObject(expected)`.
fn generate_assert_match_object(actual: &Value, expected: &Value) {
    fn matches(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => {
                expected.iter().all(|(key, expected)| {
                    actual
                        .get(key)
                        .is_some_and(|actual| matches(actual, expected))
                })
            }
            (Value::Array(actual), Value::Array(expected)) => {
                actual.len() == expected.len()
                    && actual
                        .iter()
                        .zip(expected)
                        .all(|(actual, expected)| matches(actual, expected))
            }
            (actual, expected) => actual == expected,
        }
    }
    assert!(
        matches(actual, expected),
        "expected {actual:#} to match object {expected:#}"
    );
}

/// The `type: 'provider'` tool `{ id, name, args: {} }`.
fn generate_provider_tool(id: &str, name: &str) -> Value {
    json!({ "type": "provider", "id": id, "name": name, "args": {} })
}

/// The body upstream's `simpleResponseBody` of the top-level reasoning tests.
fn generate_simple_response_body() -> Value {
    json!({
        "candidates": [{
            "content": { "parts": [{ "text": "response" }], "role": "model" },
            "finishReason": "STOP",
            "safetyRatings": generate_safety_ratings(),
        }],
        "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3 },
    })
}

/// A one-candidate body answering `done`, as the Vertex tool result tests
/// serve.
fn generate_done_body() -> Value {
    json!({
        "candidates": [{
            "content": { "parts": [{ "text": "done" }], "role": "model" },
            "finishReason": "STOP",
            "index": 0,
        }],
    })
}

/// A tool message with one `viewFiles` content result holding `value`.
fn generate_view_files_prompt(value: Value) -> Value {
    json!([{
        "role": "tool",
        "content": [{
            "type": "tool-result",
            "toolCallId": "testCallId",
            "toolName": "viewFiles",
            "output": { "type": "content", "value": value },
        }],
    }])
}

/// The request body of `model.doGenerate({ prompt: TEST_PROMPT,
/// providerOptions: { google: options } })` against `google-text`.
async fn generate_body_with_google_options(options: Value) -> Value {
    let mock = generate_mock_with_fixture("google-text");
    let model = generate_test_provider(&mock).chat("gemini-pro");
    generate_with(
        &model,
        json!({ "prompt": generate_test_prompt(), "providerOptions": { "google": options } }),
    )
    .await;
    generate_only_body(&mock)
}

/// The request body of `modelId.doGenerate({ prompt: TEST_PROMPT, reasoning })`
/// against `simpleResponseBody`, and the result.
async fn generate_body_with_reasoning(model_id: &str, reasoning: &str) -> (Value, GenerateResult) {
    let mock = generate_mock_with_body(&generate_simple_response_body());
    let model = generate_test_provider(&mock).chat(model_id);
    let result = generate_with(
        &model,
        json!({ "prompt": generate_test_prompt(), "reasoning": reasoning }),
    )
    .await;
    (generate_only_body(&mock), result)
}

// ── Shared helpers of the stream / schema ports ──────────────────────────────

/// The base URL of the upstream test server.
const STREAM_TEST_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// `TEST_PROMPT`.
fn stream_test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

/// `SAFETY_RATINGS`.
fn stream_safety_ratings() -> Value {
    json!([
        { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "probability": "NEGLIGIBLE" },
        { "category": "HARM_CATEGORY_HATE_SPEECH", "probability": "NEGLIGIBLE" },
        { "category": "HARM_CATEGORY_HARASSMENT", "probability": "NEGLIGIBLE" },
        { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "probability": "NEGLIGIBLE" },
    ])
}

/// `COMPLETE_USAGE_METADATA`.
fn stream_complete_usage_metadata() -> Value {
    json!({
        "promptTokenCount": 12,
        "cachedContentTokenCount": 4,
        "candidatesTokenCount": 71,
        "toolUsePromptTokenCount": 65,
        "thoughtsTokenCount": 89,
        "totalTokenCount": 237,
        "promptTokensDetails": [
            { "modality": "TEXT", "tokenCount": 12, "nestedSentinel": "prompt" },
        ],
        "cacheTokensDetails": [
            { "modality": "TEXT", "tokenCount": 4, "nestedSentinel": "cache" },
        ],
        "candidatesTokensDetails": [
            { "modality": "TEXT", "tokenCount": 71, "nestedSentinel": "candidate" },
        ],
        "toolUsePromptTokensDetails": [
            { "modality": "TEXT", "tokenCount": 65, "nestedSentinel": "tool" },
        ],
        "serviceTier": "standard",
        "topLevelSentinel": "preserve-me",
    })
}

/// A function tool of the upstream calls (`{ type: 'function', name,
/// inputSchema }`) whose schema is `{ <property>: string }`, required.
fn stream_string_function_tool(name: &str, property: &str) -> Value {
    json!({
        "type": "function",
        "name": name,
        "inputSchema": {
            "type": "object",
            "properties": { property: { "type": "string" } },
            "required": [property],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#",
        },
    })
}

/// A provider tool of the upstream calls (`{ type: 'provider', id, name,
/// args: {} }`).
fn stream_provider_tool(id: &str, name: &str) -> Value {
    json!({ "type": "provider", "id": id, "name": name, "args": {} })
}

/// One `data: <JSON.stringify(value)>\n\n` SSE chunk.
fn sse_data_chunk(value: &Value) -> String {
    format!("data: {value}\n\n")
}

/// `prepareChunksFixtureResponse`: one SSE chunk per non-empty fixture line.
fn sse_fixture_chunks(fixture: &str) -> Vec<String> {
    fixture
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("data: {line}\n\n"))
        .collect()
}

/// `prepareStreamResponse`: one chunk per text, `STOP` on every chunk and the
/// usage on the last one only.
fn sse_text_chunks(
    content: &[&str],
    grounding_metadata: Option<Value>,
    url_context_metadata: Option<Value>,
) -> Vec<String> {
    content
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let mut candidate = json!({
                "content": { "parts": [{ "text": text }], "role": "model" },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": stream_safety_ratings(),
            });
            if let Some(grounding_metadata) = &grounding_metadata {
                candidate["groundingMetadata"] = grounding_metadata.clone();
            }
            if let Some(url_context_metadata) = &url_context_metadata {
                candidate["urlContextMetadata"] = url_context_metadata.clone();
            }
            let mut chunk = json!({ "candidates": [candidate] });
            if index == content.len() - 1 {
                chunk["usageMetadata"] = json!({
                    "promptTokenCount": 294,
                    "candidatesTokenCount": 233,
                    "totalTokenCount": 527,
                });
            }
            sse_data_chunk(&chunk)
        })
        .collect()
}

/// The test server's `stream-chunks` response: the SSE headers it always
/// sends, then `headers`.
fn sse_stream_response(chunks: Vec<String>, headers: &[(&str, &str)]) -> Canned {
    let mut all_headers: Vec<(String, String)> = vec![
        ("content-type".into(), "text/event-stream".into()),
        ("cache-control".into(), "no-cache".into()),
        ("connection".into(), "keep-alive".into()),
    ];
    all_headers.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    Canned {
        status: 200,
        headers: all_headers,
        body: chunks.concat().into_bytes(),
    }
}

/// A mock transport answering one request with the SSE `chunks`.
fn sse_stream_mock(chunks: Vec<String>) -> Arc<MockFetch> {
    MockFetch::new(vec![sse_stream_response(chunks, &[])])
}

/// `createGoogle({ apiKey: 'test-api-key', generateId: () => 'test-id' })`
/// on the mock transport.
fn stream_test_provider(mock: &Arc<MockFetch>) -> GoogleProvider {
    create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".into()),
        generate_id: Some(Arc::new(|| "test-id".into())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("valid settings")
}

/// `new GoogleLanguageModel(modelId, { provider, baseURL, headers:
/// { 'x-goog-api-key': 'test-api-key' }, generateId: () => 'test-id' })` on
/// the mock transport. Upstream picks the Vertex metadata keys when the
/// provider string contains `vertex`; aimux takes that as the namespace.
fn stream_test_model_for_provider(
    model_id: &str,
    provider: &str,
    mock: &Arc<MockFetch>,
) -> GoogleModel {
    let namespace = if provider.contains("vertex") {
        Namespace::Vertex
    } else {
        Namespace::Google
    };
    GoogleModel::from_config(
        model_id.to_owned(),
        EndpointConfig::fixed(
            provider.to_owned(),
            STREAM_TEST_BASE_URL.to_owned(),
            ProviderHeaders::new(
                Credential::None,
                AuthScheme::Bearer,
                vec![("x-goog-api-key".into(), "test-api-key".into())],
                None,
            ),
            Some(mock.transport()),
            None,
        ),
        namespace,
    )
    .with_generate_id(Some(Arc::new(|| "test-id".into())))
}

/// The `google.vertex.chat` model of the upstream Vertex cases.
fn vertex_stream_test_model(model_id: &str, mock: &Arc<MockFetch>) -> GoogleModel {
    stream_test_model_for_provider(model_id, "google.vertex.chat", mock)
}

/// Call options from their upstream (V4) JSON form.
fn stream_call_options(options: Value) -> CallOptions {
    serde_json::from_value(options).expect("call options deserialize")
}

/// `{ prompt: TEST_PROMPT, ...extra }` as call options.
fn stream_call_options_with(extra: Value) -> CallOptions {
    let mut options = json!({ "prompt": stream_test_prompt() });
    if let Value::Object(extra) = extra {
        options.as_object_mut().unwrap().extend(extra);
    }
    stream_call_options(options)
}

/// `convertReadableStreamToArray(stream)`, each part as its V4 JSON.
async fn stream_parts_of(result: StreamResult) -> Vec<Value> {
    result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(|part| serde_json::to_value(part.expect("stream part")).expect("part serializes"))
        .collect()
}

/// `model.doStream(options)` then all of its parts.
async fn stream_events_of(model: &dyn LanguageModel, options: Value) -> Vec<Value> {
    let result = model
        .do_stream(&stream_call_options_with(options))
        .await
        .expect("doStream succeeds");
    stream_parts_of(result).await
}

/// `events.filter(e => e.type === <type>)`.
fn stream_events_of_type(events: &[Value], types: &[&str]) -> Vec<Value> {
    events
        .iter()
        .filter(|event| types.iter().any(|ty| event["type"] == *ty))
        .cloned()
        .collect()
}

/// `events.find(e => e.type === 'finish')`.
fn stream_finish_event(events: &[Value]) -> Value {
    events
        .iter()
        .find(|event| event["type"] == "finish")
        .cloned()
        .expect("a finish event")
}

/// The JSON body of the `index`-th request the mock saw
/// (`server.calls[index].requestBodyJson`).
fn stream_request_body(mock: &Arc<MockFetch>, index: usize) -> Value {
    mock.seen()[index].json_body()
}

/// `expect(actual).toMatchObject(expected)`: every key of an expected object
/// is present and matches, arrays match element by element with the same
/// length, everything else is equal.
fn assert_stream_match_object(actual: &Value, expected: &Value) {
    fn walk(actual: &Value, expected: &Value, path: &str) {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => {
                for (key, expected) in expected {
                    let actual = actual
                        .get(key)
                        .unwrap_or_else(|| panic!("missing key {path}.{key}"));
                    walk(actual, expected, &format!("{path}.{key}"));
                }
            }
            (Value::Array(actual), Value::Array(expected)) => {
                assert_eq!(actual.len(), expected.len(), "array length at {path}");
                for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                    walk(actual, expected, &format!("{path}[{index}]"));
                }
            }
            _ => assert_eq!(actual, expected, "value at {path}"),
        }
    }
    walk(actual, expected, "$");
}

/// A `toMatchSnapshot()` value stored under `snapshots/`.
fn stream_snapshot(text: &str) -> Value {
    serde_json::from_str(text).expect("snapshot is JSON")
}

/// `groundingMetadataSchema.safeParse(metadata).success`: aimux has no
/// standalone schema, so the metadata is parsed where the response carries
/// it (`candidates[].groundingMetadata`).
fn grounding_metadata_parses(metadata: Value) -> bool {
    serde_json::from_value::<Candidate>(json!({ "groundingMetadata": metadata })).is_ok()
}

/// `urlContextMetadataSchema.safeParse(output).success`, parsed where the
/// response carries it (`candidates[].urlContextMetadata`).
fn url_context_metadata_parses(output: Value) -> bool {
    serde_json::from_value::<Candidate>(json!({ "urlContextMetadata": output })).is_ok()
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should use the custom code execution tool name for generated and streamed results
    #[tokio::test]
    async fn should_use_the_custom_code_execution_tool_name_for_generated_and_streamed_results() {
        let response = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "executableCode": { "language": "PYTHON", "code": "print('ok')\nprint(1/0)" } },
                        { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "ok\n" } },
                        { "codeExecutionResult": { "outcome": "OUTCOME_FAILED", "output": "ZeroDivisionError: division by zero\n" } },
                    ],
                    "role": "model",
                },
                "finishReason": "STOP",
            }],
        });
        let mock = MockFetch::new(vec![
            Canned::json(&response),
            Canned {
                status: 200,
                headers: vec![("content-type".into(), "text/event-stream".into())],
                body: format!("data: {response}\n\n").into_bytes(),
            },
        ]);
        let test_provider = generate_test_provider(&mock);
        let tools = json!([{ "type": "provider", "id": "google.code_execution", "name": "CodeExecutionTool", "args": {} }]);

        let result = generate_with(
            &test_provider.chat("gemini-2.0-pro"),
            json!({ "tools": tools, "prompt": generate_test_prompt() }),
        )
        .await;

        assert_eq!(mock.seen().len(), 1);
        let expected = json!([
            {
                "input": "{\"language\":\"PYTHON\",\"code\":\"print('ok')\\nprint(1/0)\"}",
                "providerExecuted": true,
                "toolCallId": "test-id",
                "toolName": "CodeExecutionTool",
                "type": "tool-call",
            },
            {
                "result": { "outcome": "OUTCOME_OK", "output": "ok\n" },
                "toolCallId": "test-id",
                "toolName": "CodeExecutionTool",
                "type": "tool-result",
            },
            {
                "result": { "outcome": "OUTCOME_FAILED", "output": "ZeroDivisionError: division by zero\n" },
                "toolCallId": "test-id",
                "toolName": "CodeExecutionTool",
                "type": "tool-result",
            },
        ]);
        assert_eq!(generate_json(&result.content), expected);

        let stream = test_provider
            .chat("gemini-2.0-pro")
            .do_stream(&generate_call_options(
                json!({ "tools": tools, "prompt": generate_test_prompt() }),
            ))
            .await
            .expect("doStream succeeds");
        let events: Vec<StreamPart> = stream
            .stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|event| event.expect("stream part"))
            .collect();
        let tool_events: Vec<Value> = events
            .iter()
            .filter(|event| matches!(event, StreamPart::ToolCall(_) | StreamPart::ToolResult(_)))
            .map(generate_json)
            .collect();

        assert_eq!(mock.seen().len(), 2);
        assert_eq!(Value::Array(tool_events), expected);
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should omit function call IDs from Vertex requests
    #[tokio::test]
    async fn should_omit_function_call_ids_from_vertex_requests() {
        let mock = generate_mock_with_body(&generate_prepared_json_body("done", None));
        let vertex_model = generate_direct_model(
            "gemini-pro",
            "google.vertex.chat",
            &[("x-goog-api-key", "test-api-key")],
            &mock,
        );

        generate_with(
            &vertex_model,
            json!({
                "prompt": [
                    { "role": "user", "content": [{ "type": "text", "text": "Look up the answer." }] },
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool-call",
                            "toolCallId": "call_repro_1",
                            "toolName": "lookup",
                            "input": { "query": "vertex function id repro" },
                        }],
                    },
                    {
                        "role": "tool",
                        "content": [{
                            "type": "tool-result",
                            "toolCallId": "call_repro_1",
                            "toolName": "lookup",
                            "output": { "type": "json", "value": { "answer": "known" } },
                        }],
                    },
                ],
            }),
        )
        .await;

        let request_body = generate_only_body(&mock);
        assert_eq!(
            request_body["contents"][1]["parts"][0]["functionCall"],
            json!({ "name": "lookup", "args": { "query": "vertex function id repro" } })
        );
        assert_eq!(
            request_body["contents"][2]["parts"][0]["functionResponse"],
            json!({ "name": "lookup", "response": { "name": "lookup", "content": { "answer": "known" } } })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should forward supported Vertex tool result URLs as function response file data
    #[tokio::test]
    async fn should_forward_supported_vertex_tool_result_urls_as_function_response_file_data() {
        // Upstream also passes `downloadToolResultFiles: { maxBytes,
        // supportsGoogleCloudStorageUrls: true }`; aimux's Vertex model always
        // forwards supported `gs://` URLs and has no such setting.
        let mock = generate_mock_with_body(&generate_done_body());
        let vertex_model = generate_direct_model(
            "gemini-3.7-flash",
            "google.vertex.chat",
            &[("x-goog-api-key", "test-api-key")],
            &mock,
        );

        generate_with(
            &vertex_model,
            json!({
                "prompt": generate_view_files_prompt(json!([
                    { "type": "text", "text": "hero.png activated" },
                    {
                        "type": "file",
                        "data": { "type": "url", "url": "gs://example-bucket/renditions/hero.png" },
                        "mediaType": "image/png",
                    },
                ])),
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock)["contents"][0],
            json!({
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": "viewFiles",
                        "response": { "name": "viewFiles", "content": "hero.png activated" },
                        "parts": [{
                            "fileData": {
                                "mimeType": "image/png",
                                "fileUri": "gs://example-bucket/renditions/hero.png",
                            },
                        }],
                    },
                }],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should forward supported Vertex %s tool result URLs as function response file data
    #[tokio::test]
    async fn should_forward_supported_vertex_media_type_tool_result_urls_as_function_response_file_data()
     {
        for media_type in [
            "image/png",
            "image/jpeg",
            "image/webp",
            "application/pdf",
            "text/plain",
        ] {
            let mock = generate_mock_with_body(&generate_done_body());
            let vertex_model = generate_direct_model(
                "gemini-3.7-flash",
                "google.vertex.chat",
                &[("x-goog-api-key", "test-api-key")],
                &mock,
            );

            generate_with(
                &vertex_model,
                json!({
                    "prompt": generate_view_files_prompt(json!([{
                        "type": "file",
                        "data": { "type": "url", "url": "gs://example-bucket/result" },
                        "mediaType": media_type,
                    }])),
                }),
            )
            .await;

            assert_eq!(
                generate_only_body(&mock)["contents"][0]["parts"][0]["functionResponse"]["parts"],
                json!([{ "fileData": { "mimeType": media_type, "fileUri": "gs://example-bucket/result" } }]),
                "{media_type}"
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should send PDF tool result data as inlineData for Gemini 2.5 legacy tool results
    #[tokio::test]
    async fn should_send_pdf_tool_result_data_as_inline_data_for_gemini_2_5_legacy_tool_results() {
        let mock = generate_mock_with_body(&generate_done_body());
        let model = generate_test_provider(&mock).chat("gemini-2.5-flash-lite");

        generate_with(
            &model,
            json!({
                "prompt": [{
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolName": "catalogSearch",
                        "toolCallId": "testCallId",
                        "output": {
                            "type": "content",
                            "value": [
                                { "type": "text", "text": "metadata" },
                                {
                                    "type": "file",
                                    "data": { "type": "data", "data": "JVBERi0xLjQK" },
                                    "mediaType": "application/pdf",
                                },
                            ],
                        },
                    }],
                }],
            }),
        )
        .await;

        let request_body = generate_only_body(&mock);
        let parts = &request_body["contents"][0]["parts"];
        let text_parts: Vec<&str> = parts
            .as_array()
            .expect("parts")
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect();

        assert!(
            !text_parts.iter().any(|text| text.contains("JVBERi0xLjQK")),
            "{text_parts:?}"
        );
        assert_eq!(
            parts,
            &json!([
                {
                    "functionResponse": {
                        "id": "testCallId",
                        "name": "catalogSearch",
                        "response": { "name": "catalogSearch", "content": "metadata" },
                    },
                },
                { "inlineData": { "mimeType": "application/pdf", "data": "JVBERi0xLjQK" } },
                { "text": "Tool executed successfully and returned this file as a response" },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should use newest request behavior for an unknown future Gemini model
    #[tokio::test]
    async fn should_use_newest_request_behavior_for_an_unknown_future_gemini_model() {
        let mock = generate_mock_with_body(&generate_done_body());
        let model = generate_test_provider(&mock).chat("gemini-99-pro-preview");

        generate_with(
            &model,
            json!({
                "reasoning": "high",
                "prompt": [
                    { "role": "user", "content": [{ "type": "text", "text": "Create a weather chart." }] },
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool-call",
                            "toolName": "createWeatherChart",
                            "toolCallId": "testCallId",
                            "input": { "location": "San Francisco" },
                        }],
                    },
                    {
                        "role": "tool",
                        "content": [{
                            "type": "tool-result",
                            "toolName": "createWeatherChart",
                            "toolCallId": "testCallId",
                            "output": {
                                "type": "content",
                                "value": [
                                    { "type": "text", "text": "Weather chart" },
                                    {
                                        "type": "file",
                                        "data": { "type": "data", "data": "iVBORw0KGgo=" },
                                        "mediaType": "image/png",
                                    },
                                ],
                            },
                        }],
                    },
                ],
            }),
        )
        .await;

        let request_body = generate_only_body(&mock);
        assert_eq!(
            request_body["generationConfig"]["thinkingConfig"],
            json!({ "thinkingLevel": "high" })
        );
        generate_assert_match_object(
            &request_body["contents"][1]["parts"][0],
            &json!({
                "functionCall": {
                    "id": "testCallId",
                    "name": "createWeatherChart",
                    "args": { "location": "San Francisco" },
                },
                "thoughtSignature": "skip_thought_signature_validator",
            }),
        );
        assert_eq!(
            request_body["contents"][2]["parts"],
            json!([{
                "functionResponse": {
                    "id": "testCallId",
                    "name": "createWeatherChart",
                    "response": { "name": "createWeatherChart", "content": "Weather chart" },
                    "parts": [{ "inlineData": { "mimeType": "image/png", "data": "iVBORw0KGgo=" } }],
                },
            }])
        );
    }

    mod text {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › text › should extract text response
        #[tokio::test]
        async fn should_extract_text_response() {
            let mock = generate_mock_with_fixture("google-text");
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            assert_eq!(
                generate_json(&result),
                generate_snapshot("do_generate_text_extract_text_response")
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › text › should extract usage
        #[tokio::test]
        async fn should_extract_usage() {
            let mock = generate_mock_with_fixture("google-text");
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            assert_eq!(
                generate_json(&result.usage),
                generate_snapshot("do_generate_text_extract_usage")
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › text › should preserve complete raw usage metadata
        #[tokio::test]
        async fn should_preserve_complete_raw_usage_metadata() {
            let mock = generate_mock_with_body(&json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "Blue." }], "role": "model" },
                    "finishReason": "STOP",
                }],
                "usageMetadata": generate_complete_usage_metadata(),
            }));
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            assert_eq!(
                generate_json(&result.usage),
                json!({
                    "inputTokens": { "total": 77, "noCache": 73, "cacheRead": 4 },
                    "outputTokens": { "total": 160, "text": 71, "reasoning": 89 },
                    "raw": generate_complete_usage_metadata(),
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › text › should send additional response information
        #[tokio::test]
        async fn should_send_additional_response_information() {
            let mock = generate_mock_with_fixture("google-text");
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            let response = result.response.expect("response");
            assert_eq!(
                json!({
                    "id": response.id,
                    "timestamp": response.timestamp.map(|timestamp| generate_json(&timestamp)),
                    "modelId": response.model_id,
                }),
                json!({ "id": "Un6LacrVMcjUxs0PmJfWoQc", "modelId": null, "timestamp": null })
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should reject invalid usage metadata values: %j
    #[tokio::test]
    async fn should_reject_invalid_usage_metadata_values() {
        for invalid_usage in [
            json!({ "toolUsePromptTokenCount": "65" }),
            json!({ "cacheTokensDetails": [{ "modality": "TEXT", "tokenCount": "4" }] }),
            json!({ "toolUsePromptTokensDetails": [{ "modality": 1, "tokenCount": 65 }] }),
        ] {
            let mut usage_metadata = json!({ "promptTokenCount": 12 });
            usage_metadata
                .as_object_mut()
                .unwrap()
                .extend(invalid_usage.as_object().unwrap().clone());
            let mock = generate_mock_with_body(&json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "Blue." }], "role": "model" },
                    "finishReason": "STOP",
                }],
                "usageMetadata": usage_metadata,
            }));
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = model
                .do_generate(&generate_call_options(
                    json!({ "prompt": generate_test_prompt() }),
                ))
                .await;

            assert!(result.is_err(), "{invalid_usage} was accepted");
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should handle MALFORMED_FUNCTION_CALL finish reason and empty content object
    #[tokio::test]
    async fn should_handle_malformed_function_call_finish_reason_and_empty_content_object() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{ "content": {}, "finishReason": "MALFORMED_FUNCTION_CALL" }],
            "usageMetadata": {
                "promptTokenCount": 9056,
                "totalTokenCount": 9056,
                "promptTokensDetails": [{ "modality": "TEXT", "tokenCount": 9056 }],
            },
            "modelVersion": "gemini-2.0-flash-lite",
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(generate_json(&result.content), json!([]));
        assert_eq!(
            generate_json(&result.finish_reason),
            json!({ "raw": "MALFORMED_FUNCTION_CALL", "unified": "error" })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should expose finishMessage in provider metadata
    #[tokio::test]
    async fn should_expose_finish_message_in_provider_metadata() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {},
                "finishReason": "MALFORMED_FUNCTION_CALL",
                "finishMessage": "Malformed function call: print(default_api.create(name='test'))",
            }],
            "usageMetadata": {
                "promptTokenCount": 130,
                "totalTokenCount": 130,
                "promptTokensDetails": [{ "modality": "TEXT", "tokenCount": 130 }],
            },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.provider_metadata)["google"]["finishMessage"],
            json!("Malformed function call: print(default_api.create(name='test'))")
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should expose null finishMessage in provider metadata when not present
    #[tokio::test]
    async fn should_expose_null_finish_message_in_provider_metadata_when_not_present() {
        let mock = generate_mock_with_body(&generate_prepared_json_body("test response", None));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        let metadata = generate_json(&result.provider_metadata);
        assert_eq!(metadata["google"].get("finishMessage"), Some(&Value::Null));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should send serviceTier in request body when specified
    #[tokio::test]
    async fn should_send_service_tier_in_request_body_when_specified() {
        let mock = generate_mock_with_body(&generate_prepared_json_body("test response", None));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({ "prompt": generate_test_prompt(), "providerOptions": { "google": { "serviceTier": "flex" } } }),
        )
        .await;

        generate_assert_match_object(
            &generate_only_body(&mock),
            &json!({ "serviceTier": "flex" }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should not send serviceTier in request body when not specified
    #[tokio::test]
    async fn should_not_send_service_tier_in_request_body_when_not_specified() {
        let mock = generate_mock_with_body(&generate_prepared_json_body("test response", None));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with_test_prompt(&model).await;

        assert!(generate_only_body(&mock).get("serviceTier").is_none());
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should send sharedRequestType as X-Vertex-AI-LLM-Shared-Request-Type header on Vertex
    #[tokio::test]
    async fn should_send_shared_request_type_as_x_vertex_ai_llm_shared_request_type_header_on_vertex()
     {
        let mock = generate_mock_with_body(&generate_prepared_json_body("test response", None));
        let vertex_model = generate_direct_model(
            "gemini-pro",
            "google.vertex.chat",
            &[("x-goog-api-key", "test-api-key")],
            &mock,
        );

        generate_with(
            &vertex_model,
            json!({ "prompt": generate_test_prompt(), "providerOptions": { "google": { "sharedRequestType": "flex" } } }),
        )
        .await;

        let call = generate_only_call(&mock);
        assert_eq!(
            generate_request_headers(&call)
                .get("x-vertex-ai-llm-shared-request-type")
                .map(String::as_str),
            Some("flex")
        );
        assert!(call.json_body().get("serviceTier").is_none());
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should send requestType as X-Vertex-AI-LLM-Request-Type header on Vertex
    #[tokio::test]
    async fn should_send_request_type_as_x_vertex_ai_llm_request_type_header_on_vertex() {
        let mock = generate_mock_with_body(&generate_prepared_json_body("test response", None));
        let vertex_model = generate_direct_model(
            "gemini-pro",
            "google.vertex.chat",
            &[("x-goog-api-key", "test-api-key")],
            &mock,
        );

        generate_with(
            &vertex_model,
            json!({
                "prompt": generate_test_prompt(),
                "providerOptions": { "google": { "sharedRequestType": "priority", "requestType": "shared" } },
            }),
        )
        .await;

        let headers = generate_request_headers(&generate_only_call(&mock));
        assert_eq!(
            headers
                .get("x-vertex-ai-llm-shared-request-type")
                .map(String::as_str),
            Some("priority")
        );
        assert_eq!(
            headers
                .get("x-vertex-ai-llm-request-type")
                .map(String::as_str),
            Some("shared")
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should warn and drop serviceTier on Vertex provider
    #[tokio::test]
    async fn should_warn_and_drop_service_tier_on_vertex_provider() {
        let mock = generate_mock_with_body(&generate_prepared_json_body("test response", None));
        let vertex_model = generate_direct_model(
            "gemini-pro",
            "google.vertex.chat",
            &[("x-goog-api-key", "test-api-key")],
            &mock,
        );

        let result = generate_with(
            &vertex_model,
            json!({ "prompt": generate_test_prompt(), "providerOptions": { "google": { "serviceTier": "priority" } } }),
        )
        .await;

        let call = generate_only_call(&mock);
        assert!(call.json_body().get("serviceTier").is_none());
        assert!(
            !generate_request_headers(&call).contains_key("x-vertex-ai-llm-shared-request-type")
        );
        let warnings = generate_json(&result.warnings);
        assert!(
            warnings.as_array().unwrap().iter().any(|warning| {
                warning["type"] == "other"
                    && warning["message"].as_str().is_some_and(|message| {
                        message.contains("'serviceTier' is a Gemini API option")
                    })
            }),
            "{warnings}"
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should warn when sharedRequestType is set on a non-Vertex provider
    #[tokio::test]
    async fn should_warn_when_shared_request_type_is_set_on_a_non_vertex_provider() {
        let mock = generate_mock_with_body(&generate_prepared_json_body("test response", None));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with(
            &model,
            json!({ "prompt": generate_test_prompt(), "providerOptions": { "google": { "sharedRequestType": "priority" } } }),
        )
        .await;

        assert!(
            !generate_request_headers(&generate_only_call(&mock))
                .contains_key("x-vertex-ai-llm-shared-request-type")
        );
        let warnings = generate_json(&result.warnings);
        assert!(
            warnings.as_array().unwrap().iter().any(|warning| {
                warning["type"] == "other"
                    && warning["message"].as_str().is_some_and(|message| {
                        message.contains("'sharedRequestType' and 'requestType'")
                    })
            }),
            "{warnings}"
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should read serviceTier from usageMetadata.serviceTier
    #[tokio::test]
    async fn should_read_service_tier_from_usage_metadata_service_tier() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": { "parts": [{ "text": "Blue." }], "role": "model" },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": generate_safety_ratings(),
            }],
            "promptFeedback": { "safetyRatings": generate_safety_ratings() },
            "usageMetadata": {
                "promptTokenCount": 1,
                "candidatesTokenCount": 2,
                "totalTokenCount": 3,
                "serviceTier": "priority",
            },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.provider_metadata)["google"]["serviceTier"],
            json!("priority")
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should expose null serviceTier in provider metadata when not present
    #[tokio::test]
    async fn should_expose_null_service_tier_in_provider_metadata_when_not_present() {
        let mock = generate_mock_with_body(&generate_prepared_json_body("test response", None));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        let metadata = generate_json(&result.provider_metadata);
        assert_eq!(metadata["google"].get("serviceTier"), Some(&Value::Null));
    }

    mod tool_call {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › tool-call › should extract tool calls
        #[tokio::test]
        async fn should_extract_tool_calls() {
            let mock = generate_mock_with_fixture("google-tool-call");
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with(
                &model,
                json!({
                    "tools": [{
                        "type": "function",
                        "name": "weather",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "location": { "type": "string" } },
                            "required": ["location"],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#",
                        },
                    }],
                    "prompt": generate_test_prompt(),
                }),
            )
            .await;

            assert_eq!(
                generate_json(&result),
                generate_snapshot("do_generate_tool_call_extract_tool_calls")
            );
        }
    }

    mod reasoning_gemini3 {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › reasoning-gemini3 › should extract reasoning with thoughtSignature
        #[tokio::test]
        async fn should_extract_reasoning_with_thought_signature() {
            let mock = generate_mock_with_fixture("google-reasoning-gemini3");
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            assert_eq!(
                generate_json(&result),
                generate_snapshot("do_generate_reasoning_gemini3_extract_reasoning")
            );
        }
    }

    mod tool_call_gemini3 {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › tool-call-gemini3 › should extract tool call with thoughtSignature
        #[tokio::test]
        async fn should_extract_tool_call_with_thought_signature() {
            let mock = generate_mock_with_fixture("google-tool-call-gemini3");
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            assert_eq!(
                generate_json(&result),
                generate_snapshot("do_generate_tool_call_gemini3_extract_tool_call")
            );
        }
    }

    mod no_args_tool_call_unary {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › no-args tool call (unary) › should extract a no-args tool call with thoughtSignature as input "{}"
        #[tokio::test]
        async fn should_extract_a_no_args_tool_call_with_thought_signature_as_input() {
            let mock = generate_mock_with_body(&json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{
                            "functionCall": { "name": "read_theme" },
                            "thoughtSignature": "sig-no-args-unary",
                        }],
                    },
                    "finishReason": "STOP",
                    "index": 0,
                    "safetyRatings": generate_safety_ratings(),
                }],
                "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3 },
            }));
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            let tool_calls: Vec<Value> = generate_json(&result.content)
                .as_array()
                .unwrap()
                .iter()
                .filter(|content| content["type"] == "tool-call")
                .cloned()
                .collect();
            assert_eq!(
                Value::Array(tool_calls),
                json!([{
                    "type": "tool-call",
                    "toolCallId": "test-id",
                    "toolName": "read_theme",
                    "input": "{}",
                    "providerMetadata": { "google": { "thoughtSignature": "sig-no-args-unary" } },
                }])
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › no-args tool call (unary) › should generate an ID when the function call ID is empty
        #[tokio::test]
        async fn should_generate_an_id_when_the_function_call_id_is_empty() {
            let mock = generate_mock_with_body(&json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{ "functionCall": { "id": "", "name": "read_theme", "args": {} } }],
                    },
                    "finishReason": "STOP",
                    "index": 0,
                    "safetyRatings": generate_safety_ratings(),
                }],
                "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3 },
            }));
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            let content = generate_json(&result.content);
            let expected = json!({
                "type": "tool-call",
                "toolCallId": "test-id",
                "toolName": "read_theme",
                "input": "{}",
            });
            assert!(
                content.as_array().unwrap().contains(&expected),
                "{content:#}"
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let mock = MockFetch::new(vec![generate_json_value_response_with_headers(
            &generate_fixture("google-text"),
            &[("test-header", "test-value")],
        )]);
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.response.expect("response").headers),
            json!({
                "content-length": "549",
                "content-type": "application/json",
                "test-header": "test-value",
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass the model, messages, and options
    #[tokio::test]
    async fn should_pass_the_model_messages_and_options() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({
                "prompt": [
                    { "role": "system", "content": "test system instruction" },
                    { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                ],
                "seed": 123,
                "temperature": 0.5,
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": { "seed": 123, "temperature": 0.5 },
                "systemInstruction": { "parts": [{ "text": "test system instruction" }] },
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should omit unsupported %s for %s
    #[tokio::test]
    async fn should_omit_unsupported_penalty_for_model() {
        for (penalty, model_id) in [
            ("frequencyPenalty", "gemini-2.5-pro"),
            ("presencePenalty", "gemini-2.5-pro"),
            ("frequencyPenalty", "gemini-2.5-flash"),
            ("presencePenalty", "gemini-2.5-flash"),
            ("frequencyPenalty", "gemini-2.5-flash-lite"),
            ("presencePenalty", "gemini-2.5-flash-lite"),
        ] {
            let mock = generate_mock_with_fixture("google-text");
            let model = generate_test_provider(&mock).chat(model_id);

            let mut options = json!({ "prompt": generate_test_prompt() });
            options[penalty] = json!(0.5);
            let result = generate_with(&model, options).await;

            let request_body = generate_only_body(&mock);
            assert_eq!(
                request_body["generationConfig"],
                json!({}),
                "{penalty} {model_id}"
            );
            assert_eq!(
                generate_json(&result.warnings),
                json!([{ "type": "unsupported", "feature": penalty }]),
                "{penalty} {model_id}"
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass penalty settings for Gemini 2.0 models
    #[tokio::test]
    async fn should_pass_penalty_settings_for_gemini_2_0_models() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-2.0-pro");

        let result = generate_with(
            &model,
            json!({ "prompt": generate_test_prompt(), "frequencyPenalty": 0.5, "presencePenalty": 0.5 }),
        )
        .await;

        generate_assert_match_object(
            &generate_only_body(&mock),
            &json!({ "generationConfig": { "frequencyPenalty": 0.5, "presencePenalty": 0.5 } }),
        );
        assert_eq!(generate_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass penalty settings for Vertex Gemini 2.5 models
    #[tokio::test]
    async fn should_pass_penalty_settings_for_vertex_gemini_2_5_models() {
        let mock = generate_mock_with_fixture("google-text");
        let vertex_model = generate_direct_model(
            "gemini-2.5-flash",
            "google.vertex.chat",
            &[("x-goog-api-key", "test-api-key")],
            &mock,
        );

        let result = generate_with(
            &vertex_model,
            json!({ "prompt": generate_test_prompt(), "frequencyPenalty": 0.5, "presencePenalty": 0.5 }),
        )
        .await;

        generate_assert_match_object(
            &generate_only_body(&mock),
            &json!({ "generationConfig": { "frequencyPenalty": 0.5, "presencePenalty": 0.5 } }),
        );
        assert_eq!(generate_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should only pass valid provider options
    #[tokio::test]
    async fn should_only_pass_valid_provider_options() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({
                "prompt": [
                    { "role": "system", "content": "test system instruction" },
                    { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                ],
                "seed": 123,
                "temperature": 0.5,
                "providerOptions": { "google": { "foo": "bar", "responseModalities": ["TEXT", "IMAGE"] } },
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": {
                    "responseModalities": ["TEXT", "IMAGE"],
                    "seed": 123,
                    "temperature": 0.5,
                },
                "systemInstruction": { "parts": [{ "text": "test system instruction" }] },
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should expand standalone threshold provider option into safetySettings
    #[tokio::test]
    async fn should_expand_standalone_threshold_provider_option_into_safety_settings() {
        let request_body =
            generate_body_with_google_options(json!({ "threshold": "BLOCK_NONE" })).await;

        let expected_safety_settings = [
            json!({ "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_NONE" }),
            json!({ "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "BLOCK_NONE" }),
            json!({ "category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE" }),
            json!({ "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "threshold": "BLOCK_NONE" }),
        ];
        let safety_settings = request_body["safetySettings"]
            .as_array()
            .unwrap_or_else(|| panic!("no safetySettings in {request_body}"));
        for expected in &expected_safety_settings {
            assert!(safety_settings.contains(expected), "{request_body}");
        }
        assert_eq!(safety_settings.len(), expected_safety_settings.len());
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should let safetySettings take precedence over standalone threshold provider option
    #[tokio::test]
    async fn should_let_safety_settings_take_precedence_over_standalone_threshold_provider_option()
    {
        let request_body = generate_body_with_google_options(json!({
            "threshold": "BLOCK_NONE",
            "safetySettings": [{ "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_ONLY_HIGH" }],
        }))
        .await;

        assert_eq!(
            request_body["safetySettings"],
            json!([{ "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_ONLY_HIGH" }])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass tools and toolChoice
    #[tokio::test]
    async fn should_pass_tools_and_tool_choice() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({
                "tools": [{
                    "type": "function",
                    "name": "test-tool",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                    },
                }],
                "toolChoice": { "type": "tool", "toolName": "test-tool" },
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": {},
                "toolConfig": {
                    "functionCallingConfig": { "allowedFunctionNames": ["test-tool"], "mode": "ANY" },
                },
                "tools": [{
                    "functionDeclarations": [{
                        "description": "",
                        "name": "test-tool",
                        "parametersJsonSchema": {
                            "$schema": "http://json-schema.org/draft-07/schema#",
                            "additionalProperties": false,
                            "properties": { "value": { "type": "string" } },
                            "required": ["value"],
                            "type": "object",
                        },
                    }],
                }],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should preserve local JSON Schema references in $name tool requests
    #[tokio::test]
    async fn should_preserve_local_json_schema_references_in_name_tool_requests() {
        for name in ["Gemini Developer API", "Vertex AI"] {
            let mock = generate_mock_with_fixture("google-text");
            let model = if name == "Vertex AI" {
                generate_direct_model(
                    "gemini-pro",
                    "google.vertex.chat",
                    &[("x-goog-api-key", "test-api-key")],
                    &mock,
                )
            } else {
                generate_test_provider(&mock).chat("gemini-pro")
            };

            generate_with(
                &model,
                json!({
                    "tools": [{
                        "type": "function",
                        "name": "format-date",
                        "description": "Format a date",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "locale": { "$ref": "#/$defs/Locale", "description": "Locale for formatting" },
                            },
                            "required": ["locale"],
                            "additionalProperties": false,
                            "$defs": { "Locale": { "type": "string", "enum": ["de", "en"] } },
                        },
                    }],
                    "prompt": generate_test_prompt(),
                }),
            )
            .await;

            assert_eq!(
                generate_only_body(&mock)["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"],
                json!({
                    "type": "object",
                    "properties": {
                        "locale": { "$ref": "#/$defs/Locale", "description": "Locale for formatting" },
                    },
                    "required": ["locale"],
                    "additionalProperties": false,
                    "$defs": { "Locale": { "type": "string", "enum": ["de", "en"] } },
                }),
                "{name}"
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should send recursive tool schemas as JSON Schema
    #[tokio::test]
    async fn should_send_recursive_tool_schemas_as_json_schema() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");
        let input_schema = json!({
            "type": "object",
            "properties": { "condition": { "$ref": "#/$defs/Condition" } },
            "required": ["condition"],
            "$defs": {
                "Condition": {
                    "type": "object",
                    "properties": {
                        "children": { "type": "array", "items": { "$ref": "#/$defs/Condition" } },
                    },
                },
            },
        });

        generate_with(
            &model,
            json!({
                "tools": [{
                    "type": "function",
                    "name": "search",
                    "description": "Search with a condition tree",
                    "inputSchema": input_schema,
                }],
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(mock.seen().len(), 1);
        assert_eq!(
            generate_only_body(&mock)["tools"][0]["functionDeclarations"][0],
            json!({
                "name": "search",
                "description": "Search with a condition tree",
                "parametersJsonSchema": input_schema,
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should set response mime type with responseFormat
    #[tokio::test]
    async fn should_set_response_mime_type_with_response_format() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({
                "responseFormat": {
                    "type": "json",
                    "schema": { "type": "object", "properties": { "location": { "type": "string" } } },
                },
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": {
                    "responseJsonSchema": {
                        "properties": { "location": { "type": "string" } },
                        "type": "object",
                    },
                    "responseMimeType": "application/json",
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass array length constraints in response schemas
    #[tokio::test]
    async fn should_pass_array_length_constraints_in_response_schemas() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");
        let schema = json!({
            "type": "object",
            "properties": {
                "elements": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 4 },
            },
            "required": ["elements"],
        });

        generate_with(
            &model,
            json!({ "responseFormat": { "type": "json", "schema": schema }, "prompt": generate_test_prompt() }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock)["generationConfig"]["responseJsonSchema"],
            schema
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should preserve local JSON Schema references in response schemas
    #[tokio::test]
    async fn should_preserve_local_json_schema_references_in_response_schemas() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");
        let schema = json!({
            "type": "object",
            "properties": { "locale": { "$ref": "#/$defs/Locale" } },
            "required": ["locale"],
            "$defs": { "Locale": { "type": "string", "enum": ["de", "en"] } },
        });

        generate_with(
            &model,
            json!({ "responseFormat": { "type": "json", "schema": schema }, "prompt": generate_test_prompt() }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock)["generationConfig"]["responseJsonSchema"],
            schema
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass specification with responseFormat and structuredOutputs = true (default)
    #[tokio::test]
    async fn should_pass_specification_with_response_format_and_structured_outputs_true_default() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "property1": { "type": "string" }, "property2": { "type": "number" } },
                        "required": ["property1", "property2"],
                        "additionalProperties": false,
                    },
                },
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": {
                    "responseJsonSchema": {
                        "additionalProperties": false,
                        "properties": { "property1": { "type": "string" }, "property2": { "type": "number" } },
                        "required": ["property1", "property2"],
                        "type": "object",
                    },
                    "responseMimeType": "application/json",
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should not pass specification with responseFormat and structuredOutputs = false
    #[tokio::test]
    async fn should_not_pass_specification_with_response_format_and_structured_outputs_false() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({
                "providerOptions": { "google": { "structuredOutputs": false } },
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "property1": { "type": "string" }, "property2": { "type": "number" } },
                        "required": ["property1", "property2"],
                        "additionalProperties": false,
                    },
                },
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": { "responseMimeType": "application/json" },
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass tools and toolChoice #2
    #[tokio::test]
    async fn should_pass_tools_and_tool_choice_2() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({
                "tools": [{
                    "name": "test-tool",
                    "type": "function",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "property1": { "type": "string" }, "property2": { "type": "number" } },
                        "required": ["property1", "property2"],
                        "additionalProperties": false,
                    },
                }],
                "toolChoice": { "type": "required" },
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": {},
                "toolConfig": { "functionCallingConfig": { "mode": "ANY" } },
                "tools": [{
                    "functionDeclarations": [{
                        "description": "",
                        "name": "test-tool",
                        "parametersJsonSchema": {
                            "additionalProperties": false,
                            "properties": { "property1": { "type": "string" }, "property2": { "type": "number" } },
                            "required": ["property1", "property2"],
                            "type": "object",
                        },
                    }],
                }],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = generate_mock_with_fixture("google-text");
        let provider = create_google(GoogleProviderSettings {
            api_key: Some("test-api-key".into()),
            headers: Some(HeaderMapOpt::from([(
                "Custom-Provider-Header".to_string(),
                Some("provider-header-value".to_string()),
            )])),
            fetch: Some(mock.transport()),
            ..Default::default()
        })
        .expect("valid settings");

        generate_with(
            &provider.chat("gemini-pro"),
            json!({
                "prompt": generate_test_prompt(),
                "headers": { "Custom-Request-Header": "request-header-value" },
            }),
        )
        .await;

        let call = generate_only_call(&mock);
        assert_eq!(
            generate_json(&generate_request_headers(&call)),
            json!({
                "content-type": "application/json",
                "custom-provider-header": "provider-header-value",
                "custom-request-header": "request-header-value",
                "x-goog-api-key": "test-api-key",
            })
        );
        // Upstream mocks the package version as `0.0.0-test`; aimux pins
        // `4.0.87`.
        let user_agent = call.headers.get("user-agent").cloned().unwrap_or_default();
        assert!(user_agent.contains("ai-sdk-google/4.0.87"), "{user_agent}");
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass response format
    #[tokio::test]
    async fn should_pass_response_format() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with(
            &model,
            json!({
                "prompt": generate_test_prompt(),
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": { "text": { "type": "string" } },
                        "required": ["text"],
                    },
                },
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": {
                    "responseJsonSchema": {
                        "properties": { "text": { "type": "string" } },
                        "required": ["text"],
                        "type": "object",
                    },
                    "responseMimeType": "application/json",
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_only_body(&mock),
            json!({
                "contents": [{ "parts": [{ "text": "Hello" }], "role": "user" }],
                "generationConfig": {},
            })
        );
    }

    /// The content of `model.doGenerate({ prompt: TEST_PROMPT })` against
    /// `prepareJsonResponse({ content, groundingMetadata })`.
    async fn generate_content_with_grounding(content: &str, grounding_metadata: Value) -> Value {
        let mock = generate_mock_with_body(&generate_prepared_json_body(
            content,
            Some(grounding_metadata),
        ));
        let model = generate_test_provider(&mock).chat("gemini-pro");
        generate_json(&generate_with_test_prompt(&model).await.content)
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should extract sources from grounding metadata
    #[tokio::test]
    async fn should_extract_sources_from_grounding_metadata() {
        let content = generate_content_with_grounding(
            "test response",
            json!({
                "groundingChunks": [{ "web": { "uri": "https://source.example.com", "title": "Source Title" } }],
            }),
        )
        .await;

        assert_eq!(
            content,
            json!([
                { "text": "test response", "type": "text" },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Source Title",
                    "type": "source",
                    "url": "https://source.example.com",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should extract sources from RAG retrievedContext chunks
    #[tokio::test]
    async fn should_extract_sources_from_rag_retrieved_context_chunks() {
        let content = generate_content_with_grounding(
            "test response with RAG",
            json!({
                "groundingChunks": [
                    { "web": { "uri": "https://web.example.com", "title": "Web Source" } },
                    {
                        "retrievedContext": {
                            "uri": "gs://rag-corpus/document.pdf",
                            "title": "RAG Document",
                            "text": "Retrieved context...",
                        },
                    },
                    {
                        "retrievedContext": {
                            "uri": "https://external-rag-source.com/page",
                            "title": "External RAG Source",
                            "text": "External retrieved context...",
                        },
                    },
                ],
            }),
        )
        .await;

        assert_eq!(
            content,
            json!([
                { "text": "test response with RAG", "type": "text" },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Web Source",
                    "type": "source",
                    "url": "https://web.example.com",
                },
                {
                    "filename": "document.pdf",
                    "id": "test-id",
                    "mediaType": "application/pdf",
                    "sourceType": "document",
                    "title": "RAG Document",
                    "type": "source",
                },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "External RAG Source",
                    "type": "source",
                    "url": "https://external-rag-source.com/page",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should extract sources from File Search retrievedContext (new format)
    #[tokio::test]
    async fn should_extract_sources_from_file_search_retrieved_context_new_format() {
        let content = generate_content_with_grounding(
            "test response with File Search",
            json!({
                "groundingChunks": [
                    {
                        "retrievedContext": {
                            "text": "Sample content for testing...",
                            "fileSearchStore": "fileSearchStores/test-store-xyz",
                            "title": "Test Document",
                        },
                    },
                    {
                        "retrievedContext": {
                            "text": "Another document content...",
                            "fileSearchStore": "fileSearchStores/another-store-abc",
                        },
                    },
                ],
            }),
        )
        .await;

        assert_eq!(
            content,
            json!([
                { "text": "test response with File Search", "type": "text" },
                {
                    "filename": "test-store-xyz",
                    "id": "test-id",
                    "mediaType": "application/octet-stream",
                    "sourceType": "document",
                    "title": "Test Document",
                    "type": "source",
                },
                {
                    "filename": "another-store-abc",
                    "id": "test-id",
                    "mediaType": "application/octet-stream",
                    "sourceType": "document",
                    "title": "Unknown Document",
                    "type": "source",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should handle URL sources with undefined title correctly
    #[tokio::test]
    async fn should_handle_url_sources_with_undefined_title_correctly() {
        let content = generate_content_with_grounding(
            "test response with URLs",
            json!({
                "groundingChunks": [
                    { "web": { "uri": "https://example.com/page1" } },
                    { "retrievedContext": { "uri": "https://example.com/page2" } },
                ],
            }),
        )
        .await;

        assert_eq!(
            content,
            json!([
                { "text": "test response with URLs", "type": "text" },
                { "id": "test-id", "sourceType": "url", "type": "source", "url": "https://example.com/page1" },
                { "id": "test-id", "sourceType": "url", "type": "source", "url": "https://example.com/page2" },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should extract sources from maps grounding metadata
    #[tokio::test]
    async fn should_extract_sources_from_maps_grounding_metadata() {
        let content = generate_content_with_grounding(
            "test response with Maps",
            json!({
                "groundingChunks": [
                    {
                        "maps": {
                            "uri": "https://maps.google.com/maps?cid=12345",
                            "title": "Best Italian Restaurant",
                            "placeId": "ChIJ12345",
                        },
                    },
                    { "maps": { "uri": "https://maps.google.com/maps?cid=67890" } },
                ],
            }),
        )
        .await;

        assert_eq!(
            content,
            json!([
                { "text": "test response with Maps", "type": "text" },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Best Italian Restaurant",
                    "type": "source",
                    "url": "https://maps.google.com/maps?cid=12345",
                },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "type": "source",
                    "url": "https://maps.google.com/maps?cid=67890",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should extract sources from image grounding metadata
    #[tokio::test]
    async fn should_extract_sources_from_image_grounding_metadata() {
        let content = generate_content_with_grounding(
            "test response with image search",
            json!({
                "groundingChunks": [
                    {
                        "image": {
                            "sourceUri": "https://example.com/article",
                            "imageUri": "https://example.com/image.jpg",
                            "title": "Image Result",
                            "domain": "example.com",
                        },
                    },
                    {
                        "image": {
                            "sourceUri": "https://other.example.com/page",
                            "imageUri": "https://other.example.com/photo.png",
                        },
                    },
                ],
            }),
        )
        .await;

        assert_eq!(
            content,
            json!([
                { "text": "test response with image search", "type": "text" },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Image Result",
                    "type": "source",
                    "url": "https://example.com/article",
                },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "type": "source",
                    "url": "https://other.example.com/page",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should handle mixed source types with correct title defaults
    #[tokio::test]
    async fn should_handle_mixed_source_types_with_correct_title_defaults() {
        let content = generate_content_with_grounding(
            "test response with mixed sources",
            json!({
                "groundingChunks": [
                    { "web": { "uri": "https://web.example.com" } },
                    { "retrievedContext": { "uri": "https://external.example.com" } },
                    { "retrievedContext": { "uri": "gs://bucket/document.pdf" } },
                    { "retrievedContext": { "fileSearchStore": "fileSearchStores/store-123" } },
                ],
            }),
        )
        .await;

        assert_eq!(
            content,
            json!([
                { "text": "test response with mixed sources", "type": "text" },
                { "id": "test-id", "sourceType": "url", "type": "source", "url": "https://web.example.com" },
                { "id": "test-id", "sourceType": "url", "type": "source", "url": "https://external.example.com" },
                {
                    "filename": "document.pdf",
                    "id": "test-id",
                    "mediaType": "application/pdf",
                    "sourceType": "document",
                    "title": "Unknown Document",
                    "type": "source",
                },
                {
                    "filename": "store-123",
                    "id": "test-id",
                    "mediaType": "application/octet-stream",
                    "sourceType": "document",
                    "title": "Unknown Document",
                    "type": "source",
                },
            ])
        );
    }

    mod async_headers_handling {
        use super::*;

        /// The body the async headers tests serve.
        fn generate_empty_text_body() -> Value {
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0,
                    "safetyRatings": generate_safety_ratings(),
                }],
                "promptFeedback": { "safetyRatings": generate_safety_ratings() },
                "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3 },
            })
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › async headers handling › merges async config headers with sync request headers
        #[tokio::test]
        async fn merges_async_config_headers_with_sync_request_headers() {
            // Upstream also passes `supportedUrls`, which the request does not
            // read.
            let mock = generate_mock_with_body(&generate_empty_text_body());
            let model = generate_direct_model_with_async_headers(
                "gemini-pro",
                "google.generative-ai",
                &[
                    ("X-Async-Config", "async-config-value"),
                    ("X-Common", "config-value"),
                ],
                &mock,
            );

            generate_with(
                &model,
                json!({
                    "prompt": generate_test_prompt(),
                    "headers": { "X-Sync-Request": "sync-request-value", "X-Common": "request-value" },
                }),
            )
            .await;

            assert_eq!(
                generate_json(&generate_request_headers(&generate_only_call(&mock))),
                json!({
                    "content-type": "application/json",
                    "x-async-config": "async-config-value",
                    "x-common": "request-value",
                    "x-sync-request": "sync-request-value",
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › async headers handling › handles Promise-based headers
        #[tokio::test]
        async fn handles_promise_based_headers() {
            let mock = generate_mock_with_body(&generate_empty_text_body());
            let model = generate_direct_model_with_async_headers(
                "gemini-pro",
                "google.generative-ai",
                &[("X-Promise-Header", "promise-value")],
                &mock,
            );

            generate_with_test_prompt(&model).await;

            assert_eq!(
                generate_json(&generate_request_headers(&generate_only_call(&mock))),
                json!({ "content-type": "application/json", "x-promise-header": "promise-value" })
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › async headers handling › handles async function headers from config
        #[tokio::test]
        async fn handles_async_function_headers_from_config() {
            let mock = generate_mock_with_fixture("google-text");
            let model = generate_direct_model_with_async_headers(
                "gemini-pro",
                "google.generative-ai",
                &[("X-Async-Header", "async-value")],
                &mock,
            );

            generate_with_test_prompt(&model).await;

            assert_eq!(
                generate_json(&generate_request_headers(&generate_only_call(&mock))),
                json!({ "content-type": "application/json", "x-async-header": "async-value" })
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should expose safety ratings in provider metadata
    #[tokio::test]
    async fn should_expose_safety_ratings_in_provider_metadata() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": { "parts": [{ "text": "test response" }], "role": "model" },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": [{
                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                    "probability": "NEGLIGIBLE",
                    "probabilityScore": 0.1,
                    "severity": "LOW",
                    "severityScore": 0.2,
                    "blocked": false,
                }],
            }],
            "promptFeedback": { "safetyRatings": generate_safety_ratings() },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.provider_metadata)["google"]["safetyRatings"],
            json!([{
                "blocked": false,
                "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                "probability": "NEGLIGIBLE",
                "probabilityScore": 0.1,
                "severity": "LOW",
                "severityScore": 0.2,
            }])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should expose PromptFeedback in provider metadata
    #[tokio::test]
    async fn should_expose_prompt_feedback_in_provider_metadata() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": { "parts": [{ "text": "No" }], "role": "model" },
                "finishReason": "SAFETY",
                "index": 0,
                "safetyRatings": generate_safety_ratings(),
            }],
            "promptFeedback": { "blockReason": "SAFETY", "safetyRatings": generate_safety_ratings() },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.provider_metadata)["google"]["promptFeedback"],
            json!({
                "blockReason": "SAFETY",
                "safetyRatings": [
                    { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "probability": "NEGLIGIBLE" },
                    { "category": "HARM_CATEGORY_HATE_SPEECH", "probability": "NEGLIGIBLE" },
                    { "category": "HARM_CATEGORY_HARASSMENT", "probability": "NEGLIGIBLE" },
                    { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "probability": "NEGLIGIBLE" },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should surface prompt-level blocks without candidates
    #[tokio::test]
    async fn should_surface_prompt_level_blocks_without_candidates() {
        let mock = generate_mock_with_body(&json!({
            "promptFeedback": { "blockReason": "PROHIBITED_CONTENT" },
            "usageMetadata": { "promptTokenCount": 9, "totalTokenCount": 9, "serviceTier": "standard" },
            "responseId": "blocked-response-id",
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(generate_json(&result.content), json!([]));
        assert_eq!(
            generate_json(&result.finish_reason),
            json!({ "unified": "content-filter", "raw": "PROHIBITED_CONTENT" })
        );
        generate_assert_match_object(
            &generate_json(&result.provider_metadata)["google"],
            &json!({
                "promptFeedback": { "blockReason": "PROHIBITED_CONTENT" },
                "groundingMetadata": null,
                "urlContextMetadata": null,
                "safetyRatings": null,
                "usageMetadata": { "promptTokenCount": 9, "totalTokenCount": 9, "serviceTier": "standard" },
                "finishMessage": null,
                "serviceTier": "standard",
            }),
        );
        assert_eq!(
            result.response.expect("response").id.as_deref(),
            Some("blocked-response-id")
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should not classify the default prompt block reason %j as a content filter
    #[tokio::test]
    async fn should_not_classify_the_default_prompt_block_reason_as_a_content_filter() {
        for block_reason in ["", "BLOCK_REASON_UNSPECIFIED", "BLOCKED_REASON_UNSPECIFIED"] {
            let mock = generate_mock_with_body(&json!({
                "candidates": [],
                "promptFeedback": { "blockReason": block_reason },
                "usageMetadata": { "promptTokenCount": 9, "totalTokenCount": 9 },
            }));
            let model = generate_test_provider(&mock).chat("gemini-pro");

            let result = generate_with_test_prompt(&model).await;

            assert_eq!(
                generate_json(&result.content),
                json!([]),
                "{block_reason:?}"
            );
            assert_eq!(
                generate_json(&result.finish_reason),
                json!({ "unified": "other" }),
                "{block_reason:?}"
            );
            assert_eq!(
                generate_json(&result.provider_metadata)["google"]["promptFeedback"],
                json!({ "blockReason": block_reason }),
                "{block_reason:?}"
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should expose grounding metadata in provider metadata
    #[tokio::test]
    async fn should_expose_grounding_metadata_in_provider_metadata() {
        let grounding_metadata = json!({
            "webSearchQueries": ["What's the weather in Chicago this weekend?"],
            "searchEntryPoint": { "renderedContent": "Sample rendered content for search results" },
            "groundingChunks": [{
                "web": { "uri": "https://example.com/weather", "title": "Chicago Weather Forecast" },
            }],
            "groundingSupports": [{
                "segment": {
                    "startIndex": 0,
                    "endIndex": 65,
                    "text": "Chicago weather changes rapidly, so layers let you adjust easily.",
                },
                "groundingChunkIndices": [0],
                "confidenceScores": [0.99],
            }],
            "retrievalMetadata": { "webDynamicRetrievalScore": 0.96879 },
        });
        let mock = generate_mock_with_body(&generate_prepared_json_body(
            "test response",
            Some(grounding_metadata),
        ));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.provider_metadata)["google"]["groundingMetadata"],
            json!({
                "groundingChunks": [{
                    "web": { "title": "Chicago Weather Forecast", "uri": "https://example.com/weather" },
                }],
                "groundingSupports": [{
                    "confidenceScores": [0.99],
                    "groundingChunkIndices": [0],
                    "segment": {
                        "endIndex": 65,
                        "startIndex": 0,
                        "text": "Chicago weather changes rapidly, so layers let you adjust easily.",
                    },
                }],
                "retrievalMetadata": { "webDynamicRetrievalScore": 0.96879 },
                "searchEntryPoint": { "renderedContent": "Sample rendered content for search results" },
                "webSearchQueries": ["What's the weather in Chicago this weekend?"],
            })
        );
    }

    /// A one-candidate `STOP` body with `parts`.
    fn generate_parts_body(parts: Value) -> Value {
        json!({ "candidates": [{ "content": { "parts": parts, "role": "model" }, "finishReason": "STOP" }] })
    }

    /// The `print(1+1)` code execution body of the code execution tests.
    fn generate_code_execution_body() -> Value {
        generate_parts_body(json!([
            { "executableCode": { "language": "PYTHON", "code": "print(1+1)" } },
            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "2" } },
        ]))
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should handle code execution tool calls
    #[tokio::test]
    async fn should_handle_code_execution_tool_calls() {
        let mock = generate_mock_with_body(&generate_code_execution_body());
        let model = generate_test_provider(&mock).chat("gemini-2.0-pro");

        let result = generate_with(
            &model,
            json!({
                "tools": [generate_provider_tool("google.code_execution", "code_execution")],
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock)["tools"],
            json!([{ "codeExecution": {} }])
        );
        assert_eq!(
            generate_json(&result.content),
            json!([
                {
                    "input": "{\"language\":\"PYTHON\",\"code\":\"print(1+1)\"}",
                    "providerExecuted": true,
                    "toolCallId": "test-id",
                    "toolName": "code_execution",
                    "type": "tool-call",
                },
                {
                    "result": { "outcome": "OUTCOME_OK", "output": "2" },
                    "toolCallId": "test-id",
                    "toolName": "code_execution",
                    "type": "tool-result",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should handle code execution result with missing output field
    #[tokio::test]
    async fn should_handle_code_execution_result_with_missing_output_field() {
        let mock = generate_mock_with_body(&generate_parts_body(json!([
            {
                "executableCode": {
                    "language": "PYTHON",
                    "code": "import PIL.Image\nimg = PIL.Image.open('input.png')\nimg.save('output.png')\n",
                },
            },
            { "codeExecutionResult": { "outcome": "OUTCOME_OK" } },
        ])));
        let model = generate_test_provider(&mock).chat("gemini-2.0-pro");

        let result = generate_with(
            &model,
            json!({
                "tools": [generate_provider_tool("google.code_execution", "code_execution")],
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                {
                    "input": "{\"language\":\"PYTHON\",\"code\":\"import PIL.Image\\nimg = PIL.Image.open('input.png')\\nimg.save('output.png')\\n\"}",
                    "providerExecuted": true,
                    "toolCallId": "test-id",
                    "toolName": "code_execution",
                    "type": "tool-call",
                },
                {
                    "result": { "outcome": "OUTCOME_OK", "output": "" },
                    "toolCallId": "test-id",
                    "toolName": "code_execution",
                    "type": "tool-result",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should return stop finish reason for code execution (provider-executed tool)
    #[tokio::test]
    async fn should_return_stop_finish_reason_for_code_execution_provider_executed_tool() {
        let mock = generate_mock_with_body(&generate_code_execution_body());
        let model = generate_test_provider(&mock).chat("gemini-2.0-pro");

        let result = generate_with(
            &model,
            json!({
                "tools": [generate_provider_tool("google.code_execution", "code_execution")],
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_json(&result.finish_reason),
            json!({ "raw": "STOP", "unified": "stop" })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should return stop finish reason for code execution with text response (structured output scenario)
    #[tokio::test]
    async fn should_return_stop_finish_reason_for_code_execution_with_text_response() {
        let mock = generate_mock_with_body(&generate_parts_body(json!([
            {
                "executableCode": {
                    "language": "PYTHON",
                    "code": "primes = [2, 3, 5, 7, 11]\nprint(sum(primes))",
                },
            },
            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "28" } },
            { "text": "{\"answer\": 28, \"explanation\": \"Sum of first 5 primes\"}" },
        ])));
        let model = generate_test_provider(&mock).chat("gemini-2.0-pro");

        let result = generate_with(
            &model,
            json!({
                "tools": [generate_provider_tool("google.code_execution", "code_execution")],
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_json(&result.finish_reason),
            json!({ "raw": "STOP", "unified": "stop" })
        );
        let content = generate_json(&result.content);
        let text_part = content
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["type"] == "text")
            .expect("text part");
        assert_eq!(
            text_part["text"],
            json!("{\"answer\": 28, \"explanation\": \"Sum of first 5 primes\"}")
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should return tool-calls finish reason when code execution is combined with function tools
    #[tokio::test]
    async fn should_return_tool_calls_finish_reason_when_code_execution_is_combined_with_function_tools()
     {
        let mock = generate_mock_with_body(&generate_parts_body(json!([
            { "executableCode": { "language": "PYTHON", "code": "print(1+1)" } },
            { "codeExecutionResult": { "outcome": "OUTCOME_OK", "output": "2" } },
            { "functionCall": { "name": "test-tool", "args": { "value": "test" } } },
        ])));
        let model = generate_test_provider(&mock).chat("gemini-2.0-pro");

        let result = generate_with(
            &model,
            json!({
                "tools": [
                    generate_provider_tool("google.code_execution", "code_execution"),
                    {
                        "type": "function",
                        "name": "test-tool",
                        "inputSchema": { "type": "object", "properties": { "value": { "type": "string" } } },
                    },
                ],
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_json(&result.finish_reason),
            json!({ "raw": "STOP", "unified": "tool-calls" })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should handle server-side toolCall and toolResponse parts (tool combination)
    #[tokio::test]
    async fn should_handle_server_side_tool_call_and_tool_response_parts_tool_combination() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {
                            "toolCall": {
                                "toolType": "GOOGLE_SEARCH_WEB",
                                "args": { "query": "San Francisco weather" },
                                "id": "server-call-1",
                            },
                            "thoughtSignature": "sig-abc",
                        },
                        {
                            "toolResponse": {
                                "toolType": "GOOGLE_SEARCH_WEB",
                                "response": { "results": [{ "title": "Weather in SF" }] },
                                "id": "server-call-1",
                            },
                            "thoughtSignature": "sig-def",
                        },
                        { "text": "The weather in San Francisco is sunny." },
                    ],
                    "role": "model",
                },
                "finishReason": "STOP",
            }],
            "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 20, "totalTokenCount": 30 },
        }));
        let model = generate_test_provider(&mock).chat("gemini-3-pro-preview");

        let result = generate_with(
            &model,
            json!({
                "tools": [
                    generate_provider_tool("google.google_search", "google_search"),
                    {
                        "type": "function",
                        "name": "weather",
                        "inputSchema": { "type": "object", "properties": { "location": { "type": "string" } } },
                    },
                ],
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                {
                    "dynamic": true,
                    "input": "{\"query\":\"San Francisco weather\"}",
                    "providerExecuted": true,
                    "providerMetadata": {
                        "google": {
                            "serverToolCallId": "server-call-1",
                            "serverToolType": "GOOGLE_SEARCH_WEB",
                            "thoughtSignature": "sig-abc",
                        },
                    },
                    "toolCallId": "server-call-1",
                    "toolName": "server:GOOGLE_SEARCH_WEB",
                    "type": "tool-call",
                },
                {
                    "providerMetadata": {
                        "google": {
                            "serverToolCallId": "server-call-1",
                            "serverToolType": "GOOGLE_SEARCH_WEB",
                            "thoughtSignature": "sig-def",
                        },
                    },
                    "result": { "results": [{ "title": "Weather in SF" }] },
                    "toolCallId": "server-call-1",
                    "toolName": "server:GOOGLE_SEARCH_WEB",
                    "type": "tool-result",
                },
                { "text": "The weather in San Francisco is sunny.", "type": "text" },
            ])
        );
        assert_eq!(
            generate_json(&result.finish_reason),
            json!({ "raw": "STOP", "unified": "stop" })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should return stop finish reason for server tool calls (provider-executed)
    #[tokio::test]
    async fn should_return_stop_finish_reason_for_server_tool_calls_provider_executed() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "toolCall": { "toolType": "GOOGLE_SEARCH_WEB", "args": {}, "id": "sc-1" } },
                        { "toolResponse": { "toolType": "GOOGLE_SEARCH_WEB", "response": { "results": [] }, "id": "sc-1" } },
                    ],
                    "role": "model",
                },
                "finishReason": "STOP",
            }],
            "usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 10, "totalTokenCount": 15 },
        }));
        let model = generate_test_provider(&mock).chat("gemini-3-pro-preview");

        let result = generate_with(
            &model,
            json!({
                "tools": [generate_provider_tool("google.google_search", "google_search")],
                "prompt": generate_test_prompt(),
            }),
        )
        .await;

        assert_eq!(
            generate_json(&result.finish_reason),
            json!({ "raw": "STOP", "unified": "stop" })
        );
    }

    mod search_tool_selection {
        use super::*;

        /// `modelId.doGenerate({ prompt: TEST_PROMPT, tools: [tool] })` against
        /// `google-text`: the request body and the result.
        async fn generate_with_one_tool(model_id: &str, tool: Value) -> (Value, GenerateResult) {
            let mock = generate_mock_with_fixture("google-text");
            let model = generate_test_provider(&mock).chat(model_id);
            let result = generate_with(
                &model,
                json!({ "prompt": generate_test_prompt(), "tools": [tool] }),
            )
            .await;
            (generate_only_body(&mock), result)
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › search tool selection › should use googleSearch for gemini-2.0-pro
        #[tokio::test]
        async fn should_use_google_search_for_gemini_2_0_pro() {
            let (request_body, _) = generate_with_one_tool(
                "gemini-2.0-pro",
                generate_provider_tool("google.google_search", "google_search"),
            )
            .await;

            generate_assert_match_object(
                &request_body,
                &json!({ "tools": [{ "googleSearch": {} }] }),
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › search tool selection › should use googleSearch for gemini-2.0-flash-exp
        #[tokio::test]
        async fn should_use_google_search_for_gemini_2_0_flash_exp() {
            let (request_body, _) = generate_with_one_tool(
                "gemini-2.0-flash-exp",
                generate_provider_tool("google.google_search", "google_search"),
            )
            .await;

            generate_assert_match_object(
                &request_body,
                &json!({ "tools": [{ "googleSearch": {} }] }),
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › search tool selection › should warn for google search on non-gemini-2 models
        #[tokio::test]
        async fn should_warn_for_google_search_on_non_gemini_2_models() {
            let (_, result) = generate_with_one_tool(
                "gemini-1.0-pro",
                generate_provider_tool("google.google_search", "google_search"),
            )
            .await;

            assert_eq!(
                generate_json(&result.warnings),
                json!([{
                    "details": "Google Search requires Gemini 2.0 or newer.",
                    "feature": "provider-defined tool google.google_search",
                    "type": "unsupported",
                }])
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › search tool selection › should use urlContextTool for gemini-2.0-pro
        #[tokio::test]
        async fn should_use_url_context_tool_for_gemini_2_0_pro() {
            let (request_body, _) = generate_with_one_tool(
                "gemini-2.0-pro",
                generate_provider_tool("google.url_context", "url_context"),
            )
            .await;

            generate_assert_match_object(
                &request_body,
                &json!({ "tools": [{ "urlContext": {} }] }),
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › search tool selection › should use vertexRagStore for gemini-2.0-pro
        #[tokio::test]
        async fn should_use_vertex_rag_store_for_gemini_2_0_pro() {
            let (request_body, _) = generate_with_one_tool(
                "gemini-2.0-pro",
                json!({
                    "type": "provider",
                    "id": "google.vertex_rag_store",
                    "name": "vertex_rag_store",
                    "args": {
                        "ragCorpus": "projects/my-project/locations/us-central1/ragCorpora/my-rag-corpus",
                        "topK": 5,
                    },
                }),
            )
            .await;

            generate_assert_match_object(
                &request_body,
                &json!({
                    "tools": [{
                        "retrieval": {
                            "vertex_rag_store": {
                                "rag_resources": {
                                    "rag_corpus": "projects/my-project/locations/us-central1/ragCorpora/my-rag-corpus",
                                },
                                "similarity_top_k": 5,
                            },
                        },
                    }],
                }),
            );
        }
    }

    /// The body of the image output tests: a one-candidate `STOP` response
    /// with `parts`, safety ratings and usage.
    fn generate_image_parts_body(parts: Value) -> Value {
        json!({
            "candidates": [{
                "content": { "parts": parts, "role": "model" },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": generate_safety_ratings(),
            }],
            "promptFeedback": { "safetyRatings": generate_safety_ratings() },
            "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 20, "totalTokenCount": 30 },
        })
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should extract image file outputs
    #[tokio::test]
    async fn should_extract_image_file_outputs() {
        let mock = generate_mock_with_body(&generate_image_parts_body(json!([
            { "text": "Here is an image:" },
            { "inlineData": { "mimeType": "image/jpeg", "data": "base64encodedimagedata" } },
            { "text": "And another image:" },
            { "inlineData": { "mimeType": "image/png", "data": "anotherbase64encodedimagedata" } },
        ])));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                { "text": "Here is an image:", "type": "text" },
                {
                    "data": { "data": "base64encodedimagedata", "type": "data" },
                    "mediaType": "image/jpeg",
                    "type": "file",
                },
                { "text": "And another image:", "type": "text" },
                {
                    "data": { "data": "anotherbase64encodedimagedata", "type": "data" },
                    "mediaType": "image/png",
                    "type": "file",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should handle responses with only images and no text
    #[tokio::test]
    async fn should_handle_responses_with_only_images_and_no_text() {
        let mock = generate_mock_with_body(&generate_image_parts_body(json!([
            { "inlineData": { "mimeType": "image/jpeg", "data": "imagedata1" } },
            { "inlineData": { "mimeType": "image/png", "data": "imagedata2" } },
        ])));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                { "data": { "data": "imagedata1", "type": "data" }, "mediaType": "image/jpeg", "type": "file" },
                { "data": { "data": "imagedata2", "type": "data" }, "mediaType": "image/png", "type": "file" },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should preserve thought flag on file parts in providerMetadata
    #[tokio::test]
    async fn should_preserve_thought_flag_on_file_parts_in_provider_metadata() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {
                            "inlineData": { "mimeType": "image/png", "data": "thoughtimagedata" },
                            "thought": true,
                            "thoughtSignature": "img_sig1",
                        },
                        { "inlineData": { "mimeType": "image/jpeg", "data": "regularimagedata" } },
                    ],
                    "role": "model",
                },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": generate_safety_ratings(),
            }],
            "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 20, "totalTokenCount": 30 },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                {
                    "data": { "data": "thoughtimagedata", "type": "data" },
                    "mediaType": "image/png",
                    "providerMetadata": { "google": { "thoughtSignature": "img_sig1" } },
                    "type": "reasoning-file",
                },
                {
                    "data": { "data": "regularimagedata", "type": "data" },
                    "mediaType": "image/jpeg",
                    "type": "file",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass responseModalities in provider options
    #[tokio::test]
    async fn should_pass_response_modalities_in_provider_options() {
        let request_body =
            generate_body_with_google_options(json!({ "responseModalities": ["TEXT", "IMAGE"] }))
                .await;

        generate_assert_match_object(
            &request_body,
            &json!({ "generationConfig": { "responseModalities": ["TEXT", "IMAGE"] } }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass mediaResolution in provider options
    #[tokio::test]
    async fn should_pass_media_resolution_in_provider_options() {
        let request_body =
            generate_body_with_google_options(json!({ "mediaResolution": "MEDIA_RESOLUTION_LOW" }))
                .await;

        generate_assert_match_object(
            &request_body,
            &json!({ "generationConfig": { "mediaResolution": "MEDIA_RESOLUTION_LOW" } }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass imageConfig.aspectRatio in provider options
    #[tokio::test]
    async fn should_pass_image_config_aspect_ratio_in_provider_options() {
        let request_body =
            generate_body_with_google_options(json!({ "imageConfig": { "aspectRatio": "16:9" } }))
                .await;

        generate_assert_match_object(
            &request_body,
            &json!({ "generationConfig": { "imageConfig": { "aspectRatio": "16:9" } } }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass imageConfig.imageSize in provider options
    #[tokio::test]
    async fn should_pass_image_config_image_size_in_provider_options() {
        let request_body =
            generate_body_with_google_options(json!({ "imageConfig": { "imageSize": "4K" } }))
                .await;

        generate_assert_match_object(
            &request_body,
            &json!({ "generationConfig": { "imageConfig": { "imageSize": "4K" } } }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass imageConfig with both aspectRatio and imageSize
    #[tokio::test]
    async fn should_pass_image_config_with_both_aspect_ratio_and_image_size() {
        let request_body = generate_body_with_google_options(
            json!({ "imageConfig": { "aspectRatio": "16:9", "imageSize": "2K" } }),
        )
        .await;

        generate_assert_match_object(
            &request_body,
            &json!({ "generationConfig": { "imageConfig": { "aspectRatio": "16:9", "imageSize": "2K" } } }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass imageConfig.personGeneration, imageConfig.prominentPeople and imageConfig.imageOutputOptions on Vertex
    #[tokio::test]
    async fn should_pass_image_config_person_generation_prominent_people_and_image_output_options_on_vertex()
     {
        let mock = generate_mock_with_fixture("google-text");
        let vertex_model = generate_direct_model(
            "gemini-pro",
            "google.vertex.chat",
            &[("x-goog-api-key", "test-api-key")],
            &mock,
        );
        let image_config = json!({
            "aspectRatio": "16:9",
            "personGeneration": "ALLOW_ADULT",
            "prominentPeople": "BLOCK_PROMINENT_PEOPLE",
            "imageOutputOptions": { "mimeType": "image/jpeg", "compressionQuality": 75 },
        });

        let result = generate_with(
            &vertex_model,
            json!({
                "prompt": generate_test_prompt(),
                "providerOptions": { "google": { "imageConfig": image_config } },
            }),
        )
        .await;

        generate_assert_match_object(
            &generate_only_body(&mock),
            &json!({ "generationConfig": { "imageConfig": image_config } }),
        );
        assert_eq!(generate_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should warn and drop Vertex-only imageConfig fields on the Gemini API
    #[tokio::test]
    async fn should_warn_and_drop_vertex_only_image_config_fields_on_the_gemini_api() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with(
            &model,
            json!({
                "prompt": generate_test_prompt(),
                "providerOptions": {
                    "google": {
                        "imageConfig": { "aspectRatio": "16:9", "prominentPeople": "BLOCK_PROMINENT_PEOPLE" },
                    },
                },
            }),
        )
        .await;

        assert_eq!(
            generate_only_body(&mock)["generationConfig"]["imageConfig"],
            json!({ "aspectRatio": "16:9" })
        );
        assert_eq!(
            generate_json(&result.warnings),
            json!([{
                "type": "other",
                "message": "'imageConfig.prominentPeople' is a Vertex AI option and is ignored with the current Google provider (google.generative-ai).",
            }])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass retrievalConfig in provider options
    #[tokio::test]
    async fn should_pass_retrieval_config_in_provider_options() {
        let mock = generate_mock_with_fixture("google-text");
        let model = generate_test_provider(&mock).chat("gemini-2.0-flash-exp");

        generate_with(
            &model,
            json!({
                "prompt": generate_test_prompt(),
                "tools": [generate_provider_tool("google.google_maps", "google_maps")],
                "providerOptions": {
                    "google": {
                        "retrievalConfig": { "latLng": { "latitude": 34.090199, "longitude": -117.881081 } },
                    },
                },
            }),
        )
        .await;

        generate_assert_match_object(
            &generate_only_body(&mock),
            &json!({
                "tools": [{ "googleMaps": {} }],
                "toolConfig": {
                    "retrievalConfig": { "latLng": { "latitude": 34.090199, "longitude": -117.881081 } },
                },
            }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should include non-image inlineData parts
    #[tokio::test]
    async fn should_include_non_image_inline_data_parts() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "Here is content:" },
                        { "inlineData": { "mimeType": "image/jpeg", "data": "validimagedata" } },
                        { "inlineData": { "mimeType": "application/pdf", "data": "pdfdata" } },
                    ],
                    "role": "model",
                },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": generate_safety_ratings(),
            }],
            "promptFeedback": { "safetyRatings": generate_safety_ratings() },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                { "text": "Here is content:", "type": "text" },
                { "data": { "data": "validimagedata", "type": "data" }, "mediaType": "image/jpeg", "type": "file" },
                { "data": { "data": "pdfdata", "type": "data" }, "mediaType": "application/pdf", "type": "file" },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should correctly parse and separate reasoning parts from text output
    #[tokio::test]
    async fn should_correctly_parse_and_separate_reasoning_parts_from_text_output() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "Visible text part 1. " },
                        { "text": "This is a thought process.", "thought": true },
                        { "text": "Visible text part 2." },
                        { "text": "Another internal thought.", "thought": true },
                    ],
                    "role": "model",
                },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": generate_safety_ratings(),
            }],
            "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 20, "totalTokenCount": 30 },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                { "text": "Visible text part 1. ", "type": "text" },
                { "text": "This is a thought process.", "type": "reasoning" },
                { "text": "Visible text part 2.", "type": "text" },
                { "text": "Another internal thought.", "type": "reasoning" },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should correctly parse thought signatures with reasoning parts
    #[tokio::test]
    async fn should_correctly_parse_thought_signatures_with_reasoning_parts() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "Visible text part 1. ", "thoughtSignature": "sig1" },
                        { "text": "This is a thought process.", "thought": true, "thoughtSignature": "sig2" },
                        { "text": "Visible text part 2.", "thoughtSignature": "sig3" },
                    ],
                    "role": "model",
                },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": generate_safety_ratings(),
            }],
            "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 20, "totalTokenCount": 30 },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                {
                    "providerMetadata": { "google": { "thoughtSignature": "sig1" } },
                    "text": "Visible text part 1. ",
                    "type": "text",
                },
                {
                    "providerMetadata": { "google": { "thoughtSignature": "sig2" } },
                    "text": "This is a thought process.",
                    "type": "reasoning",
                },
                {
                    "providerMetadata": { "google": { "thoughtSignature": "sig3" } },
                    "text": "Visible text part 2.",
                    "type": "text",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should correctly parse thought signatures with function calls
    #[tokio::test]
    async fn should_correctly_parse_thought_signatures_with_function_calls() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": { "name": "test-tool", "args": { "value": "test" } },
                        "thoughtSignature": "func_sig1",
                    }],
                    "role": "model",
                },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": generate_safety_ratings(),
            }],
            "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 20, "totalTokenCount": 30 },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with_test_prompt(&model).await;

        assert_eq!(
            generate_json(&result.content),
            json!([{
                "input": "{\"value\":\"test\"}",
                "providerMetadata": { "google": { "thoughtSignature": "func_sig1" } },
                "toolCallId": "test-id",
                "toolName": "test-tool",
                "type": "tool-call",
            }])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should support includeThoughts with google generative ai provider
    #[tokio::test]
    async fn should_support_include_thoughts_with_google_generative_ai_provider() {
        let mock = generate_mock_with_body(&json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {
                            "text": "let me think about this problem",
                            "thought": true,
                            "thoughtSignature": "reasoning_sig",
                        },
                        { "text": "the answer is 42" },
                    ],
                    "role": "model",
                },
                "finishReason": "STOP",
                "safetyRatings": generate_safety_ratings(),
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 15,
                "totalTokenCount": 25,
                "thoughtsTokenCount": 8,
            },
        }));
        let model = generate_test_provider(&mock).chat("gemini-pro");

        let result = generate_with(
            &model,
            json!({
                "prompt": generate_test_prompt(),
                "providerOptions": {
                    "google": { "thinkingConfig": { "includeThoughts": true, "thinkingBudget": 1024 } },
                },
            }),
        )
        .await;

        assert_eq!(
            generate_json(&result.content),
            json!([
                {
                    "providerMetadata": { "google": { "thoughtSignature": "reasoning_sig" } },
                    "text": "let me think about this problem",
                    "type": "reasoning",
                },
                { "text": "the answer is 42", "type": "text" },
            ])
        );
        assert_eq!(
            generate_json(&result.usage),
            json!({
                "inputTokens": { "cacheRead": 0, "noCache": 10, "total": 10 },
                "outputTokens": { "reasoning": 8, "text": 15, "total": 23 },
                "raw": {
                    "candidatesTokenCount": 15,
                    "promptTokenCount": 10,
                    "thoughtsTokenCount": 8,
                    "totalTokenCount": 25,
                },
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass thinkingLevel in provider options
    #[tokio::test]
    async fn should_pass_thinking_level_in_provider_options() {
        let request_body = generate_body_with_google_options(
            json!({ "thinkingConfig": { "thinkingLevel": "high" } }),
        )
        .await;

        generate_assert_match_object(
            &request_body,
            &json!({ "generationConfig": { "thinkingConfig": { "thinkingLevel": "high" } } }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass thinkingLevel "minimal" in provider options
    #[tokio::test]
    async fn should_pass_thinking_level_minimal_in_provider_options() {
        let request_body = generate_body_with_google_options(
            json!({ "thinkingConfig": { "thinkingLevel": "minimal" } }),
        )
        .await;

        generate_assert_match_object(
            &request_body,
            &json!({ "generationConfig": { "thinkingConfig": { "thinkingLevel": "minimal" } } }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › should pass thinkingLevel "medium" in provider options
    #[tokio::test]
    async fn should_pass_thinking_level_medium_in_provider_options() {
        let request_body = generate_body_with_google_options(
            json!({ "thinkingConfig": { "thinkingLevel": "medium" } }),
        )
        .await;

        generate_assert_match_object(
            &request_body,
            &json!({ "generationConfig": { "thinkingConfig": { "thinkingLevel": "medium" } } }),
        );
    }

    mod top_level_reasoning_option {
        use super::*;

        mod gemini_3_models_thinking_level {
            use super::*;

            /// Asserts the request of `model_id` with `reasoning` sends
            /// `thinkingConfig: { thinkingLevel }`, and returns the result.
            async fn generate_assert_thinking_level(
                model_id: &str,
                reasoning: &str,
                thinking_level: &str,
            ) -> GenerateResult {
                let (request_body, result) =
                    generate_body_with_reasoning(model_id, reasoning).await;
                generate_assert_match_object(
                    &request_body,
                    &json!({ "generationConfig": { "thinkingConfig": { "thinkingLevel": thinking_level } } }),
                );
                result
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should map reasoning "minimal" to thinkingLevel "minimal"
            #[tokio::test]
            async fn should_map_reasoning_minimal_to_thinking_level_minimal() {
                generate_assert_thinking_level("gemini-3-pro-preview", "minimal", "minimal").await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should map reasoning "low" to thinkingLevel "low"
            #[tokio::test]
            async fn should_map_reasoning_low_to_thinking_level_low() {
                generate_assert_thinking_level("gemini-3-pro-preview", "low", "low").await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should map reasoning "medium" to thinkingLevel "medium"
            #[tokio::test]
            async fn should_map_reasoning_medium_to_thinking_level_medium() {
                generate_assert_thinking_level("gemini-3-pro-preview", "medium", "medium").await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should map reasoning "high" to thinkingLevel "high"
            #[tokio::test]
            async fn should_map_reasoning_high_to_thinking_level_high() {
                generate_assert_thinking_level("gemini-3-pro-preview", "high", "high").await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should map reasoning "none" to thinkingLevel "minimal"
            #[tokio::test]
            async fn should_map_reasoning_none_to_thinking_level_minimal() {
                generate_assert_thinking_level("gemini-3-pro-preview", "none", "minimal").await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should coerce reasoning "xhigh" to "high" with compatibility warning
            #[tokio::test]
            async fn should_coerce_reasoning_xhigh_to_high_with_compatibility_warning() {
                let result =
                    generate_assert_thinking_level("gemini-3-pro-preview", "xhigh", "high").await;

                let warnings = generate_json(&result.warnings);
                assert!(
                    warnings.as_array().unwrap().contains(&json!({
                        "type": "compatibility",
                        "feature": "reasoning",
                        "details": "reasoning \"xhigh\" is not directly supported by this model. mapped to effort \"high\".",
                    })),
                    "{warnings}"
                );
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should coerce reasoning "minimal" to thinkingLevel "low" for Gemini 3.7 Flash
            #[tokio::test]
            async fn should_coerce_reasoning_minimal_to_thinking_level_low_for_gemini_3_7_flash() {
                let result =
                    generate_assert_thinking_level("gemini-3.7-flash", "minimal", "low").await;

                let warnings = generate_json(&result.warnings);
                assert!(
                    warnings.as_array().unwrap().contains(&json!({
                        "type": "compatibility",
                        "feature": "reasoning",
                        "details": "reasoning \"minimal\" is not directly supported by this model. mapped to effort \"low\".",
                    })),
                    "{warnings}"
                );
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should coerce reasoning "none" to thinkingLevel "low" for Gemini 3.7 Flash
            #[tokio::test]
            async fn should_coerce_reasoning_none_to_thinking_level_low_for_gemini_3_7_flash() {
                generate_assert_thinking_level("gemini-3.7-flash", "none", "low").await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should map reasoning "$reasoning" to thinkingLevel "$expectedThinkingLevel" for $modelId
            #[tokio::test]
            async fn should_map_reasoning_to_thinking_level_for_model_id() {
                for (model_id, reasoning, expected_thinking_level) in [
                    ("gemini-3.7-flash-video-understanding-eap", "minimal", "low"),
                    ("gemini-3.7-flash-video-understanding-eap", "none", "low"),
                    ("gemini-flash-latest", "minimal", "low"),
                    ("gemini-flash-latest", "none", "low"),
                    ("models/gemini-3.7-flash", "minimal", "low"),
                    ("gemini-3.8-flash", "minimal", "low"),
                    ("gemini-3.10-flash-preview", "minimal", "low"),
                    ("gemini-4.0-flash", "minimal", "low"),
                    ("gemini-3-flash-preview", "minimal", "minimal"),
                    ("gemini-3.6-flash", "minimal", "minimal"),
                    ("gemini-3.7-flash-lite", "minimal", "minimal"),
                    ("gemini-3.10-flash-lite-preview", "minimal", "minimal"),
                    ("gemini-flash-lite-latest", "minimal", "minimal"),
                ] {
                    let (request_body, _) = generate_body_with_reasoning(model_id, reasoning).await;

                    assert_eq!(
                        request_body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
                        json!(expected_thinking_level),
                        "{model_id} {reasoning}"
                    );
                }
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 3 models (thinkingLevel) › should also detect gemini-3.1 models as Gemini 3
            #[tokio::test]
            async fn should_also_detect_gemini_3_1_models_as_gemini_3() {
                generate_assert_thinking_level("gemini-3.1-pro-preview", "medium", "medium").await;
            }
        }

        mod gemini_2_5_models_thinking_budget {
            use super::*;

            /// Asserts the request of `model_id` with `reasoning` sends
            /// `thinkingConfig: { thinkingBudget }`.
            async fn generate_assert_thinking_budget(
                model_id: &str,
                reasoning: &str,
                thinking_budget: i64,
            ) {
                let (request_body, _) = generate_body_with_reasoning(model_id, reasoning).await;
                generate_assert_match_object(
                    &request_body,
                    &json!({ "generationConfig": { "thinkingConfig": { "thinkingBudget": thinking_budget } } }),
                );
            }

            /// `Math.round(65536 * share)`.
            fn generate_share_of_max_output_tokens(share: f64) -> i64 {
                (65536.0 * share).round() as i64
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 2.5 models (thinkingBudget) › should map reasoning "none" to thinkingBudget 0
            #[tokio::test]
            async fn should_map_reasoning_none_to_thinking_budget_0() {
                generate_assert_thinking_budget("gemini-2.5-pro", "none", 0).await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 2.5 models (thinkingBudget) › should map reasoning "minimal" to ~2% of maxOutputTokens
            #[tokio::test]
            async fn should_map_reasoning_minimal_to_2_of_max_output_tokens() {
                generate_assert_thinking_budget(
                    "gemini-2.5-pro",
                    "minimal",
                    generate_share_of_max_output_tokens(0.02),
                )
                .await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 2.5 models (thinkingBudget) › should map reasoning "low" to ~10% of maxOutputTokens
            #[tokio::test]
            async fn should_map_reasoning_low_to_10_of_max_output_tokens() {
                generate_assert_thinking_budget(
                    "gemini-2.5-pro",
                    "low",
                    generate_share_of_max_output_tokens(0.1),
                )
                .await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 2.5 models (thinkingBudget) › should map reasoning "medium" to ~30% of maxOutputTokens
            #[tokio::test]
            async fn should_map_reasoning_medium_to_30_of_max_output_tokens() {
                generate_assert_thinking_budget(
                    "gemini-2.5-pro",
                    "medium",
                    generate_share_of_max_output_tokens(0.3),
                )
                .await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 2.5 models (thinkingBudget) › should map reasoning "high" to ~60% of maxOutputTokens
            #[tokio::test]
            async fn should_map_reasoning_high_to_60_of_max_output_tokens() {
                generate_assert_thinking_budget("gemini-2.5-pro", "high", 32768).await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 2.5 models (thinkingBudget) › should map reasoning "xhigh" to ~90% of maxOutputTokens
            #[tokio::test]
            async fn should_map_reasoning_xhigh_to_90_of_max_output_tokens() {
                generate_assert_thinking_budget("gemini-2.5-pro", "xhigh", 32768).await;
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › Gemini 2.5 models (thinkingBudget) › should use lower maxOutputTokens for flash-lite models
            #[tokio::test]
            async fn should_use_lower_max_output_tokens_for_flash_lite_models() {
                generate_assert_thinking_budget(
                    "gemini-2.5-flash-lite",
                    "medium",
                    generate_share_of_max_output_tokens(0.3),
                )
                .await;
            }
        }

        mod provider_options_precedence {
            use super::*;

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › providerOptions precedence › should use providerOptions thinkingConfig when both reasoning and providerOptions are set
            #[tokio::test]
            async fn should_use_provider_options_thinking_config_when_both_reasoning_and_provider_options_are_set()
             {
                let mock = generate_mock_with_fixture("google-text");
                let model = generate_test_provider(&mock).chat("gemini-pro");

                generate_with(
                    &model,
                    json!({
                        "prompt": generate_test_prompt(),
                        "reasoning": "high",
                        "providerOptions": { "google": { "thinkingConfig": { "thinkingBudget": 999 } } },
                    }),
                )
                .await;

                let body = generate_only_body(&mock);
                generate_assert_match_object(
                    &body,
                    &json!({ "generationConfig": { "thinkingConfig": { "thinkingBudget": 999 } } }),
                );
                assert!(
                    body["generationConfig"]["thinkingConfig"]
                        .get("thinkingLevel")
                        .is_none()
                );
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › providerOptions precedence › should not set thinkingConfig when neither reasoning nor providerOptions are set
            #[tokio::test]
            async fn should_not_set_thinking_config_when_neither_reasoning_nor_provider_options_are_set()
             {
                let mock = generate_mock_with_fixture("google-text");
                let model = generate_test_provider(&mock).chat("gemini-pro");

                generate_with_test_prompt(&model).await;

                let body = generate_only_body(&mock);
                assert!(body["generationConfig"].get("thinkingConfig").is_none());
            }

            /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › top-level reasoning option › providerOptions precedence › should not set thinkingConfig when reasoning is "provider-default"
            #[tokio::test]
            async fn should_not_set_thinking_config_when_reasoning_is_provider_default() {
                let mock = generate_mock_with_fixture("google-text");
                let model = generate_test_provider(&mock).chat("gemini-pro");

                generate_with(
                    &model,
                    json!({ "prompt": generate_test_prompt(), "reasoning": "provider-default" }),
                )
                .await;

                let body = generate_only_body(&mock);
                assert!(body["generationConfig"].get("thinkingConfig").is_none());
            }
        }
    }

    mod provider_metadata_key_based_on_provider_string {
        use super::*;

        /// The body of the metadata key tests: one `Hello!` candidate with
        /// grounding metadata, safety ratings, prompt feedback and usage.
        fn generate_hello_with_grounding_body() -> Value {
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "Hello!" }], "role": "model" },
                    "finishReason": "STOP",
                    "safetyRatings": generate_safety_ratings(),
                    "groundingMetadata": { "webSearchQueries": ["test query"] },
                }],
                "promptFeedback": { "safetyRatings": generate_safety_ratings() },
                "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3 },
            })
        }

        /// A one-candidate body with `parts`, safety ratings, prompt feedback
        /// and usage.
        fn generate_metadata_parts_body(parts: Value) -> Value {
            json!({
                "candidates": [{
                    "content": { "parts": parts, "role": "model" },
                    "finishReason": "STOP",
                    "safetyRatings": generate_safety_ratings(),
                }],
                "promptFeedback": { "safetyRatings": generate_safety_ratings() },
                "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3 },
            })
        }

        /// Asserts `metadata[key]` has object `promptFeedback` and
        /// `groundingMetadata` and an array `safetyRatings`.
        fn generate_assert_response_metadata_shape(metadata: &Value, key: &str) {
            let namespace = &metadata[key];
            assert!(namespace["promptFeedback"].is_object(), "{metadata}");
            assert!(namespace["groundingMetadata"].is_object(), "{metadata}");
            assert!(namespace["safetyRatings"].is_array(), "{metadata}");
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › providerMetadata key based on provider string › should use "vertex" as providerMetadata key when provider includes "vertex"
        #[tokio::test]
        async fn should_use_vertex_as_provider_metadata_key_when_provider_includes_vertex() {
            let mock = generate_mock_with_body(&generate_hello_with_grounding_body());
            let model = generate_direct_model(
                "gemini-pro",
                "google.vertex.chat",
                &[("x-goog-api-key", "test-api-key")],
                &mock,
            );

            let result = generate_with_test_prompt(&model).await;

            let metadata = generate_json(&result.provider_metadata);
            assert!(metadata.get("vertex").is_some(), "{metadata}");
            assert!(metadata.get("google").is_none(), "{metadata}");
            generate_assert_response_metadata_shape(&metadata, "vertex");
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › providerMetadata key based on provider string › should use "google" as providerMetadata key when provider does not include "vertex"
        #[tokio::test]
        async fn should_use_google_as_provider_metadata_key_when_provider_does_not_include_vertex()
        {
            let mock = generate_mock_with_body(&generate_hello_with_grounding_body());
            let model = generate_direct_model(
                "gemini-pro",
                "google.generative-ai",
                &[("x-goog-api-key", "test-api-key")],
                &mock,
            );

            let result = generate_with_test_prompt(&model).await;

            let metadata = generate_json(&result.provider_metadata);
            assert!(metadata.get("google").is_some(), "{metadata}");
            assert!(metadata.get("vertex").is_none(), "{metadata}");
            generate_assert_response_metadata_shape(&metadata, "google");
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › providerMetadata key based on provider string › should use "vertex" as providerMetadata key in thoughtSignature content when provider includes "vertex"
        #[tokio::test]
        async fn should_use_vertex_as_provider_metadata_key_in_thought_signature_content() {
            let mock = generate_mock_with_body(&generate_metadata_parts_body(json!([
                { "text": "thinking...", "thought": true, "thoughtSignature": "sig123" },
                { "text": "Final answer" },
            ])));
            let model = generate_direct_model(
                "gemini-pro",
                "google.vertex.chat",
                &[("x-goog-api-key", "test-api-key")],
                &mock,
            );

            let result = generate_with_test_prompt(&model).await;

            let content = generate_json(&result.content);
            let reasoning_part = content
                .as_array()
                .unwrap()
                .iter()
                .find(|part| part["type"] == "reasoning")
                .expect("reasoning part");
            let metadata = &reasoning_part["providerMetadata"];
            assert!(metadata.get("vertex").is_some(), "{metadata}");
            assert!(metadata.get("google").is_none(), "{metadata}");
            generate_assert_match_object(
                &metadata["vertex"],
                &json!({ "thoughtSignature": "sig123" }),
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doGenerate › providerMetadata key based on provider string › should use "vertex" as providerMetadata key in tool call content when provider includes "vertex"
        #[tokio::test]
        async fn should_use_vertex_as_provider_metadata_key_in_tool_call_content() {
            let mock = generate_mock_with_body(&generate_metadata_parts_body(json!([{
                "functionCall": { "name": "test-tool", "args": { "value": "test" } },
                "thoughtSignature": "tool_sig",
            }])));
            let model = generate_direct_model(
                "gemini-pro",
                "google.vertex.chat",
                &[("x-goog-api-key", "test-api-key")],
                &mock,
            );

            let result = generate_with_test_prompt(&model).await;

            let content = generate_json(&result.content);
            let tool_call_part = content
                .as_array()
                .unwrap()
                .iter()
                .find(|part| part["type"] == "tool-call")
                .expect("tool call part");
            let metadata = &tool_call_part["providerMetadata"];
            assert!(metadata.get("vertex").is_some(), "{metadata}");
            assert!(metadata.get("google").is_none(), "{metadata}");
            generate_assert_match_object(
                &metadata["vertex"],
                &json!({ "thoughtSignature": "tool_sig" }),
            );
        }
    }
}

mod grounding_metadata_schema {
    use super::*;

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates complete grounding metadata with web search results
    #[test]
    fn validates_complete_grounding_metadata_with_web_search_results() {
        let metadata = json!({
            "webSearchQueries": ["What's the weather in Chicago this weekend?"],
            "searchEntryPoint": {
                "renderedContent": "Sample rendered content for search results",
            },
            "groundingChunks": [
                {
                    "web": {
                        "uri": "https://example.com/weather",
                        "title": "Chicago Weather Forecast",
                    },
                },
            ],
            "groundingSupports": [
                {
                    "segment": {
                        "startIndex": 0,
                        "endIndex": 65,
                        "text": "Chicago weather changes rapidly, so layers let you adjust easily.",
                    },
                    "groundingChunkIndices": [0],
                    "confidenceScores": [0.99],
                },
            ],
            "retrievalMetadata": {
                "webDynamicRetrievalScore": 0.96879,
            },
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates groundingChunks[].web with missing title
    #[test]
    fn validates_grounding_chunks_web_with_missing_title() {
        let metadata = json!({
            "groundingChunks": [
                {
                    "web": {
                        // Missing `title`
                        "uri": "https://example.com/weather",
                    },
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates complete grounding metadata with Vertex AI Search results
    #[test]
    fn validates_complete_grounding_metadata_with_vertex_ai_search_results() {
        let metadata = json!({
            "retrievalQueries": ["How to make appointment to renew driving license?"],
            "groundingChunks": [
                {
                    "retrievedContext": {
                        "uri": "https://vertexaisearch.cloud.google.com/grounding-api-redirect/AXiHM.....QTN92V5ePQ==",
                        "title": "dmv",
                    },
                },
            ],
            "groundingSupports": [
                {
                    "segment": {
                        "startIndex": 25,
                        "endIndex": 147,
                    },
                    "segment_text": "ipsum lorem ...",
                    "supportChunkIndices": [1, 2],
                    "confidenceScore": [0.9541752, 0.97726375],
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates groundingChunks[].retrievedContext with missing title
    #[test]
    fn validates_grounding_chunks_retrieved_context_with_missing_title() {
        let metadata = json!({
            "groundingChunks": [
                {
                    "retrievedContext": {
                        // Missing `title`
                        "uri": "https://vertexaisearch.cloud.google.com/grounding-api-redirect/AXiHM.....QTN92V5ePQ==",
                    },
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates groundingChunks[].retrievedContext with fileSearchStore (new format)
    #[test]
    fn validates_grounding_chunks_retrieved_context_with_file_search_store_new_format() {
        let metadata = json!({
            "groundingChunks": [
                {
                    "retrievedContext": {
                        "text": "Sample content for testing...",
                        "fileSearchStore": "fileSearchStores/test-store-xyz",
                        "title": "Test Document",
                    },
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates groundingChunks[].retrievedContext with fileSearchStore and missing uri
    #[test]
    fn validates_grounding_chunks_retrieved_context_with_file_search_store_and_missing_uri() {
        let metadata = json!({
            "groundingChunks": [
                {
                    "retrievedContext": {
                        "text": "Content without uri field",
                        "fileSearchStore": "fileSearchStores/store-abc",
                        // Missing `uri` - should still be valid
                    },
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates groundingChunks[].image
    #[test]
    fn validates_grounding_chunks_image() {
        let metadata = json!({
            "imageSearchQueries": ["Super Bowl halftime show in space"],
            "groundingChunks": [
                {
                    "image": {
                        "sourceUri": "https://example.com/article",
                        "imageUri": "https://example.com/image.jpg",
                        "title": "Image Title",
                        "domain": "example.com",
                    },
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates groundingChunks[].image with missing optional fields
    #[test]
    fn validates_grounding_chunks_image_with_missing_optional_fields() {
        let metadata = json!({
            "groundingChunks": [
                {
                    "image": {
                        "sourceUri": "https://example.com/article",
                        "imageUri": "https://example.com/image.jpg",
                    },
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates partial grounding metadata
    #[test]
    fn validates_partial_grounding_metadata() {
        let metadata = json!({
            "webSearchQueries": ["sample query"],
            // Missing other optional fields
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates empty grounding metadata
    #[test]
    fn validates_empty_grounding_metadata() {
        let metadata = json!({});

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates grounding metadata with maps chunks
    #[test]
    fn validates_grounding_metadata_with_maps_chunks() {
        let metadata = json!({
            "groundingChunks": [
                {
                    "maps": {
                        "uri": "https://maps.google.com/maps?cid=12345",
                        "title": "Best Italian Restaurant",
                        "text": "A great Italian restaurant",
                        "placeId": "ChIJ12345",
                    },
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates groundingChunks[].maps with missing optional fields
    #[test]
    fn validates_grounding_chunks_maps_with_missing_optional_fields() {
        let metadata = json!({
            "groundingChunks": [
                {
                    "maps": {
                        "uri": "https://maps.google.com/maps?cid=12345",
                    },
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates metadata with empty retrievalMetadata
    #[test]
    fn validates_metadata_with_empty_retrieval_metadata() {
        let metadata = json!({
            "webSearchQueries": ["sample query"],
            "retrievalMetadata": {},
        });

        assert!(grounding_metadata_parses(metadata));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › groundingMetadataSchema › validates groundingSupports with missing segment
    #[test]
    fn validates_grounding_supports_with_missing_segment() {
        let metadata = json!({
            "groundingSupports": [
                {
                    // Missing `segment`
                    "groundingChunkIndices": [0],
                },
            ],
        });

        assert!(grounding_metadata_parses(metadata));
    }
}

mod url_context_metadata {
    use super::*;

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › urlContextMetadata › validates complete url context output
    #[test]
    fn validates_complete_url_context_output() {
        let output = json!({
            "urlMetadata": [
                {
                    "retrievedUrl": "https://example.com/weather",
                    "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS",
                },
            ],
        });

        assert!(url_context_metadata_parses(output));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › urlContextMetadata › validates empty url context output
    #[test]
    fn validates_empty_url_context_output() {
        let output = json!({ "urlMetadata": [] });

        assert!(url_context_metadata_parses(output));
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › urlContextMetadata › validates empty object without urlMetadata key
    #[test]
    fn validates_empty_object_without_url_metadata_key() {
        let output = json!({});

        assert!(url_context_metadata_parses(output));
    }
}

mod do_stream {
    use super::*;

    mod text {
        use super::*;

        fn text_fixture_mock() -> Arc<MockFetch> {
            sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-text.chunks.txt"
            )))
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › text › should stream text deltas
        #[tokio::test]
        async fn should_stream_text_deltas() {
            let mock = text_fixture_mock();
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

            assert_eq!(
                Value::Array(events),
                stream_snapshot(include_str!(
                    "snapshots/do_stream_text_stream_text_deltas.json"
                ))
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › text › should include raw chunks when includeRawChunks is enabled
        #[tokio::test]
        async fn should_include_raw_chunks_when_include_raw_chunks_is_enabled() {
            let mock = text_fixture_mock();
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let chunks = stream_events_of(&model, json!({ "includeRawChunks": true })).await;

            assert_eq!(
                Value::Array(stream_events_of_type(&chunks, &["raw"])),
                stream_snapshot(include_str!("snapshots/do_stream_text_raw_chunks.json"))
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › text › should not include raw chunks when includeRawChunks is false
        #[tokio::test]
        async fn should_not_include_raw_chunks_when_include_raw_chunks_is_false() {
            let mock = text_fixture_mock();
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let chunks = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

            assert_eq!(stream_events_of_type(&chunks, &["raw"]).len(), 0);
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › text › should emit response-metadata with the provider responseId
        #[tokio::test]
        async fn should_emit_response_metadata_with_the_provider_response_id() {
            // Gemini repeats `responseId` on every chunk; the provider emits a single
            // response-metadata part carrying it (see google-text.chunks.txt fixture).
            let mock = text_fixture_mock();
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
            let response_metadata = stream_events_of_type(&events, &["response-metadata"]);

            assert_eq!(
                Value::Array(response_metadata),
                json!([{ "type": "response-metadata", "id": "bH6LaZW8Fp_3nsEPqtaSwQ4" }])
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › text › should expose the raw response headers
        #[tokio::test]
        async fn should_expose_the_raw_response_headers() {
            let mock = MockFetch::new(vec![sse_stream_response(
                sse_fixture_chunks(include_str!("fixtures/google-text.chunks.txt")),
                &[("test-header", "test-value")],
            )]);
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let result = model
                .do_stream(&stream_call_options_with(
                    json!({ "includeRawChunks": false }),
                ))
                .await
                .expect("doStream succeeds");

            assert_eq!(
                serde_json::to_value(result.response.and_then(|response| response.headers))
                    .unwrap(),
                json!({
                    "cache-control": "no-cache",
                    "connection": "keep-alive",
                    "content-type": "text/event-stream",
                    "test-header": "test-value",
                })
            );
        }
    }

    mod tool_call {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › tool-call › should stream tool call
        #[tokio::test]
        async fn should_stream_tool_call() {
            let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-tool-call.chunks.txt"
            )));
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let events = stream_events_of(
                &model,
                json!({
                    "tools": [stream_string_function_tool("weather", "location")],
                    "includeRawChunks": false,
                }),
            )
            .await;

            assert_eq!(
                Value::Array(events),
                stream_snapshot(include_str!("snapshots/do_stream_tool_call.json"))
            );
        }
    }

    mod reasoning {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › reasoning › should stream reasoning and text parts
        #[tokio::test]
        async fn should_stream_reasoning_and_text_parts() {
            let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-reasoning.chunks.txt"
            )));
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

            assert_eq!(
                Value::Array(events),
                stream_snapshot(include_str!("snapshots/do_stream_reasoning.json"))
            );
        }
    }

    mod reasoning_gemini3 {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › reasoning-gemini3 › should stream reasoning with thoughtSignature
        #[tokio::test]
        async fn should_stream_reasoning_with_thought_signature() {
            let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-reasoning-gemini3.chunks.txt"
            )));
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

            assert_eq!(
                Value::Array(events),
                stream_snapshot(include_str!("snapshots/do_stream_reasoning_gemini3.json"))
            );
        }
    }

    mod tool_call_gemini3 {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › tool-call-gemini3 › should stream tool call with thoughtSignature
        #[tokio::test]
        async fn should_stream_tool_call_with_thought_signature() {
            let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-tool-call-gemini3.chunks.txt"
            )));
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

            assert_eq!(
                Value::Array(events),
                stream_snapshot(include_str!("snapshots/do_stream_tool_call_gemini3.json"))
            );
        }
    }

    mod streaming_tool_call_arguments {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › streaming-tool-call-arguments › should stream partial function call arguments with parallel tool calls
        #[tokio::test]
        async fn should_stream_partial_function_call_arguments_with_parallel_tool_calls() {
            let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-stream-tool-call-arguments.chunks.txt"
            )));
            let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

            let events = stream_events_of(
                &vertex_model,
                json!({
                    "includeRawChunks": false,
                    "tools": [stream_string_function_tool("getWeather", "location")],
                }),
            )
            .await;

            assert_eq!(
                Value::Array(events),
                stream_snapshot(include_str!(
                    "snapshots/do_stream_streaming_tool_call_arguments.json"
                ))
            );
        }
    }

    mod streaming_tool_call_array_arguments {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › streaming-tool-call-array-arguments › should finalize streamed function call arguments when the final partialArgs chunk omits willContinue
        #[tokio::test]
        async fn should_finalize_streamed_arguments_when_final_partial_args_chunk_omits_will_continue()
         {
            let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-stream-tool-call-array-arguments-missing-terminal-function-call.chunks.txt"
            )));
            let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

            let events = stream_events_of(
                &vertex_model,
                json!({
                    "includeRawChunks": false,
                    "tools": [
                        {
                            "type": "function",
                            "name": "writeItems",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "operations": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "action": { "type": "string" },
                                                "description": { "type": "string" },
                                                "itemid": { "type": "string" },
                                                "price": { "type": "number" },
                                            },
                                            "required": ["action", "description", "itemid", "price"],
                                            "additionalProperties": false,
                                        },
                                    },
                                },
                                "required": ["operations"],
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        },
                    ],
                    "providerOptions": {
                        "vertex": {
                            "streamFunctionCallArguments": true,
                        },
                    },
                }),
            )
            .await;
            let input_deltas = stream_events_of_type(&events, &["tool-input-delta"]);
            let tool_calls = stream_events_of_type(&events, &["tool-call"]);
            let finish = stream_finish_event(&events);

            assert_eq!(
                input_deltas
                    .iter()
                    .map(|event| event["delta"].as_str().unwrap())
                    .collect::<String>(),
                r#"{"operations":[{"action":"add","description":"Fresh red apple","itemid":"apple_001","price":0.5},{"action":"add","description":"Ripe yellow banana","itemid":"banana_001","price":0.3}]}"#
            );
            assert_eq!(
                tool_calls
                    .iter()
                    .map(|call| json!({ "toolName": call["toolName"], "input": call["input"] }))
                    .collect::<Vec<_>>(),
                vec![json!({
                    "input": r#"{"operations":[{"action":"add","description":"Fresh red apple","itemid":"apple_001","price":0.5},{"action":"add","description":"Ripe yellow banana","itemid":"banana_001","price":0.3}]}"#,
                    "toolName": "writeItems",
                })]
            );
            assert_eq!(finish["finishReason"]["unified"], "tool-calls");
        }
    }

    mod streaming_no_args_tool_call {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › streaming-no-args-tool-call › should emit no-args function calls and preserve thoughtSignature alongside streamed-args calls
        #[tokio::test]
        async fn should_emit_no_args_calls_and_preserve_thought_signature_with_streamed_args_calls()
        {
            let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-stream-no-args-tool-call.chunks.txt"
            )));
            let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

            let events = stream_events_of(
                &vertex_model,
                json!({
                    "includeRawChunks": false,
                    "tools": [
                        {
                            "type": "function",
                            "name": "read_theme",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        },
                        stream_string_function_tool("read_screen", "id"),
                    ],
                    "providerOptions": {
                        "vertex": {
                            "streamFunctionCallArguments": true,
                        },
                    },
                }),
            )
            .await;

            // The fixture is a real Vertex recording where the model picked the
            // no-args `read_theme` first (carrying the response's only
            // thoughtSignature), then streamed three parallel `read_screen` calls
            // for A, B, C. Pre-fix the read_theme call and its signature were
            // dropped entirely.
            let tool_calls = stream_events_of_type(&events, &["tool-call"]);
            assert_eq!(
                tool_calls
                    .iter()
                    .map(|call| json!({ "toolName": call["toolName"], "input": call["input"] }))
                    .collect::<Vec<_>>(),
                vec![
                    json!({ "toolName": "read_theme", "input": "{}" }),
                    json!({ "toolName": "read_screen", "input": r#"{"id":"A"}"# }),
                    json!({ "toolName": "read_screen", "input": r#"{"id":"B"}"# }),
                    json!({ "toolName": "read_screen", "input": r#"{"id":"C"}"# }),
                ]
            );

            let read_theme_call = tool_calls
                .iter()
                .find(|call| call["toolName"] == "read_theme")
                .unwrap();
            let signature = &read_theme_call["providerMetadata"]["vertex"]["thoughtSignature"];
            assert!(signature.is_string());
            assert!(signature.as_str().unwrap().len() > 100);
            assert_eq!(
                &read_theme_call["providerMetadata"]["googleVertex"]["thoughtSignature"],
                signature
            );

            // The no-args call must emit start/end framing too, so the signature
            // is exposed at every stage downstream consumers might inspect.
            let tool_input_starts = stream_events_of_type(&events, &["tool-input-start"]);
            let no_args_start = tool_input_starts
                .iter()
                .find(|event| event["toolName"] == "read_theme");
            assert_eq!(
                no_args_start.map(|event| &event["providerMetadata"]["vertex"]["thoughtSignature"]),
                Some(signature)
            );

            let tool_input_ends = stream_events_of_type(&events, &["tool-input-end"]);
            let no_args_end = tool_input_ends.iter().find(|event| {
                &event["providerMetadata"]["vertex"]["thoughtSignature"] == signature
            });
            assert!(no_args_end.is_some());

            // The no-args branch must not emit any tool-input-delta events.
            let deltas = stream_events_of_type(&events, &["tool-input-delta"]);
            assert!(deltas.iter().all(
                |event| &event["providerMetadata"]["vertex"]["thoughtSignature"] != signature
            ));
        }
    }

    mod streaming_tool_call_arguments_nested {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › streaming-tool-call-arguments-nested › should stream nested partial function call arguments into proper nested JSON
        #[tokio::test]
        async fn should_stream_nested_partial_function_call_arguments_into_proper_nested_json() {
            let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
                "fixtures/google-vertex-stream-tool-call-arguments-nested.1.chunks.txt"
            )));
            let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

            let events = stream_events_of(
                &vertex_model,
                json!({
                    "includeRawChunks": false,
                    "tools": [
                        {
                            "type": "function",
                            "name": "cookRecipe",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "recipe": {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string" },
                                            "ingredients": {
                                                "type": "array",
                                                "items": {
                                                    "type": "object",
                                                    "properties": {
                                                        "name": { "type": "string" },
                                                        "amount": { "type": "string" },
                                                    },
                                                },
                                            },
                                            "steps": { "type": "array", "items": { "type": "string" } },
                                        },
                                    },
                                },
                            },
                        },
                    ],
                }),
            )
            .await;

            assert_eq!(
                Value::Array(events),
                stream_snapshot(include_str!(
                    "snapshots/do_stream_streaming_tool_call_arguments_nested.json"
                ))
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should expose grounding metadata in provider metadata on finish
    #[tokio::test]
    async fn should_expose_grounding_metadata_in_provider_metadata_on_finish() {
        let mock = sse_stream_mock(sse_text_chunks(
            &["test"],
            Some(json!({
                "webSearchQueries": ["What's the weather in Chicago this weekend?"],
                "searchEntryPoint": {
                    "renderedContent": "Sample rendered content for search results",
                },
                "groundingChunks": [
                    {
                        "web": {
                            "uri": "https://example.com/weather",
                            "title": "Chicago Weather Forecast",
                        },
                    },
                ],
                "groundingSupports": [
                    {
                        "segment": {
                            "startIndex": 0,
                            "endIndex": 65,
                            "text": "Chicago weather changes rapidly, so layers let you adjust easily.",
                        },
                        "groundingChunkIndices": [0],
                        "confidenceScores": [0.99],
                    },
                ],
                "retrievalMetadata": {
                    "webDynamicRetrievalScore": 0.96879,
                },
            })),
            None,
        ));
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"]["groundingMetadata"],
            json!({
                "groundingChunks": [
                    {
                        "web": {
                            "title": "Chicago Weather Forecast",
                            "uri": "https://example.com/weather"
                        }
                    }
                ],
                "groundingSupports": [
                    {
                        "confidenceScores": [
                            0.99
                        ],
                        "groundingChunkIndices": [
                            0
                        ],
                        "segment": {
                            "endIndex": 65,
                            "startIndex": 0,
                            "text": "Chicago weather changes rapidly, so layers let you adjust easily."
                        }
                    }
                ],
                "retrievalMetadata": {
                    "webDynamicRetrievalScore": 0.96879
                },
                "searchEntryPoint": {
                    "renderedContent": "Sample rendered content for search results"
                },
                "webSearchQueries": [
                    "What's the weather in Chicago this weekend?"
                ]
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should preserve grounding metadata when it arrives before the finishReason chunk
    #[tokio::test]
    async fn should_preserve_grounding_metadata_when_it_arrives_before_the_finish_reason_chunk() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "hello" }], "role": "model" },
                        "index": 0,
                        "groundingMetadata": {
                            "webSearchQueries": ["super bowl 2026 halftime show"],
                            "groundingChunks": [
                                {
                                    "web": {
                                        "uri": "https://example.com/superbowl",
                                        "title": "Super Bowl 2026",
                                    },
                                },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": " world" }], "role": "model" },
                        "finishReason": "STOP",
                        "index": 0,
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 38,
                    "candidatesTokenCount": 1335,
                    "totalTokenCount": 1890,
                },
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"]["groundingMetadata"],
            json!({
                "webSearchQueries": ["super bowl 2026 halftime show"],
                "groundingChunks": [
                    {
                        "web": {
                            "uri": "https://example.com/superbowl",
                            "title": "Super Bowl 2026",
                        },
                    },
                ],
            })
        );

        assert_eq!(stream_events_of_type(&events, &["source"]).len(), 1);
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should preserve url context metadata when it arrives before the finishReason chunk
    #[tokio::test]
    async fn should_preserve_url_context_metadata_when_it_arrives_before_the_finish_reason_chunk() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "hello" }], "role": "model" },
                        "index": 0,
                        "urlContextMetadata": {
                            "urlMetadata": [
                                {
                                    "retrievedUrl": "https://example.com/page",
                                    "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS",
                                },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": " world" }], "role": "model" },
                        "finishReason": "STOP",
                        "index": 0,
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 10,
                    "candidatesTokenCount": 5,
                    "totalTokenCount": 15,
                },
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"]["urlContextMetadata"],
            json!({
                "urlMetadata": [
                    {
                        "retrievedUrl": "https://example.com/page",
                        "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should expose url context metadata in provider metadata on finish
    #[tokio::test]
    async fn should_expose_url_context_metadata_in_provider_metadata_on_finish() {
        let mock = sse_stream_mock(sse_text_chunks(
            &["test"],
            None,
            Some(json!({
                "urlMetadata": [
                    {
                        "retrievedUrl": "https://example.com/weather",
                        "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS",
                    },
                ],
            })),
        ));
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"]["urlContextMetadata"],
            json!({
                "urlMetadata": [
                    {
                        "retrievedUrl": "https://example.com/weather",
                        "urlRetrievalStatus": "URL_RETRIEVAL_STATUS_SUCCESS"
                    }
                ]
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should expose safety ratings in provider metadata on finish
    #[tokio::test]
    async fn should_expose_safety_ratings_in_provider_metadata_on_finish() {
        let mock = sse_stream_mock(vec![
            concat!(
                r#"data: {"candidates": [{"content": {"parts": [{"text": "test"}],"role": "model"},"#,
                r#""finishReason": "STOP","index": 0,"safetyRatings": ["#,
                r#"{"category": "HARM_CATEGORY_DANGEROUS_CONTENT","probability": "NEGLIGIBLE","#,
                r#""probabilityScore": 0.1,"severity": "LOW","severityScore": 0.2,"blocked": false}]}]}"#,
                "\n\n"
            )
            .to_owned(),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"]["safetyRatings"],
            json!([
                {
                    "blocked": false,
                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                    "probability": "NEGLIGIBLE",
                    "probabilityScore": 0.1,
                    "severity": "LOW",
                    "severityScore": 0.2
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should expose PromptFeedback in provider metadata on finish
    #[tokio::test]
    async fn should_expose_prompt_feedback_in_provider_metadata_on_finish() {
        let mock = sse_stream_mock(vec![
            concat!(
                r#"data: {"candidates": [{"content": {"parts": [{"text": "No"}],"role": "model"},"#,
                r#""finishReason": "PROHIBITED_CONTENT","index": 0}],"#,
                r#""promptFeedback": {"blockReason": "PROHIBITED_CONTENT","safetyRatings": ["#,
                r#"{"category": "HARM_CATEGORY_SEXUALLY_EXPLICIT","probability": "NEGLIGIBLE"},"#,
                r#"{"category": "HARM_CATEGORY_HATE_SPEECH","probability": "NEGLIGIBLE"},"#,
                r#"{"category": "HARM_CATEGORY_HARASSMENT","probability": "NEGLIGIBLE"},"#,
                r#"{"category": "HARM_CATEGORY_DANGEROUS_CONTENT","probability": "NEGLIGIBLE"}]}}"#,
                "\n\n"
            )
            .to_owned(),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({})).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"]["promptFeedback"],
            json!({
                "blockReason": "PROHIBITED_CONTENT",
                "safetyRatings": [
                    {
                        "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
                        "probability": "NEGLIGIBLE"
                    },
                    {
                        "category": "HARM_CATEGORY_HATE_SPEECH",
                        "probability": "NEGLIGIBLE"
                    },
                    {
                        "category": "HARM_CATEGORY_HARASSMENT",
                        "probability": "NEGLIGIBLE"
                    },
                    {
                        "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                        "probability": "NEGLIGIBLE"
                    }
                ]
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should surface streamed prompt-level blocks without candidates
    #[tokio::test]
    async fn should_surface_streamed_prompt_level_blocks_without_candidates() {
        let mock = sse_stream_mock(vec![sse_data_chunk(&json!({
            "promptFeedback": {
                "blockReason": "PROHIBITED_CONTENT",
            },
            "usageMetadata": {
                "promptTokenCount": 9,
                "totalTokenCount": 9,
                "serviceTier": "standard",
            },
            "responseId": "blocked-response-id",
        }))]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({})).await;

        assert!(events.contains(&json!({
            "type": "response-metadata",
            "id": "blocked-response-id",
        })));
        assert_stream_match_object(
            &stream_finish_event(&events),
            &json!({
                "type": "finish",
                "finishReason": {
                    "unified": "content-filter",
                    "raw": "PROHIBITED_CONTENT",
                },
                "providerMetadata": {
                    "google": {
                        "promptFeedback": {
                            "blockReason": "PROHIBITED_CONTENT",
                        },
                        "groundingMetadata": null,
                        "urlContextMetadata": null,
                        "safetyRatings": null,
                        "usageMetadata": {
                            "promptTokenCount": 9,
                            "totalTokenCount": 9,
                            "serviceTier": "standard",
                        },
                        "finishMessage": null,
                        "serviceTier": "standard",
                    },
                },
            }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should not classify the default prompt block reason %j as a content filter
    #[tokio::test]
    async fn should_not_classify_the_default_prompt_block_reason_as_a_content_filter() {
        for block_reason in ["", "BLOCK_REASON_UNSPECIFIED", "BLOCKED_REASON_UNSPECIFIED"] {
            let mock = sse_stream_mock(vec![sse_data_chunk(&json!({
                "candidates": [],
                "promptFeedback": { "blockReason": block_reason },
                "usageMetadata": {
                    "promptTokenCount": 9,
                    "totalTokenCount": 9,
                },
            }))]);
            let model = stream_test_provider(&mock).chat("gemini-pro");

            let finish_event = stream_finish_event(&stream_events_of(&model, json!({})).await);

            assert_eq!(
                finish_event["finishReason"],
                json!({ "unified": "other" }),
                "{block_reason:?}"
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should preserve prompt feedback and trailing usage from separate chunks
    #[tokio::test]
    async fn should_preserve_prompt_feedback_and_trailing_usage_from_separate_chunks() {
        let prompt_feedback = json!({
            "blockReason": "BLOCK_REASON_UNSPECIFIED",
            "safetyRatings": [],
        });
        let usage_metadata = json!({
            "promptTokenCount": 10,
            "candidatesTokenCount": 3,
            "totalTokenCount": 13,
        });

        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({ "promptFeedback": prompt_feedback })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "role": "model",
                            "parts": [{ "text": "Fixture text." }],
                        },
                        "finishReason": "STOP",
                    },
                ],
            })),
            sse_data_chunk(&json!({ "usageMetadata": usage_metadata })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let finish_event = stream_finish_event(&stream_events_of(&model, json!({})).await);

        assert_stream_match_object(
            &finish_event,
            &json!({
                "type": "finish",
                "finishReason": {
                    "unified": "stop",
                    "raw": "STOP",
                },
                "usage": {
                    "outputTokens": {
                        "total": 3,
                    },
                },
                "providerMetadata": {
                    "google": {
                        "promptFeedback": prompt_feedback,
                        "usageMetadata": usage_metadata,
                    },
                },
            }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should keep a confirmed prompt block terminal across later chunks
    #[tokio::test]
    async fn should_keep_a_confirmed_prompt_block_terminal_across_later_chunks() {
        let usage_metadata = json!({
            "promptTokenCount": 10,
            "candidatesTokenCount": 0,
            "totalTokenCount": 10,
        });

        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "promptFeedback": { "blockReason": "SAFETY" },
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "role": "model",
                            "parts": [{ "text": "Fixture text." }],
                        },
                        "finishReason": "STOP",
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "promptFeedback": {
                    "blockReason": "BLOCK_REASON_UNSPECIFIED",
                },
                "usageMetadata": usage_metadata,
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({})).await;

        assert_eq!(
            stream_events_of_type(&events, &["text-delta"]),
            Vec::<Value>::new()
        );
        assert_stream_match_object(
            &stream_finish_event(&events),
            &json!({
                "type": "finish",
                "finishReason": {
                    "unified": "content-filter",
                    "raw": "SAFETY",
                },
                "providerMetadata": {
                    "google": {
                        "promptFeedback": { "blockReason": "SAFETY" },
                        "usageMetadata": usage_metadata,
                    },
                },
            }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should expose finishMessage in provider metadata on finish
    #[tokio::test]
    async fn should_expose_finish_message_in_provider_metadata_on_finish() {
        let mock = sse_stream_mock(vec![sse_data_chunk(&json!({
            "candidates": [
                {
                    "content": {},
                    "finishReason": "MALFORMED_FUNCTION_CALL",
                    "finishMessage": "Malformed function call: print(default_api.create(name='test'))",
                },
            ],
            "usageMetadata": {
                "promptTokenCount": 130,
                "totalTokenCount": 130,
                "promptTokensDetails": [{ "modality": "TEXT", "tokenCount": 130 }],
            },
        }))]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({})).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"]["finishMessage"],
            "Malformed function call: print(default_api.create(name='test'))"
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should expose null finishMessage in provider metadata on finish when not present
    #[tokio::test]
    async fn should_expose_null_finish_message_in_provider_metadata_on_finish_when_not_present() {
        let mock = sse_stream_mock(sse_text_chunks(&["test"], None, None));
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({})).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"].get("finishMessage"),
            Some(&Value::Null)
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should read serviceTier from chunk usageMetadata
    #[tokio::test]
    async fn should_read_service_tier_from_chunk_usage_metadata() {
        // Gemini streaming does not set the x-gemini-service-tier response header
        // but does include serviceTier in each chunk's usageMetadata.
        let mock = sse_stream_mock(vec![sse_data_chunk(&json!({
            "candidates": [
                {
                    "content": { "parts": [{ "text": "Blue" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0,
                },
            ],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 1,
                "totalTokenCount": 11,
                "serviceTier": "priority",
            },
        }))]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({})).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"]["serviceTier"],
            "priority"
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should preserve the final complete raw usage metadata
    #[tokio::test]
    async fn should_preserve_the_final_complete_raw_usage_metadata() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "Blue" }], "role": "model" },
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 1,
                    "candidatesTokenCount": 1,
                    "totalTokenCount": 2,
                },
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "." }], "role": "model" },
                        "finishReason": "STOP",
                    },
                ],
                "usageMetadata": stream_complete_usage_metadata(),
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let finish_event = stream_finish_event(&stream_events_of(&model, json!({})).await);

        assert_eq!(
            finish_event["usage"],
            json!({
                "inputTokens": {
                    "total": 77,
                    "noCache": 73,
                    "cacheRead": 4,
                },
                "outputTokens": {
                    "total": 160,
                    "text": 71,
                    "reasoning": 89,
                },
                "raw": stream_complete_usage_metadata(),
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should expose null serviceTier in provider metadata on finish when not present
    #[tokio::test]
    async fn should_expose_null_service_tier_in_provider_metadata_on_finish_when_not_present() {
        let mock = sse_stream_mock(sse_text_chunks(&["test"], None, None));
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({})).await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["providerMetadata"]["google"].get("serviceTier"),
            Some(&Value::Null)
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream code execution tool calls and results
    #[tokio::test]
    async fn should_stream_code_execution_tool_calls_and_results() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "executableCode": {
                                        "language": "PYTHON",
                                        "code": "print(\"hello\")",
                                    },
                                },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "codeExecutionResult": {
                                        "outcome": "OUTCOME_OK",
                                        "output": "hello\n",
                                    },
                                },
                            ],
                        },
                        "finishReason": "STOP",
                    },
                ],
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-2.0-pro");

        let events = stream_events_of(
            &model,
            json!({ "tools": [stream_provider_tool("google.code_execution", "code_execution")] }),
        )
        .await;

        let tool_events = stream_events_of_type(&events, &["tool-call", "tool-result"]);

        assert_eq!(
            Value::Array(tool_events),
            json!([
                {
                    "input": "{\"language\":\"PYTHON\",\"code\":\"print(\\\"hello\\\")\"}",
                    "providerExecuted": true,
                    "toolCallId": "test-id",
                    "toolName": "code_execution",
                    "type": "tool-call"
                },
                {
                    "result": {
                        "outcome": "OUTCOME_OK",
                        "output": "hello\n"
                    },
                    "toolCallId": "test-id",
                    "toolName": "code_execution",
                    "type": "tool-result"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream code execution result with missing output field
    #[tokio::test]
    async fn should_stream_code_execution_result_with_missing_output_field() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "executableCode": {
                                        "language": "PYTHON",
                                        "code": "img = PIL.Image.open('input.png')\nimg.save('output.png')\n",
                                    },
                                },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "codeExecutionResult": {
                                        "outcome": "OUTCOME_OK",
                                    },
                                },
                            ],
                        },
                        "finishReason": "STOP",
                    },
                ],
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-2.0-pro");

        let events = stream_events_of(
            &model,
            json!({ "tools": [stream_provider_tool("google.code_execution", "code_execution")] }),
        )
        .await;

        let tool_events = stream_events_of_type(&events, &["tool-call", "tool-result"]);

        assert_eq!(
            Value::Array(tool_events),
            json!([
                {
                    "input": "{\"language\":\"PYTHON\",\"code\":\"img = PIL.Image.open('input.png')\\nimg.save('output.png')\\n\"}",
                    "providerExecuted": true,
                    "toolCallId": "test-id",
                    "toolName": "code_execution",
                    "type": "tool-call"
                },
                {
                    "result": {
                        "outcome": "OUTCOME_OK",
                        "output": ""
                    },
                    "toolCallId": "test-id",
                    "toolName": "code_execution",
                    "type": "tool-result"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should return stop finish reason for streamed code execution (provider-executed tool)
    #[tokio::test]
    async fn should_return_stop_finish_reason_for_streamed_code_execution_provider_executed_tool() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "executableCode": {
                                        "language": "PYTHON",
                                        "code": "print(\"hello\")",
                                    },
                                },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "codeExecutionResult": {
                                        "outcome": "OUTCOME_OK",
                                        "output": "hello\n",
                                    },
                                },
                                {
                                    "text": "{\"result\": \"hello\"}",
                                },
                            ],
                        },
                        "finishReason": "STOP",
                    },
                ],
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-2.0-pro");

        let events = stream_events_of(
            &model,
            json!({ "tools": [stream_provider_tool("google.code_execution", "code_execution")] }),
        )
        .await;

        let finish_event = stream_finish_event(&events);

        // Provider-executed tools should not trigger 'tool-calls' finish reason
        // since they don't require SDK iteration - allows structured output to work
        assert_stream_match_object(
            &finish_event,
            &json!({
                "type": "finish",
                "finishReason": {
                    "raw": "STOP",
                    "unified": "stop",
                },
            }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream server-side toolCall and toolResponse parts (tool combination)
    #[tokio::test]
    async fn should_stream_server_side_tool_call_and_tool_response_parts_tool_combination() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "toolCall": {
                                        "toolType": "GOOGLE_SEARCH_WEB",
                                        "args": { "query": "SF weather" },
                                        "id": "sc-1",
                                    },
                                    "thoughtSignature": "sig-1",
                                },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "toolResponse": {
                                        "toolType": "GOOGLE_SEARCH_WEB",
                                        "response": { "results": [{ "title": "Weather" }] },
                                        "id": "sc-1",
                                    },
                                },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [{ "text": "It is sunny." }],
                        },
                        "finishReason": "STOP",
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 10,
                    "candidatesTokenCount": 20,
                    "totalTokenCount": 30,
                },
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-3-pro-preview");

        let events = stream_events_of(
            &model,
            json!({ "tools": [stream_provider_tool("google.google_search", "google_search")] }),
        )
        .await;

        let tool_events = stream_events_of_type(&events, &["tool-call", "tool-result"]);

        assert_eq!(
            Value::Array(tool_events),
            json!([
                {
                    "dynamic": true,
                    "input": "{\"query\":\"SF weather\"}",
                    "providerExecuted": true,
                    "providerMetadata": {
                        "google": {
                            "serverToolCallId": "sc-1",
                            "serverToolType": "GOOGLE_SEARCH_WEB",
                            "thoughtSignature": "sig-1"
                        }
                    },
                    "toolCallId": "sc-1",
                    "toolName": "server:GOOGLE_SEARCH_WEB",
                    "type": "tool-call"
                },
                {
                    "providerMetadata": {
                        "google": {
                            "serverToolCallId": "sc-1",
                            "serverToolType": "GOOGLE_SEARCH_WEB"
                        }
                    },
                    "result": {
                        "results": [
                            {
                                "title": "Weather"
                            }
                        ]
                    },
                    "toolCallId": "sc-1",
                    "toolName": "server:GOOGLE_SEARCH_WEB",
                    "type": "tool-result"
                }
            ])
        );

        let text_events = stream_events_of_type(&events, &["text-delta"]);
        assert_eq!(
            Value::Array(text_events),
            json!([
                {
                    "delta": "It is sunny.",
                    "id": "0",
                    "type": "text-delta"
                }
            ])
        );

        let finish_event = stream_finish_event(&events);
        assert_stream_match_object(
            &finish_event,
            &json!({
                "type": "finish",
                "finishReason": {
                    "raw": "STOP",
                    "unified": "stop",
                },
            }),
        );
    }

    mod search_tool_selection {
        use super::*;

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › search tool selection › should use googleSearch for gemini-2.0-pro
        #[tokio::test]
        async fn should_use_google_search_for_gemini_2_0_pro() {
            let mock = sse_stream_mock(sse_text_chunks(&[""], None, None));

            let gemini2_pro = stream_test_provider(&mock).chat("gemini-2.0-pro");
            gemini2_pro
                .do_stream(&stream_call_options_with(json!({
                    "includeRawChunks": false,
                    "tools": [stream_provider_tool("google.google_search", "google_search")],
                })))
                .await
                .expect("doStream succeeds");

            assert_stream_match_object(
                &stream_request_body(&mock, 0),
                &json!({ "tools": [{ "googleSearch": {} }] }),
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › search tool selection › should use googleSearch for gemini-2.0-flash-exp
        #[tokio::test]
        async fn should_use_google_search_for_gemini_2_0_flash_exp() {
            let mock = sse_stream_mock(sse_text_chunks(&[""], None, None));

            let gemini2_flash = stream_test_provider(&mock).chat("gemini-2.0-flash-exp");

            gemini2_flash
                .do_stream(&stream_call_options_with(json!({
                    "includeRawChunks": false,
                    "tools": [stream_provider_tool("google.google_search", "google_search")],
                })))
                .await
                .expect("doStream succeeds");

            assert_stream_match_object(
                &stream_request_body(&mock, 0),
                &json!({ "tools": [{ "googleSearch": {} }] }),
            );
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › search tool selection › should warn for google search on non-gemini-2 models
        #[tokio::test]
        async fn should_warn_for_google_search_on_non_gemini_2_models() {
            let mock = sse_stream_mock(sse_text_chunks(&[""], None, None));

            let gemini_pro = stream_test_provider(&mock).chat("gemini-1.0-pro");
            let events = stream_events_of(
                &gemini_pro,
                json!({
                    "includeRawChunks": false,
                    "tools": [stream_provider_tool("google.google_search", "google_search")],
                }),
            )
            .await;

            assert_eq!(
                events[0],
                json!({
                    "type": "stream-start",
                    "warnings": [
                        {
                            "details": "Google Search requires Gemini 2.0 or newer.",
                            "feature": "provider-defined tool google.google_search",
                            "type": "unsupported"
                        }
                    ]
                })
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream source events
    #[tokio::test]
    async fn should_stream_source_events() {
        let mock = sse_stream_mock(sse_text_chunks(
            &["Some initial text"],
            Some(json!({
                "groundingChunks": [
                    {
                        "web": {
                            "uri": "https://source.example.com",
                            "title": "Source Title",
                        },
                    },
                ],
            })),
            None,
        ));
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let source_events = stream_events_of_type(&events, &["source"]);

        assert_eq!(
            Value::Array(source_events),
            json!([
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Source Title",
                    "type": "source",
                    "url": "https://source.example.com"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream source events from image grounding metadata
    #[tokio::test]
    async fn should_stream_source_events_from_image_grounding_metadata() {
        let mock = sse_stream_mock(sse_text_chunks(
            &["image search response"],
            Some(json!({
                "groundingChunks": [
                    {
                        "image": {
                            "sourceUri": "https://example.com/article",
                            "imageUri": "https://example.com/image.jpg",
                            "title": "Image Source",
                            "domain": "example.com",
                        },
                    },
                ],
            })),
            None,
        ));
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let source_events = stream_events_of_type(&events, &["source"]);

        assert_eq!(
            Value::Array(source_events),
            json!([
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Image Source",
                    "type": "source",
                    "url": "https://example.com/article"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream sources during intermediate chunks
    #[tokio::test]
    async fn should_stream_sources_during_intermediate_chunks() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "text" }], "role": "model" },
                        "index": 0,
                        "safetyRatings": stream_safety_ratings(),
                        "groundingMetadata": {
                            "groundingChunks": [
                                { "web": { "uri": "https://a.com", "title": "A" } },
                                { "web": { "uri": "https://b.com", "title": "B" } },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "more" }], "role": "model" },
                        "finishReason": "STOP",
                        "index": 0,
                        "safetyRatings": stream_safety_ratings(),
                    },
                ],
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let source_events = stream_events_of_type(&events, &["source"]);

        assert_eq!(
            Value::Array(source_events),
            json!([
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "A",
                    "type": "source",
                    "url": "https://a.com"
                },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "B",
                    "type": "source",
                    "url": "https://b.com"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should deduplicate sources across chunks
    #[tokio::test]
    async fn should_deduplicate_sources_across_chunks() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "first chunk" }], "role": "model" },
                        "index": 0,
                        "safetyRatings": stream_safety_ratings(),
                        "groundingMetadata": {
                            "groundingChunks": [
                                { "web": { "uri": "https://example.com", "title": "Example" } },
                                { "web": { "uri": "https://unique.com", "title": "Unique" } },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "second chunk" }], "role": "model" },
                        "index": 0,
                        "safetyRatings": stream_safety_ratings(),
                        "groundingMetadata": {
                            "groundingChunks": [
                                {
                                    "web": {
                                        "uri": "https://example.com",
                                        "title": "Example Duplicate",
                                    },
                                },
                                { "web": { "uri": "https://another.com", "title": "Another" } },
                            ],
                        },
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": { "parts": [{ "text": "final chunk" }], "role": "model" },
                        "finishReason": "STOP",
                        "index": 0,
                        "safetyRatings": stream_safety_ratings(),
                    },
                ],
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;
        let source_events = stream_events_of_type(&events, &["source"]);

        assert_eq!(
            Value::Array(source_events),
            json!([
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Example",
                    "type": "source",
                    "url": "https://example.com"
                },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Unique",
                    "type": "source",
                    "url": "https://unique.com"
                },
                {
                    "id": "test-id",
                    "sourceType": "url",
                    "title": "Another",
                    "type": "source",
                    "url": "https://another.com"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream files
    #[tokio::test]
    async fn should_stream_files() {
        let mock = sse_stream_mock(vec![
            concat!(
                r#"data: {"candidates": [{"content": {"parts": [{"inlineData": {"data": "test","mimeType": "text/plain"}}]"#,
                r#","role": "model"},"#,
                r#""finishReason": "STOP","index": 0,"safetyRatings": ["#,
                r#"{"category": "HARM_CATEGORY_SEXUALLY_EXPLICIT","probability": "NEGLIGIBLE"},"#,
                r#"{"category": "HARM_CATEGORY_HATE_SPEECH","probability": "NEGLIGIBLE"},"#,
                r#"{"category": "HARM_CATEGORY_HARASSMENT","probability": "NEGLIGIBLE"},"#,
                r#"{"category": "HARM_CATEGORY_DANGEROUS_CONTENT","probability": "NEGLIGIBLE"}]}]}"#,
                "\n\n"
            )
            .to_owned(),
            concat!(
                r#"data: {"usageMetadata": {"promptTokenCount": 294,"candidatesTokenCount": 233,"totalTokenCount": 527}}"#,
                "\n\n"
            )
            .to_owned(),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

        assert_eq!(
            Value::Array(events),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "data": {
                        "data": "test",
                        "type": "data"
                    },
                    "mediaType": "text/plain",
                    "type": "file"
                },
                {
                    "finishReason": {
                        "raw": "STOP",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "google": {
                            "finishMessage": null,
                            "groundingMetadata": null,
                            "promptFeedback": null,
                            "safetyRatings": [
                                {
                                    "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_HATE_SPEECH",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_HARASSMENT",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                                    "probability": "NEGLIGIBLE"
                                }
                            ],
                            "serviceTier": null,
                            "urlContextMetadata": null,
                            "usageMetadata": {
                                "candidatesTokenCount": 233,
                                "promptTokenCount": 294,
                                "totalTokenCount": 527
                            }
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 294,
                            "total": 294
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 233,
                            "total": 233
                        },
                        "raw": {
                            "candidatesTokenCount": 233,
                            "promptTokenCount": 294,
                            "totalTokenCount": 527
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream file parts with thought flag in providerMetadata
    #[tokio::test]
    async fn should_stream_file_parts_with_thought_flag_in_provider_metadata() {
        let mock = sse_stream_mock(vec![sse_data_chunk(&json!({
            "candidates": [
                {
                    "content": {
                        "parts": [
                            {
                                "inlineData": { "data": "thoughtimg", "mimeType": "image/png" },
                                "thought": true,
                                "thoughtSignature": "stream_sig",
                            },
                            {
                                "inlineData": {
                                    "data": "regularimg",
                                    "mimeType": "image/jpeg",
                                },
                            },
                        ],
                        "role": "model",
                    },
                    "finishReason": "STOP",
                    "index": 0,
                    "safetyRatings": stream_safety_ratings(),
                },
            ],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 20,
                "totalTokenCount": 30,
            },
        }))]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

        let file_events = stream_events_of_type(&events, &["file", "reasoning-file"]);
        assert_eq!(
            Value::Array(file_events),
            json!([
                {
                    "data": {
                        "data": "thoughtimg",
                        "type": "data"
                    },
                    "mediaType": "image/png",
                    "providerMetadata": {
                        "google": {
                            "thoughtSignature": "stream_sig"
                        }
                    },
                    "type": "reasoning-file"
                },
                {
                    "data": {
                        "data": "regularimg",
                        "type": "data"
                    },
                    "mediaType": "image/jpeg",
                    "type": "file"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream text and files in correct order
    #[tokio::test]
    async fn should_stream_text_and_files_in_correct_order() {
        let mock = sse_stream_mock(vec![sse_data_chunk(&json!({
            "candidates": [
                {
                    "content": {
                        "parts": [
                            { "text": "Step 1: " },
                            { "inlineData": { "data": "image1", "mimeType": "image/png" } },
                            { "text": " Step 2: " },
                            { "inlineData": { "data": "image2", "mimeType": "image/jpeg" } },
                            { "text": " Done" },
                        ],
                        "role": "model",
                    },
                    "finishReason": "STOP",
                    "index": 0,
                    "safetyRatings": stream_safety_ratings(),
                },
            ],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 20,
                "totalTokenCount": 30,
            },
        }))]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

        // Filter to content events only (excluding metadata)
        let content_events =
            stream_events_of_type(&events, &["text-start", "text-delta", "text-end", "file"]);

        // Verify that text and file parts are interleaved in the correct order
        assert_eq!(
            Value::Array(content_events),
            json!([
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Step 1: ",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "0",
                    "type": "text-end"
                },
                {
                    "data": {
                        "data": "image1",
                        "type": "data"
                    },
                    "mediaType": "image/png",
                    "type": "file"
                },
                {
                    "id": "1",
                    "type": "text-start"
                },
                {
                    "delta": " Step 2: ",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "id": "1",
                    "type": "text-end"
                },
                {
                    "data": {
                        "data": "image2",
                        "type": "data"
                    },
                    "mediaType": "image/jpeg",
                    "type": "file"
                },
                {
                    "id": "2",
                    "type": "text-start"
                },
                {
                    "delta": " Done",
                    "id": "2",
                    "type": "text-delta"
                },
                {
                    "id": "2",
                    "type": "text-end"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should set finishReason to tool-calls when chunk contains functionCall
    #[tokio::test]
    async fn should_set_finish_reason_to_tool_calls_when_chunk_contains_function_call() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [{ "text": "Initial text response" }],
                            "role": "model",
                        },
                        "index": 0,
                        "safetyRatings": stream_safety_ratings(),
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "functionCall": {
                                        "name": "test-tool",
                                        "args": { "value": "example value" },
                                    },
                                },
                            ],
                            "role": "model",
                        },
                        "finishReason": "STOP",
                        "index": 0,
                        "safetyRatings": stream_safety_ratings(),
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 10,
                    "candidatesTokenCount": 20,
                    "totalTokenCount": 30,
                },
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(
            &model,
            json!({
                "tools": [stream_string_function_tool("test-tool", "value")],
                "includeRawChunks": false,
            }),
        )
        .await;
        let finish_event = stream_finish_event(&events);

        assert_eq!(
            finish_event["finishReason"],
            json!({
                "raw": "STOP",
                "unified": "tool-calls"
            })
        );
    }

    /// One chunk of text `Hello` with `STOP`, the upstream reply of the
    /// request-shape cases.
    fn stream_hello_chunks() -> Vec<String> {
        vec![sse_data_chunk(&json!({
            "candidates": [
                {
                    "content": { "parts": [{ "text": "Hello" }], "role": "model" },
                    "finishReason": "STOP",
                    "safetyRatings": stream_safety_ratings(),
                },
            ],
            "usageMetadata": {
                "promptTokenCount": 1,
                "candidatesTokenCount": 1,
                "totalTokenCount": 2,
            },
        }))]
    }

    /// The same reply as a unary (`json-value`) response.
    fn generate_hello_response() -> Canned {
        Canned::json(&json!({
            "candidates": [
                {
                    "content": { "parts": [{ "text": "Hello" }], "role": "model" },
                    "finishReason": "STOP",
                    "safetyRatings": stream_safety_ratings(),
                },
            ],
            "usageMetadata": {
                "promptTokenCount": 1,
                "candidatesTokenCount": 1,
                "totalTokenCount": 2,
            },
        }))
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should omit server-side tool invocation flag for Vertex Gemini 3
    #[tokio::test]
    async fn should_omit_server_side_tool_invocation_flag_for_vertex_gemini_3() {
        let mock = sse_stream_mock(stream_hello_chunks());
        let vertex_model = vertex_stream_test_model("gemini-3-pro-preview", &mock);

        let result = vertex_model
            .do_stream(&stream_call_options_with(json!({
                "includeRawChunks": false,
                "tools": [
                    stream_string_function_tool("test-tool", "value"),
                    stream_provider_tool("google.google_search", "google_search"),
                ],
            })))
            .await
            .expect("doStream succeeds");

        let request_body = stream_request_body(&mock, 0);
        assert_eq!(
            json!({
                "toolConfig": request_body.get("toolConfig"),
                "tools": request_body.get("tools"),
            }),
            json!({
                "toolConfig": {
                    "functionCallingConfig": {
                        "mode": "VALIDATED"
                    }
                },
                "tools": [
                    {
                        "googleSearch": {}
                    },
                    {
                        "functionDeclarations": [
                            {
                                "description": "",
                                "name": "test-tool",
                                "parametersJsonSchema": {
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                    "additionalProperties": false,
                                    "properties": {
                                        "value": {
                                            "type": "string"
                                        }
                                    },
                                    "required": [
                                        "value"
                                    ],
                                    "type": "object"
                                }
                            }
                        ]
                    }
                ]
            })
        );

        let events = stream_parts_of(result).await;
        assert_eq!(
            events[0],
            json!({
                "type": "stream-start",
                "warnings": []
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should send streamFunctionCallArguments in toolConfig when provider option is set with Vertex provider
    #[tokio::test]
    async fn should_send_stream_function_call_arguments_when_option_is_set_with_vertex_provider() {
        let mock = sse_stream_mock(stream_hello_chunks());
        let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

        vertex_model
            .do_stream(&stream_call_options_with(json!({
                "includeRawChunks": false,
                "tools": [stream_string_function_tool("test-tool", "value")],
                "providerOptions": {
                    "vertex": {
                        "streamFunctionCallArguments": true,
                    },
                },
            })))
            .await
            .expect("doStream succeeds");

        assert_eq!(
            stream_request_body(&mock, 0)["toolConfig"],
            json!({
                "functionCallingConfig": {
                    "streamFunctionCallArguments": true
                }
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should emit warning and not send streamFunctionCallArguments when using non-Vertex provider
    #[tokio::test]
    async fn should_emit_warning_and_not_send_stream_function_call_arguments_for_non_vertex() {
        let mock = sse_stream_mock(stream_hello_chunks());
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let events = stream_events_of(
            &model,
            json!({
                "includeRawChunks": false,
                "tools": [stream_string_function_tool("test-tool", "value")],
                "providerOptions": {
                    "google": {
                        "streamFunctionCallArguments": true,
                    },
                },
            }),
        )
        .await;

        assert_eq!(
            Value::Array(events),
            json!([
                {
                    "type": "stream-start",
                    "warnings": [
                        {
                            "message": "'streamFunctionCallArguments' is only supported on the Vertex AI API and will be ignored with the current Google provider (google.generative-ai). See https://docs.cloud.google.com/vertex-ai/generative-ai/docs/multimodal/function-calling#streaming-fc",
                            "type": "other"
                        }
                    ]
                },
                {
                    "id": "0",
                    "type": "text-start"
                },
                {
                    "delta": "Hello",
                    "id": "0",
                    "type": "text-delta"
                },
                {
                    "id": "0",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "STOP",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "google": {
                            "finishMessage": null,
                            "groundingMetadata": null,
                            "promptFeedback": null,
                            "safetyRatings": [
                                {
                                    "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_HATE_SPEECH",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_HARASSMENT",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                                    "probability": "NEGLIGIBLE"
                                }
                            ],
                            "serviceTier": null,
                            "urlContextMetadata": null,
                            "usageMetadata": {
                                "candidatesTokenCount": 1,
                                "promptTokenCount": 1,
                                "totalTokenCount": 2
                            }
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 1,
                            "total": 1
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 1,
                            "total": 1
                        },
                        "raw": {
                            "candidatesTokenCount": 1,
                            "promptTokenCount": 1,
                            "totalTokenCount": 2
                        }
                    }
                }
            ])
        );

        assert_eq!(
            stream_request_body(&mock, 0)
                .get("toolConfig")
                .and_then(|config| config.get("functionCallingConfig"))
                .and_then(|config| config.get("streamFunctionCallArguments")),
            None
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should default streamFunctionCallArguments to false for Vertex provider without provider option
    #[tokio::test]
    async fn should_default_stream_function_call_arguments_to_false_for_vertex_without_option() {
        let mock = sse_stream_mock(stream_hello_chunks());
        let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

        vertex_model
            .do_stream(&stream_call_options_with(json!({
                "includeRawChunks": false,
                "tools": [stream_string_function_tool("test-tool", "value")],
            })))
            .await
            .expect("doStream succeeds");

        assert_eq!(
            stream_request_body(&mock, 0)
                .get("toolConfig")
                .and_then(|config| config.get("functionCallingConfig"))
                .and_then(|config| config.get("streamFunctionCallArguments")),
            None
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should allow Vertex provider to opt out of streamFunctionCallArguments by setting it to false
    #[tokio::test]
    async fn should_allow_vertex_to_opt_out_of_stream_function_call_arguments_by_setting_false() {
        let mock = sse_stream_mock(stream_hello_chunks());
        let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

        vertex_model
            .do_stream(&stream_call_options_with(json!({
                "includeRawChunks": false,
                "tools": [stream_string_function_tool("test-tool", "value")],
                "providerOptions": {
                    "vertex": {
                        "streamFunctionCallArguments": false,
                    },
                },
            })))
            .await
            .expect("doStream succeeds");

        assert_eq!(
            stream_request_body(&mock, 0)
                .get("toolConfig")
                .and_then(|config| config.get("functionCallingConfig"))
                .and_then(|config| config.get("streamFunctionCallArguments")),
            None
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should not send streamFunctionCallArguments for Vertex provider doGenerate (unary API)
    #[tokio::test]
    async fn should_not_send_stream_function_call_arguments_for_vertex_do_generate_unary_api() {
        let mock = MockFetch::new(vec![generate_hello_response()]);
        let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

        vertex_model
            .do_generate(&stream_call_options_with(json!({
                "tools": [stream_string_function_tool("test-tool", "value")],
            })))
            .await
            .expect("doGenerate succeeds");

        assert_eq!(
            stream_request_body(&mock, 0),
            json!({
                "contents": [
                    {
                        "parts": [
                            {
                                "text": "Hello"
                            }
                        ],
                        "role": "user"
                    }
                ],
                "generationConfig": {},
                "tools": [
                    {
                        "functionDeclarations": [
                            {
                                "description": "",
                                "name": "test-tool",
                                "parametersJsonSchema": {
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                    "additionalProperties": false,
                                    "properties": {
                                        "value": {
                                            "type": "string"
                                        }
                                    },
                                    "required": [
                                        "value"
                                    ],
                                    "type": "object"
                                }
                            }
                        ]
                    }
                ]
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should not send streamFunctionCallArguments for Vertex provider doGenerate even when explicitly set
    #[tokio::test]
    async fn should_not_send_stream_function_call_arguments_for_vertex_do_generate_when_set() {
        let mock = MockFetch::new(vec![generate_hello_response()]);
        let vertex_model = vertex_stream_test_model("gemini-pro", &mock);

        vertex_model
            .do_generate(&stream_call_options_with(json!({
                "providerOptions": {
                    "vertex": {
                        "streamFunctionCallArguments": true,
                    },
                },
            })))
            .await
            .expect("doGenerate succeeds");

        assert_eq!(
            stream_request_body(&mock, 0),
            json!({
                "contents": [
                    {
                        "parts": [
                            {
                                "text": "Hello"
                            }
                        ],
                        "role": "user"
                    }
                ],
                "generationConfig": {}
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should only pass valid provider options
    #[tokio::test]
    async fn should_only_pass_valid_provider_options() {
        let mock = sse_stream_mock(sse_fixture_chunks(include_str!(
            "fixtures/google-text.chunks.txt"
        )));
        let model = stream_test_provider(&mock).chat("gemini-pro");

        model
            .do_stream(&stream_call_options_with(json!({
                "includeRawChunks": false,
                "providerOptions": {
                    "google": { "foo": "bar", "responseModalities": ["TEXT", "IMAGE"] },
                },
            })))
            .await
            .expect("doStream succeeds");

        assert_stream_match_object(
            &stream_request_body(&mock, 0),
            &json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [{ "text": "Hello" }],
                    },
                ],
                "generationConfig": {
                    "responseModalities": ["TEXT", "IMAGE"],
                },
            }),
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream reasoning parts separately from text parts
    #[tokio::test]
    async fn should_stream_reasoning_parts_separately_from_text_parts() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "text": "I need to think about this carefully. The user wants a simple explanation.",
                                    "thought": true,
                                },
                            ],
                            "role": "model",
                        },
                        "index": 0,
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 14,
                    "totalTokenCount": 84,
                    "thoughtsTokenCount": 70,
                },
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "text": "Let me organize my thoughts and provide a clear answer.",
                                    "thought": true,
                                },
                            ],
                            "role": "model",
                        },
                        "index": 0,
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 14,
                    "totalTokenCount": 156,
                    "thoughtsTokenCount": 142,
                },
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "text": "Here is a simple explanation: ",
                                },
                            ],
                            "role": "model",
                        },
                        "index": 0,
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 14,
                    "candidatesTokenCount": 8,
                    "totalTokenCount": 164,
                    "thoughtsTokenCount": 142,
                },
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "text": "The concept works because of basic principles.",
                                },
                            ],
                            "role": "model",
                        },
                        "finishReason": "STOP",
                        "index": 0,
                    },
                ],
                "usageMetadata": {
                    "promptTokenCount": 14,
                    "candidatesTokenCount": 18,
                    "totalTokenCount": 174,
                    "thoughtsTokenCount": 142,
                },
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let all_events = stream_events_of(&model, json!({ "includeRawChunks": false })).await;

        assert_eq!(
            Value::Array(all_events),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "0",
                    "type": "reasoning-start"
                },
                {
                    "delta": "I need to think about this carefully. The user wants a simple explanation.",
                    "id": "0",
                    "type": "reasoning-delta"
                },
                {
                    "delta": "Let me organize my thoughts and provide a clear answer.",
                    "id": "0",
                    "type": "reasoning-delta"
                },
                {
                    "id": "0",
                    "type": "reasoning-end"
                },
                {
                    "id": "1",
                    "type": "text-start"
                },
                {
                    "delta": "Here is a simple explanation: ",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "delta": "The concept works because of basic principles.",
                    "id": "1",
                    "type": "text-delta"
                },
                {
                    "id": "1",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "STOP",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "google": {
                            "finishMessage": null,
                            "groundingMetadata": null,
                            "promptFeedback": null,
                            "safetyRatings": null,
                            "serviceTier": null,
                            "urlContextMetadata": null,
                            "usageMetadata": {
                                "candidatesTokenCount": 18,
                                "promptTokenCount": 14,
                                "thoughtsTokenCount": 142,
                                "totalTokenCount": 174
                            }
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 14,
                            "total": 14
                        },
                        "outputTokens": {
                            "reasoning": 142,
                            "text": 18,
                            "total": 160
                        },
                        "raw": {
                            "candidatesTokenCount": 18,
                            "promptTokenCount": 14,
                            "thoughtsTokenCount": 142,
                            "totalTokenCount": 174
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › should stream thought signatures with reasoning and text parts
    #[tokio::test]
    async fn should_stream_thought_signatures_with_reasoning_and_text_parts() {
        let mock = sse_stream_mock(vec![
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "text": "I need to think about this.",
                                    "thought": true,
                                    "thoughtSignature": "reasoning_sig1",
                                },
                            ],
                            "role": "model",
                        },
                        "index": 0,
                    },
                ],
            })),
            sse_data_chunk(&json!({
                "candidates": [
                    {
                        "content": {
                            "parts": [
                                {
                                    "text": "Here is the answer.",
                                    "thoughtSignature": "text_sig1",
                                },
                            ],
                            "role": "model",
                        },
                        "index": 0,
                        "finishReason": "STOP",
                        "safetyRatings": stream_safety_ratings(),
                    },
                ],
            })),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let chunks = stream_events_of(&model, json!({})).await;

        assert_eq!(
            Value::Array(chunks),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "0",
                    "providerMetadata": {
                        "google": {
                            "thoughtSignature": "reasoning_sig1"
                        }
                    },
                    "type": "reasoning-start"
                },
                {
                    "delta": "I need to think about this.",
                    "id": "0",
                    "providerMetadata": {
                        "google": {
                            "thoughtSignature": "reasoning_sig1"
                        }
                    },
                    "type": "reasoning-delta"
                },
                {
                    "id": "0",
                    "type": "reasoning-end"
                },
                {
                    "id": "1",
                    "providerMetadata": {
                        "google": {
                            "thoughtSignature": "text_sig1"
                        }
                    },
                    "type": "text-start"
                },
                {
                    "delta": "Here is the answer.",
                    "id": "1",
                    "providerMetadata": {
                        "google": {
                            "thoughtSignature": "text_sig1"
                        }
                    },
                    "type": "text-delta"
                },
                {
                    "id": "1",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "STOP",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "google": {
                            "finishMessage": null,
                            "groundingMetadata": null,
                            "promptFeedback": null,
                            "safetyRatings": [
                                {
                                    "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_HATE_SPEECH",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_HARASSMENT",
                                    "probability": "NEGLIGIBLE"
                                },
                                {
                                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                                    "probability": "NEGLIGIBLE"
                                }
                            ],
                            "serviceTier": null,
                            "urlContextMetadata": null,
                            "usageMetadata": null
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    mod provider_metadata_key_based_on_provider_string {
        use super::*;

        /// The two chunks `Hello` / ` World!` with grounding metadata on the
        /// last one.
        fn hello_world_grounding_chunks() -> Vec<String> {
            vec![
                sse_data_chunk(&json!({
                    "candidates": [
                        {
                            "content": { "parts": [{ "text": "Hello" }], "role": "model" },
                        },
                    ],
                })),
                sse_data_chunk(&json!({
                    "candidates": [
                        {
                            "content": { "parts": [{ "text": " World!" }], "role": "model" },
                            "finishReason": "STOP",
                            "safetyRatings": stream_safety_ratings(),
                            "groundingMetadata": {
                                "webSearchQueries": ["test query"],
                            },
                        },
                    ],
                    "usageMetadata": {
                        "promptTokenCount": 1,
                        "candidatesTokenCount": 2,
                        "totalTokenCount": 3,
                    },
                })),
            ]
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › providerMetadata key based on provider string › should use "vertex" as providerMetadata key in finish event when provider includes "vertex"
        #[tokio::test]
        async fn should_use_vertex_as_provider_metadata_key_in_finish_event_when_provider_is_vertex()
         {
            let mock = sse_stream_mock(hello_world_grounding_chunks());
            let model = stream_test_model_for_provider("gemini-pro", "google.vertex.chat", &mock);

            let events = stream_events_of(&model, json!({})).await;
            let finish_event = stream_finish_event(&events);

            assert_eq!(finish_event["type"], "finish");
            let provider_metadata = &finish_event["providerMetadata"];
            assert!(provider_metadata.get("vertex").is_some());
            assert_stream_match_object(
                &provider_metadata["vertex"],
                &json!({ "promptFeedback": null }),
            );
            assert!(provider_metadata["vertex"]["groundingMetadata"].is_object());
            assert!(provider_metadata["vertex"]["safetyRatings"].is_array());
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › providerMetadata key based on provider string › should use "google" as providerMetadata key in finish event when provider does not include "vertex"
        #[tokio::test]
        async fn should_use_google_as_provider_metadata_key_in_finish_event_when_not_vertex() {
            let mock = sse_stream_mock(hello_world_grounding_chunks());
            let model = stream_test_model_for_provider("gemini-pro", "google.generative-ai", &mock);

            let events = stream_events_of(&model, json!({})).await;
            let finish_event = stream_finish_event(&events);

            assert_eq!(finish_event["type"], "finish");
            let provider_metadata = &finish_event["providerMetadata"];
            assert!(provider_metadata.get("google").is_some());
            assert!(provider_metadata.get("vertex").is_none());
            assert_stream_match_object(
                &provider_metadata["google"],
                &json!({ "promptFeedback": null }),
            );
            assert!(provider_metadata["google"]["groundingMetadata"].is_object());
            assert!(provider_metadata["google"]["safetyRatings"].is_array());
        }

        /// upstream: @ai-sdk/google src/google-language-model.test.ts › doStream › providerMetadata key based on provider string › should use "vertex" as providerMetadata key in streaming reasoning events when provider includes "vertex"
        #[tokio::test]
        async fn should_use_vertex_as_provider_metadata_key_in_streaming_reasoning_events() {
            let mock = sse_stream_mock(vec![
                sse_data_chunk(&json!({
                    "candidates": [
                        {
                            "content": {
                                "parts": [
                                    {
                                        "text": "thinking...",
                                        "thought": true,
                                        "thoughtSignature": "stream_sig",
                                    },
                                ],
                                "role": "model",
                            },
                        },
                    ],
                })),
                sse_data_chunk(&json!({
                    "candidates": [
                        {
                            "content": { "parts": [{ "text": "Final answer" }], "role": "model" },
                            "finishReason": "STOP",
                            "safetyRatings": stream_safety_ratings(),
                        },
                    ],
                    "usageMetadata": {
                        "promptTokenCount": 1,
                        "candidatesTokenCount": 2,
                        "totalTokenCount": 3,
                    },
                })),
            ]);
            let model = stream_test_model_for_provider("gemini-pro", "google.vertex.chat", &mock);

            let events = stream_events_of(&model, json!({})).await;

            let reasoning_start_event = stream_events_of_type(&events, &["reasoning-start"])
                .into_iter()
                .next()
                .expect("a reasoning-start event");
            let provider_metadata = &reasoning_start_event["providerMetadata"];
            assert!(provider_metadata.get("vertex").is_some());
            assert!(provider_metadata.get("google").is_none());
            assert_stream_match_object(
                &provider_metadata["vertex"],
                &json!({ "thoughtSignature": "stream_sig" }),
            );

            let reasoning_delta_event = stream_events_of_type(&events, &["reasoning-delta"])
                .into_iter()
                .next()
                .expect("a reasoning-delta event");
            let provider_metadata = &reasoning_delta_event["providerMetadata"];
            assert!(provider_metadata.get("vertex").is_some());
            assert!(provider_metadata.get("google").is_none());
        }
    }
}

mod gemma_model_system_instruction_fix {
    use super::*;

    /// `TEST_PROMPT_WITH_SYSTEM`.
    fn prompt_with_system() -> Value {
        json!([
            { "role": "system", "content": "You are a helpful assistant." },
            { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
        ])
    }

    /// The `Hello!` reply of every GEMMA case.
    fn gemma_hello_response() -> Canned {
        Canned::json(&json!({
            "candidates": [
                {
                    "content": { "parts": [{ "text": "Hello!" }], "role": "model" },
                    "finishReason": "STOP",
                    "index": 0,
                },
            ],
        }))
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › GEMMA Model System Instruction Fix › should NOT send systemInstruction for GEMMA-3-12b-it model
    #[tokio::test]
    async fn should_not_send_system_instruction_for_gemma_3_12b_it_model() {
        let mock = MockFetch::new(vec![gemma_hello_response()]);
        let model = stream_test_model_for_provider("gemma-3-12b-it", "google.generative-ai", &mock);

        model
            .do_generate(&stream_call_options(
                json!({ "prompt": prompt_with_system() }),
            ))
            .await
            .expect("doGenerate succeeds");

        // Verify that systemInstruction was NOT sent for GEMMA model
        let request_body = mock.seen().last().unwrap().json_body();

        assert!(request_body.get("systemInstruction").is_none());
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › GEMMA Model System Instruction Fix › should NOT send systemInstruction for GEMMA-3-27b-it model
    #[tokio::test]
    async fn should_not_send_system_instruction_for_gemma_3_27b_it_model() {
        let mock = MockFetch::new(vec![gemma_hello_response()]);
        let model = stream_test_model_for_provider("gemma-3-27b-it", "google.generative-ai", &mock);

        model
            .do_generate(&stream_call_options(
                json!({ "prompt": prompt_with_system() }),
            ))
            .await
            .expect("doGenerate succeeds");

        let request_body = mock.seen().last().unwrap().json_body();

        assert!(request_body.get("systemInstruction").is_none());
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › GEMMA Model System Instruction Fix › should still send systemInstruction for Gemini models (regression test)
    #[tokio::test]
    async fn should_still_send_system_instruction_for_gemini_models_regression_test() {
        let mock = MockFetch::new(vec![gemma_hello_response()]);
        let model = stream_test_model_for_provider("gemini-pro", "google.generative-ai", &mock);

        model
            .do_generate(&stream_call_options(
                json!({ "prompt": prompt_with_system() }),
            ))
            .await
            .expect("doGenerate succeeds");

        let request_body = mock.seen().last().unwrap().json_body();

        assert!(request_body.get("systemInstruction").is_some());
        assert_eq!(
            request_body["systemInstruction"],
            json!({ "parts": [{ "text": "You are a helpful assistant." }] })
        );
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › GEMMA Model System Instruction Fix › should NOT generate warning when GEMMA model is used without system instructions
    #[tokio::test]
    async fn should_not_generate_warning_when_gemma_model_is_used_without_system_instructions() {
        let mock = MockFetch::new(vec![gemma_hello_response()]);
        let model = stream_test_model_for_provider("gemma-3-12b-it", "google.generative-ai", &mock);

        let prompt_without_system = json!([
            { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
        ]);

        let result = model
            .do_generate(&stream_call_options(
                json!({ "prompt": prompt_without_system }),
            ))
            .await
            .expect("doGenerate succeeds");

        assert_eq!(result.warnings.len(), 0);
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › GEMMA Model System Instruction Fix › should NOT generate warning when Gemini model is used with system instructions
    #[tokio::test]
    async fn should_not_generate_warning_when_gemini_model_is_used_with_system_instructions() {
        let mock = MockFetch::new(vec![gemma_hello_response()]);
        let model = stream_test_model_for_provider("gemini-pro", "google.generative-ai", &mock);

        let result = model
            .do_generate(&stream_call_options(
                json!({ "prompt": prompt_with_system() }),
            ))
            .await
            .expect("doGenerate succeeds");

        assert_eq!(result.warnings.len(), 0);
    }

    /// upstream: @ai-sdk/google src/google-language-model.test.ts › GEMMA Model System Instruction Fix › should prepend system instruction to first user message for GEMMA models
    #[tokio::test]
    async fn should_prepend_system_instruction_to_first_user_message_for_gemma_models() {
        let mock = MockFetch::new(vec![gemma_hello_response()]);
        let model = stream_test_model_for_provider("gemma-3-12b-it", "google.generative-ai", &mock);

        model
            .do_generate(&stream_call_options(
                json!({ "prompt": prompt_with_system() }),
            ))
            .await
            .expect("doGenerate succeeds");

        let request_body = mock.seen().last().unwrap().json_body();

        assert_eq!(
            request_body,
            json!({
                "contents": [
                    {
                        "parts": [
                            {
                                "text": "You are a helpful assistant.\n\n"
                            },
                            {
                                "text": "Hello"
                            }
                        ],
                        "role": "user"
                    }
                ],
                "generationConfig": {}
            })
        );
    }
}

// ── Older ports (previously `tests/google_upstream_e2e_test.rs`) ──
//
// Small public-factory end-to-end subset of the pinned Google upstream tests.

mod older_upstream_e2e {
    use crate::google::{GoogleProviderSettings, create_google};
    use crate::test_support::mock_fetch::{Canned, MockFetch};
    use aimux_core::AiMuxError;
    use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
    use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::LanguageModelMessage;
    use aimux_core::options::CallOptions;
    use aimux_core::result::GenerateContent;
    use aimux_core::stream_part::StreamPart;
    use aimux_core::tool::{FunctionTool, Tool};
    use aimux_core::types::FinishReasonUnified;
    use aimux_core::video_model::{VideoCallOptions, VideoData, VideoModel, VideoOperationStatus};
    use futures::StreamExt;
    use serde_json::{Value, json};
    use std::sync::Arc;

    const BASE: &str = "https://api.example.com/v1beta";

    fn settings(mock: &Arc<MockFetch>) -> GoogleProviderSettings {
        GoogleProviderSettings {
            base_url: Some(BASE.into()),
            api_key: Some("test-api-key".into()),
            fetch: Some(mock.transport()),
            generate_id: Some(Arc::new(|| "test-id".into())),
            ..Default::default()
        }
    }

    fn options() -> CallOptions {
        CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
    }

    fn basic_request() -> Value {
        json!({"contents":[{"role":"user","parts":[{"text":"Hello"}]}],"generationConfig":{}})
    }

    /// partial: `should preserve complete raw usage metadata`; full port in src/google/tests/language_model.rs.
    #[tokio::test]
    async fn generate_preserves_usage_and_metadata() {
        let usage: Value = serde_json::from_str(r##"{"promptTokenCount": 12, "cachedContentTokenCount": 4, "candidatesTokenCount": 71, "toolUsePromptTokenCount": 65, "thoughtsTokenCount": 89, "totalTokenCount": 237, "promptTokensDetails": [{"modality": "TEXT", "tokenCount": 12, "nestedSentinel": "prompt"}], "cacheTokensDetails": [{"modality": "TEXT", "tokenCount": 4, "nestedSentinel": "cache"}], "candidatesTokensDetails": [{"modality": "TEXT", "tokenCount": 71, "nestedSentinel": "candidate"}], "toolUsePromptTokensDetails": [{"modality": "TEXT", "tokenCount": 65, "nestedSentinel": "tool"}], "serviceTier": "standard", "topLevelSentinel": "preserve-me"}"##).unwrap();
        let body = json!({"candidates":[{"content":{"parts":[{"text":"Blue."}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":usage});
        let mock = MockFetch::new(vec![Canned::json(&body)]);
        let result = create_google(settings(&mock))
            .unwrap()
            .chat("gemini-pro")
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(mock.seen().len(), 1);
        let sent = &mock.seen()[0];
        assert_eq!(sent.method, "POST");
        assert_eq!(
            sent.url,
            format!("{BASE}/models/gemini-pro:generateContent")
        );
        assert_eq!(sent.headers["x-goog-api-key"], "test-api-key");
        assert_eq!(sent.json_body(), basic_request());
        assert_eq!(result.request.as_ref().unwrap().body, Some(basic_request()));
        assert_eq!(result.response.as_ref().unwrap().body, Some(body));
        assert!(
            matches!(&result.content[..], [GenerateContent::Text { text, .. }] if text == "Blue.")
        );
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(result.usage.input_tokens.total, Some(77));
        assert_eq!(result.usage.input_tokens.no_cache, Some(73));
        assert_eq!(result.usage.input_tokens.cache_read, Some(4));
        assert_eq!(result.usage.output_tokens.total, Some(160));
        assert_eq!(result.usage.output_tokens.text, Some(71));
        assert_eq!(result.usage.output_tokens.reasoning, Some(89));
        assert_eq!(result.usage.raw, usage.as_object().cloned());
        let metadata = result.provider_metadata.unwrap();
        assert_eq!(metadata["google"]["usageMetadata"], usage);
        assert_eq!(metadata["google"]["serviceTier"], "standard");
    }

    /// partial: `should stream text deltas`; full port in src/google/tests/language_model.rs.
    #[tokio::test]
    async fn stream_text_and_finish_usage() {
        let chunks = r##"{"candidates":[{"content":{"parts":[{"text":"There are **3**"}],"role":"model"},"index":0}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":5,"totalTokenCount":199,"promptTokensDetails":[{"modality":"TEXT","tokenCount":9}],"thoughtsTokenCount":185},"modelVersion":"gemini-3-pro-preview","responseId":"bH6LaZW8Fp_3nsEPqtaSwQ4"}
{"candidates":[{"content":{"parts":[{"text":" \"r\"s in strawberry.\n\nst**r**awbe**rr**y"}],"role":"model"},"index":0}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":23,"totalTokenCount":217,"promptTokensDetails":[{"modality":"TEXT","tokenCount":9}],"thoughtsTokenCount":185},"modelVersion":"gemini-3-pro-preview","responseId":"bH6LaZW8Fp_3nsEPqtaSwQ4"}
{"candidates":[{"content":{"parts":[{"text":"","thoughtSignature":"EqsFCqgFAb4+9vvtAF5n87lB4OGDOoTRMOqp35jW65XsYXh6BySMwl9nvrbAvPcl2U0xITaYUyV4CmREEDB1z0ZPpCg7iEwiZcj40Eh1jXoL8Y/BbPqxdgZKvKxdBsJx92y2ML5ytajQHVFQb9ohEMMnjs9uNadLAhDEsOU1nC5tl3FQkx94uaGfWvg61bJT3Y9OxFdo/kbpm4RBngvYhVkBzHKkHBj72T2bUd8J4HPssi7ORC5iPosPRIOyH/CAVHEtMzFYMwb7OhRu+CW8Z9u7gDieME5iJjXtJtLrNGDxgR7XtWfRRyGjsj6uDS+KvjR3SUSWPdn5eeH6w+LXZm1X///Hvhhcx+NHxsuGjF3fGhyzTVAoIzk0lxyB4+/A9I4Xa0o/T4coVDiewMzGZDwmket//ig8x9UC8cyWr/hy1joZWUO7ooJlLncv8gy4Ng+y1JdievZokSFDNWfMMNAQr3kgUwJDucqDp44C1xMtgR3lhJ75IBBnprHCE/ThgvNXujmqNkwAjp5dS4PjVbrw8fqSylfE80tvU0g9dXqg4pEyG+hGIxbANLhsWjAKLqh69hyqvVLg2Ds3wppphf61IfC4VoeLWj85CjBZMf+k85NsUIJQ6+DQS9IPNbM29ZOzpUbHoWKJB6VzNCSJse7Pi07L+pd6skl77km00y4lJdHIGHfEgi8PaOonakBcxbRqKzGJAA/urlP0tiWya2fTWrvNZOybJHyyofNNSI4s5y76yKEjP1wnPqC7ujrQk6xb7eyCeqH9ekByy3vv0JfgERFptoSUoG2toIr9M3lS/LKpnwfCvZh+z3J0iMb83d4MaPKhGhE49J4660XUsEmjygAZNi9HnjfC3KtaU/07Sx4JCezMtpsLKUxBgy4xaNqwew3FwAG37eeWcow="}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":23,"totalTokenCount":217,"promptTokensDetails":[{"modality":"TEXT","tokenCount":9}],"thoughtsTokenCount":185},"modelVersion":"gemini-3-pro-preview","responseId":"bH6LaZW8Fp_3nsEPqtaSwQ4"}"##;
        let mock = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: chunks
                .lines()
                .map(|line| format!("data: {line}\n\n"))
                .collect::<String>()
                .into_bytes(),
        }]);
        let result = create_google(settings(&mock))
            .unwrap()
            .chat("gemini-pro")
            .do_stream(&options())
            .await
            .unwrap();
        assert_eq!(result.request.as_ref().unwrap().body, Some(basic_request()));
        assert_eq!(
            result.response.as_ref().unwrap().headers.as_ref().unwrap()["content-type"],
            "text/event-stream"
        );
        let events: Vec<_> = result
            .stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(mock.seen().len(), 1);
        let sent = &mock.seen()[0];
        assert_eq!(
            sent.url,
            format!("{BASE}/models/gemini-pro:streamGenerateContent?alt=sse")
        );
        assert_eq!(sent.json_body(), basic_request());
        let deltas: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            deltas,
            vec![
                "There are **3**",
                " \"r\"s in strawberry.\n\nst**r**awbe**rr**y",
                ""
            ]
        );
        let last_chunk: Value = serde_json::from_str(chunks.lines().last().unwrap()).unwrap();
        assert!(events.iter().any(|event| matches!(event,
        StreamPart::TextDelta { delta, provider_metadata: Some(metadata), .. }
        if delta.is_empty() && metadata["google"]["thoughtSignature"] == last_chunk["candidates"][0]["content"]["parts"][0]["thoughtSignature"]
    )));
        let (finish, usage, metadata) = events
            .iter()
            .find_map(|event| match event {
                StreamPart::Finish {
                    finish_reason,
                    usage,
                    provider_metadata,
                } => Some((finish_reason, usage, provider_metadata.as_ref().unwrap())),
                _ => None,
            })
            .unwrap();
        assert_eq!(finish.unified, FinishReasonUnified::Stop);
        assert_eq!(usage.input_tokens.total, Some(9));
        assert_eq!(usage.output_tokens.total, Some(208));
        assert_eq!(usage.output_tokens.text, Some(23));
        assert_eq!(usage.output_tokens.reasoning, Some(185));
        assert_eq!(metadata["google"]["usageMetadata"]["totalTokenCount"], 217);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamPart::TextStart { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamPart::TextEnd { .. }))
                .count(),
            1
        );
    }

    /// partial: `should extract tool calls`; full port in src/google/tests/language_model.rs.
    #[tokio::test]
    async fn generate_function_call_with_signature() {
        let body: Value = serde_json::from_str(r##"{
  "candidates": [
    {
      "content": {
        "parts": [
          {
            "functionCall": {
              "name": "weather",
              "args": {
                "location": "San Francisco"
              }
            },
            "thoughtSignature": "EskgCsYgAb4+9vtF7/499YQS2bjZs3xcQI+iAl+ILn29nK1j0Kg6su7QsUUUk3nrAAfnS2w5WiVvlcCqu9fAebJ2cvfaEyBahEt5"
          }
        ],
        "role": "model"
      },
      "finishReason": "STOP",
      "index": 0,
      "finishMessage": "Model generated function call(s)."
    }
  ],
  "usageMetadata": {
    "promptTokenCount": 29,
    "candidatesTokenCount": 15,
    "totalTokenCount": 937,
    "promptTokensDetails": [
      {
        "modality": "TEXT",
        "tokenCount": 29
      }
    ],
    "thoughtsTokenCount": 893
  },
  "modelVersion": "gemini-3-pro-preview",
  "responseId": "m36LaZGyCLz1xs0PtNSB-QU"
}
"##).unwrap();
        let mock = MockFetch::new(vec![Canned::json(&body)]);
        let schema = json!({"type":"object","properties":{"location":{"type":"string"}},"required":["location"],"additionalProperties":false,"$schema":"http://json-schema.org/draft-07/schema#"});
        let mut opts = options();
        opts.tools = Some(vec![Tool::Function(FunctionTool::new(
            "weather",
            schema.clone(),
        ))]);
        let result = create_google(settings(&mock))
            .unwrap()
            .chat("gemini-pro")
            .do_generate(&opts)
            .await
            .unwrap();
        let mut expected = basic_request();
        expected["tools"] = json!([{"functionDeclarations":[{"name":"weather","description":"","parametersJsonSchema":schema}]}]);
        assert_eq!(mock.seen()[0].json_body(), expected);
        let [GenerateContent::ToolCall(call)] = &result.content[..] else {
            panic!("expected one tool call")
        };
        assert_eq!(call.tool_call_id, "test-id");
        assert_eq!(call.tool_name, "weather");
        assert_eq!(call.input, r#"{"location":"San Francisco"}"#);
        assert_eq!(
            call.provider_metadata.as_ref().unwrap()["google"]["thoughtSignature"],
            body["candidates"][0]["content"]["parts"][0]["thoughtSignature"]
        );
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(result.usage.input_tokens.total, Some(29));
        assert_eq!(result.usage.output_tokens.total, Some(908));
    }

    /// partial: `preserves google.rpc.RetryInfo in APICallError.data`; full port in src/google/tests/error.rs.
    #[tokio::test]
    async fn error_preserves_retry_info() {
        let body = json!({"error":{"code":429,"message":"You exceeded your current quota, please check your plan.","status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaId":"GenerateRequestsPerMinutePerProjectPerModel-FreeTier"}]},{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"34.4s"}]}});
        let mut canned = Canned::json(&body);
        canned.status = 429;
        let mock = MockFetch::new(vec![canned]);
        let error = create_google(settings(&mock))
            .unwrap()
            .chat("gemini-2.5-flash")
            .do_generate(&options())
            .await
            .unwrap_err();
        assert_eq!(mock.seen().len(), 1);
        assert_eq!(mock.seen()[0].json_body(), basic_request());
        let AiMuxError::ApiCall(error) = error else {
            panic!("expected API call error")
        };
        assert_eq!(error.status_code, Some(429));
        assert_eq!(
            error.message,
            "You exceeded your current quota, please check your plan."
        );
        assert_eq!(error.data, Some(body));
    }

    /// partial: `should extract embedding`; full port in src/google/tests/embedding_model.rs.
    /// partial: `should pass the model and the values`; full port in src/google/tests/embedding_model.rs.
    #[tokio::test]
    async fn embed_batch_request_and_vectors() {
        let body = json!({"embeddings":[{"values":[0.1,0.2,0.3,0.4,0.5]},{"values":[0.6,0.7,0.8,0.9,1.0]}]});
        let mock = MockFetch::new(vec![Canned::json(&body)]);
        let mut opts = EmbeddingCallOptions::new("sunny day at the beach");
        opts.values.push("rainy day in the city".into());
        let result = create_google(settings(&mock))
            .unwrap()
            .embedding("gemini-embedding-001")
            .do_embed(&opts)
            .await
            .unwrap();
        let sent = &mock.seen()[0];
        assert_eq!(
            sent.url,
            format!("{BASE}/models/gemini-embedding-001:batchEmbedContents")
        );
        assert_eq!(
            sent.json_body(),
            json!({"requests":[{"model":"models/gemini-embedding-001","content":{"role":"user","parts":[{"text":"sunny day at the beach"}]}},{"model":"models/gemini-embedding-001","content":{"role":"user","parts":[{"text":"rainy day in the city"}]}}]})
        );
        assert_eq!(
            result.embeddings,
            vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
        );
        assert!(result.usage.is_none());
        assert_eq!(result.response.unwrap().body, Some(body));
    }

    /// partial: `should use the language model endpoint and extract generated images`; full port in src/google/tests/image_model.rs.
    #[tokio::test]
    async fn image_request_and_metadata() {
        let usage = json!({"promptTokenCount":10,"candidatesTokenCount":100,"totalTokenCount":110});
        let body = json!({"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":"base64-generated-image"}}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":usage});
        let mock = MockFetch::new(vec![Canned::json(&body)]);
        let result = create_google(settings(&mock))
            .unwrap()
            .image("gemini-2.5-flash-image")
            .do_generate(&ImageCallOptions::new("A beautiful sunset"))
            .await
            .unwrap();
        let sent = &mock.seen()[0];
        assert_eq!(
            sent.url,
            format!("{BASE}/models/gemini-2.5-flash-image:generateContent")
        );
        assert_eq!(
            sent.json_body(),
            json!({"contents":[{"role":"user","parts":[{"text":"A beautiful sunset"}]}],"generationConfig":{"responseModalities":["IMAGE"]}})
        );
        let ImageOutputs::Base64(images) = result.images else {
            panic!("expected base64 images")
        };
        assert_eq!(images, vec!["base64-generated-image"]);
        assert_eq!(
            serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
            json!({"google":{"finishMessage":null,"finishReason":"STOP","groundingMetadata":null,"images":[{}],"promptFeedback":null,"safetyRatings":null,"serviceTier":null,"urlContextMetadata":null,"usageMetadata":usage}})
        );
        let usage = result.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(100));
        assert_eq!(usage.total_tokens, Some(110));
    }

    /// partial: `should return operation with operationName`; full port in src/google/tests/video_model.rs.
    /// partial: `should pass correct request body`; full port in src/google/tests/video_model.rs.
    /// partial: `should return completed with video data when done`; full port in src/google/tests/video_model.rs.
    #[tokio::test]
    async fn video_start_and_completed_status() {
        let mock = MockFetch::new(vec![
            Canned::json(&json!({"name":"operations/start-test-op","done":false})),
            Canned::json(
                &json!({"done":true,"response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":"https://api.example.com/files/video-456.mp4"}}]}}}),
            ),
        ]);
        let model = create_google(settings(&mock))
            .unwrap()
            .video("veo-3.1-generate-preview");
        let opts = VideoCallOptions::new("A futuristic city with flying cars");
        let start = model.do_start(&opts).await.unwrap();
        assert_eq!(
            start.operation,
            json!({"operationName":"operations/start-test-op"})
        );
        let sent = &mock.seen()[0];
        assert_eq!(sent.method, "POST");
        assert_eq!(
            sent.url,
            format!("{BASE}/models/veo-3.1-generate-preview:predictLongRunning")
        );
        assert_eq!(
            sent.json_body(),
            json!({"instances":[{"prompt":"A futuristic city with flying cars"}],"parameters":{"sampleCount":1}})
        );
        let status = model.do_status(&start.operation, &opts).await.unwrap();
        assert_eq!(mock.seen().len(), 2);
        assert_eq!(mock.seen()[1].method, "GET");
        assert_eq!(
            mock.seen()[1].url,
            format!("{BASE}/operations/start-test-op")
        );
        let VideoOperationStatus::Completed(result) = status else {
            panic!("expected completed video")
        };
        assert_eq!(result.videos.len(), 1);
        assert!(
            matches!(&result.videos[0], VideoData::Url { url, media_type } if url == "https://api.example.com/files/video-456.mp4?key=test-api-key" && media_type == "video/mp4")
        );
    }

    /// partial: `should throw when no videos in response`; full port in src/google/tests/video_model.rs.
    #[tokio::test]
    async fn video_completed_response_without_videos_errors() {
        let mock = MockFetch::new(vec![Canned::json(
            &json!({"done":true,"response":{"generateVideoResponse":{"generatedSamples":[]}}}),
        )]);
        let error = create_google(settings(&mock))
            .unwrap()
            .video("veo-3.1-generate-preview")
            .do_status(
                &json!({"operationName":"operations/status-test-op"}),
                &VideoCallOptions::new("A futuristic city with flying cars"),
            )
            .await
            .unwrap_err();
        assert_eq!(mock.seen()[0].method, "GET");
        assert_eq!(
            mock.seen()[0].url,
            format!("{BASE}/operations/status-test-op")
        );
        assert!(error.to_string().contains("No videos in response"));
    }

    const THOUGHT_SIGNATURE: &str = "EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH";

    /// user → assistant tool call (with `signature` in providerOptions.google)
    /// → tool result: the second turn that issue #9 saw rejected with HTTP 400.
    fn second_turn(signature: Option<&str>) -> Vec<LanguageModelMessage> {
        let mut call = json!({
            "type": "tool-call",
            "toolCallId": "call-1",
            "toolName": "weather",
            "input": {"location": "San Francisco"}
        });
        if let Some(signature) = signature {
            call["providerOptions"] = json!({"google": {"thoughtSignature": signature}});
        }
        serde_json::from_value(json!([
            {"role": "user", "content": [{"type": "text", "text": "weather in SF?"}]},
            {"role": "assistant", "content": [call]},
            {"role": "tool", "content": [{
                "type": "tool-result",
                "toolCallId": "call-1",
                "toolName": "weather",
                "output": {"type": "json", "value": {"temp": 70}}
            }]}
        ]))
        .unwrap()
    }

    /// The `parts` of the model turn that `model_id` was sent for `prompt`.
    async fn replayed_model_parts(model_id: &str, prompt: Vec<LanguageModelMessage>) -> Value {
        let body = json!({"candidates":[{"content":{"parts":[{"text":"Done."}],"role":"model"},"finishReason":"STOP"}]});
        let mock = MockFetch::new(vec![Canned::json(&body)]);
        create_google(settings(&mock))
            .unwrap()
            .chat(model_id)
            .do_generate(&CallOptions::new(prompt))
            .await
            .unwrap();
        let sent = mock.seen()[0].json_body();
        assert_eq!(sent["contents"][1]["role"], "model", "{sent}");
        sent["contents"][1]["parts"].clone()
    }

    /// convert-to-google-messages.ts:404-411: a tool call's
    /// providerOptions.google.thoughtSignature goes back as a sibling of
    /// `functionCall` in the same part, not inside it (issue #9).
    #[tokio::test]
    async fn replays_a_thought_signature_beside_the_function_call() {
        let parts =
            replayed_model_parts("gemini-2.5-pro", second_turn(Some(THOUGHT_SIGNATURE))).await;
        assert_eq!(
            parts,
            json!([{
                "functionCall": {"id": "call-1", "name": "weather", "args": {"location": "San Francisco"}},
                "thoughtSignature": THOUGHT_SIGNATURE
            }])
        );
    }

    /// An unsigned call on a model before Gemini 3 is sent without the field.
    #[tokio::test]
    async fn an_unsigned_call_has_no_thought_signature_before_gemini_3() {
        let parts = replayed_model_parts("gemini-2.5-pro", second_turn(None)).await;
        assert!(parts[0].get("functionCall").is_some(), "{parts}");
        assert!(parts[0].get("thoughtSignature").is_none(), "{parts}");
    }

    /// convert-to-google-messages.ts:540-552: on Gemini 3 an unsigned call gets
    /// the documented skip sentinel, while a signed call keeps its own signature.
    #[tokio::test]
    async fn gemini_3_fills_only_a_missing_thought_signature() {
        let parts = replayed_model_parts("gemini-3-pro-preview", second_turn(None)).await;
        assert_eq!(
            parts[0]["thoughtSignature"],
            "skip_thought_signature_validator"
        );

        let parts =
            replayed_model_parts("gemini-3-pro-preview", second_turn(Some(THOUGHT_SIGNATURE)))
                .await;
        assert_eq!(parts[0]["thoughtSignature"], THOUGHT_SIGNATURE);
    }

    /// Upstream: `candidates` is `.nullish()` (@ai-sdk/google@4.0.87 src/google-language-model.ts:1744),
    /// so a blocked prompt answered with `"candidates": null` finishes as a content filter.
    #[tokio::test]
    async fn generate_reads_null_candidates_as_none() {
        let body = json!({"candidates": null, "promptFeedback": {"blockReason": "SAFETY"}});
        let mock = MockFetch::new(vec![Canned::json(&body)]);
        let result = create_google(settings(&mock))
            .unwrap()
            .chat("gemini-pro")
            .do_generate(&options())
            .await
            .unwrap();
        assert!(result.content.is_empty());
        assert_eq!(
            result.finish_reason.unified,
            FinishReasonUnified::ContentFilter
        );
        assert_eq!(result.finish_reason.raw.as_deref(), Some("SAFETY"));
    }

    /// Upstream: server tool metadata carries `thoughtSignature` only when it is
    /// not empty (@ai-sdk/google@4.0.87 src/google-language-model.ts:585-594,
    /// :604-613).
    #[tokio::test]
    async fn server_tool_metadata_leaves_out_an_empty_thought_signature() {
        let body = json!({"candidates": [{"content": {"role": "model", "parts": [
        {"toolCall": {"id": "s1", "toolType": "GOOGLE_SEARCH_WEB", "args": {}}, "thoughtSignature": ""},
        {"toolResponse": {"id": "s1", "toolType": "GOOGLE_SEARCH_WEB", "response": {}}, "thoughtSignature": "sig"}
    ]}, "finishReason": "STOP"}]});
        let mock = MockFetch::new(vec![Canned::json(&body)]);
        let result = create_google(settings(&mock))
            .unwrap()
            .chat("gemini-pro")
            .do_generate(&options())
            .await
            .unwrap();
        let content = serde_json::to_value(&result.content).unwrap();
        assert_eq!(
            content[0]["providerMetadata"]["google"],
            json!({"serverToolCallId": "s1", "serverToolType": "GOOGLE_SEARCH_WEB"})
        );
        assert_eq!(
            content[1]["providerMetadata"]["google"]["thoughtSignature"],
            "sig"
        );
    }
}

// Not an upstream case: the shape of the error an in-stream `{"error": ...}`
// event becomes (`google_stream_error` through `create_provider_stream_error`).
mod in_stream_error {
    use super::*;

    use aimux_core::error::AiMuxError;

    #[tokio::test]
    async fn an_error_event_becomes_an_api_call_error_with_its_status_and_data() {
        let error_event = json!({
            "error": {
                "code": 429,
                "message": "Resource has been exhausted",
                "status": "RESOURCE_EXHAUSTED",
            }
        });
        let mock = sse_stream_mock(vec![
            sse_text_chunks(&["Hello"], None, None).concat(),
            sse_data_chunk(&error_event),
        ]);
        let model = stream_test_provider(&mock).chat("gemini-pro");

        let parts: Vec<StreamPart> = model
            .do_stream(&stream_call_options_with(json!({})))
            .await
            .expect("doStream succeeds")
            .stream
            .map(|part| part.expect("stream part"))
            .collect()
            .await;
        let Some(StreamPart::Error {
            error: AiMuxError::ApiCall(error),
        }) = parts
            .iter()
            .find(|part| matches!(part, StreamPart::Error { .. }))
        else {
            panic!("expected an in-stream ApiCall error, got {parts:?}");
        };

        assert_eq!(error.message, "Resource has been exhausted");
        assert_eq!(error.status_code, Some(429));
        assert_eq!(error.provider_code.as_deref(), Some("RESOURCE_EXHAUSTED"));
        assert!(error.is_retryable);
        assert_eq!(error.data, Some(error_event.clone()));
        assert_eq!(error.response_body, Some(error_event.to_string()));
        assert!(error.url.starts_with(STREAM_TEST_BASE_URL));
        assert!(matches!(parts.last(), Some(StreamPart::Finish { .. })));
    }
}
