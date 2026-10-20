//! `anthropic-language-model.test.ts` of `@ai-sdk/anthropic` 4.0.71.
//!
//! Upstream drives `createAnthropic(...)` models against `createTestServer`;
//! here the same models come from [`create_anthropic`] with a scripted
//! transport ([`MockFetch`]). Stream parts, generate results and errors are
//! compared in upstream's JSON shape (the serde form of the aimux types;
//! `undefined` values are omitted keys).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;

use crate::anthropic::options::CANONICAL;
use crate::anthropic::{
    AnthropicMessagesModel, AnthropicProvider, AnthropicProviderSettings, create_anthropic,
};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

/// The pinned user agent aimux sends (upstream mocks `0.0.0-test`).
const ANTHROPIC_PINNED_USER_AGENT: &str = "ai-sdk-anthropic/4.0.71 ai-sdk-provider-utils/5.0.53";

/// Upstream's `TEST_PROMPT`.
fn anthropic_test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

/// An upstream `__fixtures__/<name>.json` file, parsed.
macro_rules! anthropic_json_fixture {
    ($name:literal) => {
        serde_json::from_str::<Value>(include_str!(concat!("fixtures/", $name, ".json")))
            .expect("fixture is JSON")
    };
}

/// An upstream `__snapshots__` entry (stored as JSON under `snapshots/`).
macro_rules! anthropic_snapshot {
    ($name:literal) => {
        serde_json::from_str::<Value>(include_str!(concat!("snapshots/", $name, ".json")))
            .expect("snapshot is JSON")
    };
}

/// The test server's `json-value` response: the JSON body with the
/// `content-type` and `content-length` headers it sets.
fn anthropic_json_response(body: &Value) -> Canned {
    anthropic_json_response_with_headers(body, &[])
}

fn anthropic_json_response_with_headers(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let bytes = serde_json::to_vec(body).unwrap();
    let mut all = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), bytes.len().to_string()),
    ];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    Canned {
        status: 200,
        headers: all,
        body: bytes,
    }
}

/// The test server's error response: a raw body with a status.
fn anthropic_error_response(status: u16, body: &str) -> Canned {
    Canned {
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        body: body.as_bytes().to_vec(),
    }
}

/// Upstream's `mockId({ prefix: 'id' })`: `id-0`, `id-1`, ...
fn anthropic_mock_id() -> Arc<dyn Fn() -> String + Send + Sync> {
    anthropic_mock_id_from(0)
}

/// `mockId({ prefix: 'id' })` after `first` ids were handed out: upstream's
/// file-level `provider` shares one generator across its tests, so a later
/// test sees the count where the earlier tests left it.
fn anthropic_mock_id_from(first: usize) -> Arc<dyn Fn() -> String + Send + Sync> {
    let counter = Arc::new(AtomicUsize::new(first));
    Arc::new(move || format!("id-{}", counter.fetch_add(1, Ordering::SeqCst)))
}

/// Upstream's `createAnthropic({ apiKey: 'test-api-key', generateId: mockId({ prefix: 'id' }) })`
/// against the test server.
fn anthropic_test_provider(mock: &Arc<MockFetch>) -> AnthropicProvider {
    anthropic_test_provider_with(mock, AnthropicProviderSettings::default())
}

fn anthropic_test_provider_with(
    mock: &Arc<MockFetch>,
    settings: AnthropicProviderSettings,
) -> AnthropicProvider {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(mock.transport()),
        generate_id: settings
            .generate_id
            .clone()
            .or_else(|| Some(anthropic_mock_id())),
        ..settings
    })
    .unwrap()
}

/// `LanguageModelV4CallOptions` written as upstream's object literal.
fn anthropic_call_options(options: Value) -> CallOptions {
    serde_json::from_value(options).expect("call options deserialize")
}

/// One `doGenerate` call of `model_id` against one canned response; the
/// result and the transport that recorded the request.
async fn anthropic_try_generate(
    model_id: &str,
    response: Canned,
    options: Value,
) -> (Result<GenerateResult, AiMuxError>, Arc<MockFetch>) {
    let mock = MockFetch::new(vec![response]);
    let result = anthropic_test_provider(&mock)
        .messages(model_id)
        .do_generate(&anthropic_call_options(options))
        .await;
    (result, mock)
}

/// [`anthropic_generate`] with the shared id generator at `first_id`
/// (see [`anthropic_mock_id_from`]).
async fn anthropic_generate_with_first_id(
    model_id: &str,
    response: Canned,
    options: Value,
    first_id: usize,
) -> (GenerateResult, Seen) {
    let mock = MockFetch::new(vec![response]);
    let result = anthropic_test_provider_with(
        &mock,
        AnthropicProviderSettings {
            generate_id: Some(anthropic_mock_id_from(first_id)),
            ..Default::default()
        },
    )
    .messages(model_id)
    .do_generate(&anthropic_call_options(options))
    .await
    .expect("doGenerate succeeds");
    let seen = mock.seen();
    assert_eq!(seen.len(), 1, "one request");
    (result, seen[0].clone())
}

/// One successful `doGenerate` call; the result and the one request sent.
async fn anthropic_generate(
    model_id: &str,
    response: Canned,
    options: Value,
) -> (GenerateResult, Seen) {
    let (result, mock) = anthropic_try_generate(model_id, response, options).await;
    let result = result.expect("doGenerate succeeds");
    let seen = mock.seen();
    assert_eq!(seen.len(), 1, "one request");
    (result, seen[0].clone())
}

/// [`anthropic_generate`] with upstream's default `model` (`claude-3-haiku-20240307`).
async fn anthropic_generate_default(response: Canned, options: Value) -> (GenerateResult, Seen) {
    anthropic_generate("claude-3-haiku-20240307", response, options).await
}

/// A JSON value as JavaScript sees it: a float with no fraction is the same
/// number as the integer (`10.0` and `10` are both `10`).
fn anthropic_js_json(value: Value) -> Value {
    match value {
        Value::Number(number) => match number.as_f64() {
            Some(float)
                if !number.is_i64()
                    && !number.is_u64()
                    && float.fract() == 0.0
                    && float.abs() < 9_007_199_254_740_992.0 =>
            {
                json!(float as i64)
            }
            _ => Value::Number(number),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(anthropic_js_json).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, anthropic_js_json(value)))
                .collect(),
        ),
        other => other,
    }
}

/// A value as upstream's JSON (`undefined` keys omitted).
fn anthropic_to_json<T: Serialize>(value: &T) -> Value {
    anthropic_js_json(serde_json::to_value(value).unwrap())
}

/// The JSON body of a recorded request (`requestBodyJson`).
fn anthropic_request_body(seen: &Seen) -> Value {
    anthropic_js_json(seen.json_body())
}

/// The request headers the test server records, without the user agent
/// (pinned; see [`assert_anthropic_user_agent`]).
fn anthropic_request_headers(seen: &Seen) -> Value {
    Value::Object(
        seen.headers
            .iter()
            .filter(|(name, _)| name.as_str() != "user-agent")
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect(),
    )
}

#[track_caller]
fn assert_anthropic_user_agent(seen: &Seen) {
    assert_eq!(
        seen.headers.get("user-agent").map(String::as_str),
        Some(ANTHROPIC_PINNED_USER_AGENT)
    );
}

/// `expect(actual).toMatchObject(expected)`.
#[track_caller]
fn assert_anthropic_match_object(actual: &Value, expected: &Value) {
    fn matches(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => expected
                .iter()
                .all(|(key, value)| actual.get(key).is_some_and(|found| matches(found, value))),
            (Value::Array(actual), Value::Array(expected)) => {
                actual.len() == expected.len()
                    && actual.iter().zip(expected).all(|(a, e)| matches(a, e))
            }
            _ => actual == expected,
        }
    }
    assert!(
        matches(actual, expected),
        "toMatchObject failed\nactual:   {actual:#}\nexpected: {expected:#}"
    );
}

/// `expect(list).toContainEqual(item)`.
#[track_caller]
fn assert_anthropic_contains(list: &Value, item: &Value) {
    let items = list.as_array().expect("an array");
    assert!(
        items.contains(item),
        "toContainEqual failed\nlist: {list:#}\nitem: {item:#}"
    );
}

/// `expect(actual).toEqual(expected)` for JSON, reporting each differing path.
#[track_caller]
fn assert_anthropic_json_eq(actual: &Value, expected: &Value) {
    fn diff(path: &str, actual: Option<&Value>, expected: Option<&Value>, out: &mut Vec<String>) {
        match (actual, expected) {
            (Some(Value::Object(a)), Some(Value::Object(e))) => {
                for key in e.keys().chain(a.keys().filter(|key| !e.contains_key(*key))) {
                    diff(&format!("{path}.{key}"), a.get(key), e.get(key), out);
                }
            }
            (Some(Value::Array(a)), Some(Value::Array(e))) => {
                for index in 0..a.len().max(e.len()) {
                    diff(&format!("{path}[{index}]"), a.get(index), e.get(index), out);
                }
            }
            (a, e) if a == e => {}
            (a, e) => {
                let show = |value: Option<&Value>| match value {
                    None => "<absent>".to_string(),
                    Some(value) => {
                        let text = value.to_string();
                        if text.len() > 200 {
                            format!("{}...", &text[..text.floor_char_boundary(200)])
                        } else {
                            text
                        }
                    }
                };
                out.push(format!("{path}: actual {} expected {}", show(a), show(e)));
            }
        }
    }
    let mut out = Vec::new();
    diff("$", Some(actual), Some(expected), &mut out);
    assert!(out.is_empty(), "JSON differs:\n{}", out.join("\n"));
}
/// The user agent aimux pins (upstream mocks `0.0.0-test`).
const PINNED_USER_AGENT: &str = "ai-sdk-anthropic/4.0.71 ai-sdk-provider-utils/5.0.53";

/// `TEST_PROMPT`.
fn test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

/// Upstream call options (`LanguageModelV4CallOptions` JSON) as aimux call options.
fn call_options(value: Value) -> CallOptions {
    serde_json::from_value(value).expect("upstream call options deserialize")
}

/// `mockId({ prefix })`: `{prefix}-0`, `{prefix}-1`, ... counting from `start`.
///
/// Upstream's top-level provider shares one counter across the whole test
/// file, so a snapshot taken late in the file starts at a later number;
/// those tests pass the number upstream's counter stands at.
fn mock_id_from(prefix: &'static str, start: usize) -> Arc<dyn Fn() -> String + Send + Sync> {
    let counter = AtomicUsize::new(start);
    Arc::new(move || format!("{prefix}-{}", counter.fetch_add(1, Ordering::SeqCst)))
}

/// The scripted transport answering every call with the given responses.
fn server(responses: Vec<Canned>) -> Arc<MockFetch> {
    MockFetch::new(responses)
}

/// `createAnthropic({ apiKey: 'test-api-key', generateId: mockId({ prefix: 'id' }) })`,
/// the file's top-level `provider`.
fn test_provider(mock: &Arc<MockFetch>) -> crate::anthropic::AnthropicProvider {
    test_provider_with_ids(mock, 0)
}

/// [`test_provider`] whose id counter stands at `start`.
fn test_provider_with_ids(
    mock: &Arc<MockFetch>,
    start: usize,
) -> crate::anthropic::AnthropicProvider {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        generate_id: Some(mock_id_from("id", start)),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("provider")
}

/// `createAnthropic({ apiKey: 'test-api-key' })`.
fn plain_provider(mock: &Arc<MockFetch>) -> crate::anthropic::AnthropicProvider {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("provider")
}

/// The file's top-level `model`: `provider('claude-3-haiku-20240307')`.
fn test_model(mock: &Arc<MockFetch>) -> AnthropicMessagesModel {
    test_provider(mock).messages("claude-3-haiku-20240307")
}

/// The test server's default `stream-chunks` response headers.
fn sse_headers() -> Vec<(String, String)> {
    vec![
        ("content-type".into(), "text/event-stream".into()),
        ("cache-control".into(), "no-cache".into()),
        ("connection".into(), "keep-alive".into()),
    ]
}

/// A `stream-chunks` response whose chunks are `data: {chunk}\n\n`.
fn sse_data(chunks: &[&str]) -> Canned {
    let body: String = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    Canned {
        status: 200,
        headers: sse_headers(),
        body: body.into_bytes(),
    }
}

/// A `stream-chunks` response of verbatim chunks.
#[allow(dead_code)]
fn sse_raw(chunks: &[&str]) -> Canned {
    Canned {
        status: 200,
        headers: sse_headers(),
        body: chunks.concat().into_bytes(),
    }
}

/// `prepareChunksFixtureResponse`: every line of the fixture (the empty one
/// after a final newline included) as `data: {line}\n\n`, then `data: [DONE]`.
fn sse_fixture(text: &str) -> Canned {
    let mut body: String = text
        .split('\n')
        .map(|line| format!("data: {line}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    Canned {
        status: 200,
        headers: sse_headers(),
        body: body.into_bytes(),
    }
}

/// A `json-value` response.
fn json_response(body: Value) -> Canned {
    Canned::json(&body)
}

/// `prepareJsonFixtureResponse`.
fn json_fixture(text: &str) -> Canned {
    json_response(serde_json::from_str(text).expect("fixture is JSON"))
}

/// `server.calls[index].requestBodyJson`.
fn request_body(mock: &MockFetch, index: usize) -> Value {
    mock.seen()[index].json_body()
}

/// `server.calls[index].requestHeaders`, without the user agent, which is
/// compared to the pinned one here.
fn request_headers(mock: &MockFetch, index: usize) -> Value {
    let mut headers = mock.seen()[index].headers.clone();
    let user_agent = headers.remove("user-agent");
    assert_eq!(user_agent.as_deref(), Some(PINNED_USER_AGENT), "user agent");
    serde_json::to_value(headers).unwrap()
}

/// One request header, `None` when it was not sent.
fn request_header(mock: &MockFetch, index: usize, name: &str) -> Option<String> {
    mock.seen()[index].headers.get(name).cloned()
}

/// An error in the shape upstream's stream error parts and thrown
/// `APICallError`s are compared in.
fn error_json(error: &AiMuxError) -> Value {
    match error {
        AiMuxError::ApiCall(details) => {
            let frame = details
                .response_body
                .as_deref()
                .and_then(|body| serde_json::from_str::<Value>(body).ok());
            let mut out = serde_json::Map::new();
            if let Some(code) = &details.provider_code {
                out.insert("type".into(), json!(code));
            }
            out.insert("message".into(), json!(details.message));
            if let Some(code) = frame
                .as_ref()
                .and_then(|frame| frame.get("error"))
                .and_then(|error| error.get("code"))
            {
                out.insert("code".into(), code.clone());
            }
            if let Some(status) = details.status_code {
                out.insert("statusCode".into(), json!(status));
            }
            // `is_retryable` is a bool: an error type upstream infers no
            // metadata for leaves both `statusCode` and `isRetryable`
            // undefined, which aimux reports as no status and `false`.
            if details.status_code.is_some() || details.is_retryable {
                out.insert("isRetryable".into(), json!(details.is_retryable));
            }
            if let Some(data) = &details.data {
                out.insert("data".into(), data.clone());
            }
            Value::Object(out)
        }
        AiMuxError::JsonParse(message) => json!({
            "$error": { "name": "AI_JSONParseError", "message": message }
        }),
        other => json!({ "$error": { "name": "AiMuxError", "message": other.to_string() } }),
    }
}

/// A thrown `APICallError` as upstream's tests pick its fields.
fn api_call_error_json(error: &AiMuxError) -> Value {
    let AiMuxError::ApiCall(details) = error else {
        panic!("expected an APICallError, got {error:?}");
    };
    let mut out = serde_json::Map::new();
    out.insert("message".into(), json!(details.message));
    out.insert("url".into(), json!(details.url));
    out.insert(
        "requestBodyValues".into(),
        details.request_body_values.clone(),
    );
    if let Some(status) = details.status_code {
        out.insert("statusCode".into(), json!(status));
    }
    if let Some(headers) = &details.response_headers {
        out.insert(
            "responseHeaders".into(),
            serde_json::to_value(headers).unwrap(),
        );
    }
    if let Some(body) = &details.response_body {
        out.insert("responseBody".into(), json!(body));
    }
    out.insert("isRetryable".into(), json!(details.is_retryable));
    Value::Object(out)
}

/// A stream item in upstream's `LanguageModelV4StreamPart` JSON shape.
fn part_json(item: &Result<StreamPart, AiMuxError>) -> Value {
    match item {
        Ok(StreamPart::Error { error }) | Err(error) => {
            json!({ "type": "error", "error": error_json(error) })
        }
        Ok(part) => serde_json::to_value(part).expect("stream part serializes"),
    }
}

/// `convertReadableStreamToArray(result.stream)`.
async fn collect_parts(result: StreamResult) -> Vec<Value> {
    let items: Vec<_> = result.stream.collect().await;
    items.iter().map(part_json).collect()
}

/// `doStream` then `convertReadableStreamToArray`.
async fn stream_parts(model: &AnthropicMessagesModel, options: Value) -> Vec<Value> {
    collect_parts(
        model
            .do_stream(&call_options(options))
            .await
            .expect("doStream"),
    )
    .await
}

/// `doGenerate`.
async fn generate(model: &AnthropicMessagesModel, options: Value) -> GenerateResult {
    model
        .do_generate(&call_options(options))
        .await
        .expect("doGenerate")
}

/// `GenerateResult` fields as upstream JSON.
fn content_json(result: &GenerateResult) -> Value {
    serde_json::to_value(&result.content).unwrap()
}

fn warnings_json(warnings: &[aimux_core::types::Warning]) -> Value {
    serde_json::to_value(warnings).unwrap()
}

/// The first part of `type`.
fn find_part<'a>(parts: &'a [Value], kind: &str) -> Option<&'a Value> {
    parts.iter().find(|part| part["type"] == kind)
}

/// Every part of `type`.
fn filter_parts(parts: &[Value], kind: &str) -> Vec<Value> {
    parts
        .iter()
        .filter(|part| part["type"] == kind)
        .cloned()
        .collect()
}

/// The deltas of the parts of `type` (and `id`, when given), joined.
fn join_deltas(parts: &[Value], kind: &str, id: Option<&str>) -> String {
    parts
        .iter()
        .filter(|part| part["type"] == kind && id.is_none_or(|id| part["id"] == id))
        .filter_map(|part| part["delta"].as_str())
        .collect()
}

// ── vitest matchers ──────────────────────────────────────────────────────────
//
// Expected values may hold the asymmetric matchers as one-key objects:
// `{"$objectContaining": {..}}`, `{"$arrayContaining": [..]}`,
// `{"$stringContaining": ".."}`, `{"$stringMatching": "regex"}`,
// `{"$any": "String"}` and `{"$error": {"name", "message"}}` (a printed error).

fn matcher(expected: &Value) -> Option<(&str, &Value)> {
    let object = expected.as_object()?;
    let (key, value) = object.iter().next()?;
    (object.len() == 1 && key.starts_with('$')).then_some((key.as_str(), value))
}

/// `toEqual` (recursive equality; asymmetric matchers apply).
fn equals(actual: &Value, expected: &Value) -> bool {
    if let Some((kind, arg)) = matcher(expected) {
        return apply_matcher(kind, arg, actual);
    }
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            actual.len() == expected.len()
                && expected
                    .iter()
                    .all(|(key, value)| actual.get(key).is_some_and(|actual| equals(actual, value)))
        }
        (Value::Array(actual), Value::Array(expected)) => {
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| equals(actual, expected))
        }
        (Value::Number(actual), Value::Number(expected)) => actual.as_f64() == expected.as_f64(),
        _ => actual == expected,
    }
}

/// `toMatchObject` (objects match on the expected keys, recursively).
fn matches_object(actual: &Value, expected: &Value) -> bool {
    if let Some((kind, arg)) = matcher(expected) {
        return apply_matcher(kind, arg, actual);
    }
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
        _ => equals(actual, expected),
    }
}

fn apply_matcher(kind: &str, arg: &Value, actual: &Value) -> bool {
    match kind {
        "$objectContaining" => arg
            .as_object()
            .unwrap()
            .iter()
            .all(|(key, value)| actual.get(key).is_some_and(|actual| equals(actual, value))),
        "$arrayContaining" => arg.as_array().unwrap().iter().all(|expected| {
            actual
                .as_array()
                .is_some_and(|actual| actual.iter().any(|actual| equals(actual, expected)))
        }),
        "$stringContaining" => actual
            .as_str()
            .is_some_and(|actual| actual.contains(arg.as_str().unwrap())),
        "$stringMatching" => actual.as_str().is_some_and(|actual| {
            regex::Regex::new(arg.as_str().unwrap())
                .unwrap()
                .is_match(actual)
        }),
        "$any" => match arg.as_str().unwrap() {
            "String" => actual.is_string(),
            "Number" => actual.is_number(),
            "Object" => actual.is_object(),
            "Array" => actual.is_array(),
            other => panic!("expect.any({other})"),
        },
        // A printed error: its name and message.
        "$error" => actual.get("$error").is_some_and(|actual| actual == arg),
        other => panic!("unknown matcher {other}"),
    }
}

/// `value` with every object's keys sorted, pretty-printed (for diffs).
fn pretty_sorted(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut entries: Vec<_> = object.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(k, v)| (k.clone(), sorted(v)))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string_pretty(&sorted(value)).unwrap()
}

#[track_caller]
fn assert_equals(actual: &Value, expected: &Value) {
    if !equals(actual, expected) {
        assert_eq!(pretty_sorted(actual), pretty_sorted(expected), "toEqual");
        panic!("toEqual (asymmetric matcher failed)");
    }
}

#[track_caller]
fn assert_match_object(actual: &Value, expected: &Value) {
    assert!(
        matches_object(actual, expected),
        "toMatchObject\nactual: {}\nexpected: {}",
        serde_json::to_string_pretty(actual).unwrap(),
        serde_json::to_string_pretty(expected).unwrap()
    );
}

#[track_caller]
fn assert_contains_equal(actual: &[Value], expected: &Value) {
    assert!(
        actual.iter().any(|actual| equals(actual, expected)),
        "toContainEqual\nactual: {}\nexpected: {}",
        serde_json::to_string_pretty(actual).unwrap(),
        serde_json::to_string_pretty(expected).unwrap()
    );
}

/// A `toMatchSnapshot()` value stored in `snapshots/`.
fn snapshot(text: &str) -> Value {
    serde_json::from_str(text).expect("snapshot is JSON")
}

/// Cuts every `AI_JSONParseError` message at `\nError message: `: what
/// follows is the JS engine's own `JSON.parse` text, which serde words
/// differently.
fn without_parser_message(mut value: Value) -> Value {
    fn walk(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if map.get("name").and_then(Value::as_str) == Some("AI_JSONParseError")
                    && let Some(Value::String(message)) = map.get_mut("message")
                    && let Some(at) = message.find("\nError message: ")
                {
                    message.truncate(at);
                }
                map.values_mut().for_each(walk);
            }
            Value::Array(items) => items.iter_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut value);
    value
}

const ANTHROPIC_TEXT: &str = include_str!("fixtures/anthropic-text.json");

mod anthropic_language_model {
    use super::*;

    mod do_generate {
        use super::*;
        mod reasoning_thinking_enabled {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › reasoning (thinking enabled) › should pass thinking config; add budget tokens; clear out temperature, top_p, top_k; and return warnings
            #[tokio::test]
            async fn should_pass_thinking_config_add_budget_tokens_clear_sampling_and_return_warnings()
             {
                let (result, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "maxOutputTokens": 20000,
                        "temperature": 0.5,
                        "topP": 0.7,
                        "topK": 0.1,
                        "providerOptions": {
                            CANONICAL: { "thinking": { "type": "enabled", "budgetTokens": 1000 } }
                        },
                    }),
                )
                .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 21000,
                        "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                        "model": "claude-sonnet-4-5",
                        "thinking": { "budget_tokens": 1000, "type": "enabled" },
                    })
                );
                assert_eq!(
                    anthropic_to_json(&result.warnings),
                    json!([
                        {
                            "details": "temperature is not supported when thinking is enabled",
                            "feature": "temperature",
                            "type": "unsupported",
                        },
                        {
                            "details": "topK is not supported when thinking is enabled",
                            "feature": "topK",
                            "type": "unsupported",
                        },
                        {
                            "details": "topP is not supported when thinking is enabled",
                            "feature": "topP",
                            "type": "unsupported",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › reasoning (thinking enabled) › should extract reasoning response
            #[tokio::test]
            async fn should_extract_reasoning_response() {
                let (result, _) = anthropic_generate_default(
            anthropic_json_response(&json!({
                "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "I am thinking...", "signature": "1234567890" },
                    { "type": "text", "text": "Hello, World!" },
                ],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            })),
            json!({ "prompt": anthropic_test_prompt() }),
        )
        .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        {
                            "providerMetadata": { CANONICAL: { "signature": "1234567890" } },
                            "text": "I am thinking...",
                            "type": "reasoning",
                        },
                        { "text": "Hello, World!", "type": "text" },
                    ])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › reasoning (thinking enabled) › should preserve container upload response blocks as custom content
            #[tokio::test]
            async fn should_preserve_container_upload_response_blocks_as_custom_content() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&json!({
                        "id": "msg_container_upload",
                        "type": "message",
                        "role": "assistant",
                        "content": [
                            { "type": "text", "text": "Done" },
                            { "type": "container_upload", "file_id": "file_123" },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stop_reason": "end_turn",
                        "stop_sequence": null,
                        "usage": { "input_tokens": 4, "output_tokens": 2 },
                    })),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert_anthropic_contains(
                    &anthropic_to_json(&result.content),
                    &json!({
                        "type": "custom",
                        "kind": "anthropic.container_upload",
                        "providerMetadata": { CANONICAL: { "fileId": "file_123" } },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › reasoning (thinking enabled) › should use default budget when thinking type is enabled without budgetTokens
            #[tokio::test]
            async fn should_use_default_budget_when_thinking_type_is_enabled_without_budget_tokens()
            {
                let (result, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "providerOptions": { CANONICAL: { "thinking": { "type": "enabled" } } },
                    }),
                )
                .await;

                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({ "thinking": { "type": "enabled", "budget_tokens": 1024 } }),
                );
                assert_anthropic_contains(
                    &anthropic_to_json(&result.warnings),
                    &json!({
                        "type": "compatibility",
                        "feature": "extended thinking",
                        "details": "thinking budget is required when thinking is enabled. using default budget of 1024 tokens.",
                    }),
                );
            }
        }

        mod reasoning_adaptive_thinking {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › reasoning (adaptive thinking) › should send adaptive thinking without budget_tokens
            #[tokio::test]
            async fn should_send_adaptive_thinking_without_budget_tokens() {
                let (result, seen) = anthropic_generate(
                    "claude-opus-4-6",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "providerOptions": { CANONICAL: { "thinking": { "type": "adaptive" } } },
                    }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert_anthropic_match_object(
                    &body,
                    &json!({ "thinking": { "type": "adaptive" } }),
                );
                assert!(body["thinking"].get("budget_tokens").is_none());
                assert_eq!(anthropic_to_json(&result.warnings), json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › reasoning (adaptive thinking) › should report thinking tokens as reasoning usage
            #[tokio::test]
            async fn should_report_thinking_tokens_as_reasoning_usage() {
                let (result, _) = anthropic_generate(
                    "claude-opus-5",
                    anthropic_json_response(&anthropic_json_fixture!(
                        "anthropic-claude-opus-5-reasoning-high.1"
                    )),
                    json!({ "prompt": anthropic_test_prompt(), "reasoning": "high" }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.usage.output_tokens),
                    json!({ "total": 1699, "text": 1560, "reasoning": 139 })
                );
            }
        }

        mod reasoning_thinking_disabled {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › reasoning (thinking disabled) › should forward thinking { type: "disabled" } to the API instead of stripping it
            #[tokio::test]
            async fn should_forward_thinking_type_disabled_to_the_api_instead_of_stripping_it() {
                let (result, seen) = anthropic_generate(
                    "claude-sonnet-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "maxOutputTokens": 100,
                        "providerOptions": { CANONICAL: { "thinking": { "type": "disabled" } } },
                    }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert_eq!(body["thinking"], json!({ "type": "disabled" }));
                // disabled thinking must not reserve a thinking budget in max_tokens
                assert_eq!(body["max_tokens"], json!(100));
                assert_eq!(anthropic_to_json(&result.warnings), json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › reasoning (thinking disabled) › should not strip temperature / topP / topK when thinking is disabled
            #[tokio::test]
            async fn should_not_strip_temperature_top_p_top_k_when_thinking_is_disabled() {
                let (result, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "maxOutputTokens": 100,
                        "temperature": 0.5,
                        "topK": 0.1,
                        "providerOptions": { CANONICAL: { "thinking": { "type": "disabled" } } },
                    }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert_eq!(body["thinking"], json!({ "type": "disabled" }));
                // unlike enabled thinking, disabled thinking keeps sampling params
                assert_eq!(body["temperature"], json!(0.5));
                assert_eq!(body["top_k"], json!(0.1));
                assert_eq!(anthropic_to_json(&result.warnings), json!([]));
            }
        }

        mod top_level_reasoning_newer_models_with_adaptive_thinking {
            use super::*;

            async fn generate_with_reasoning(model_id: &str, reasoning: &str) -> (Value, Value) {
                let (result, seen) = anthropic_generate(
                    model_id,
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "reasoning": reasoning }),
                )
                .await;
                (
                    anthropic_request_body(&seen),
                    anthropic_to_json(&result.warnings),
                )
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (newer models with adaptive thinking) › should not set thinking config when reasoning is "provider-default"
            #[tokio::test]
            async fn should_not_set_thinking_config_when_reasoning_is_provider_default() {
                let (body, warnings) =
                    generate_with_reasoning("claude-sonnet-4-6", "provider-default").await;
                assert!(body.get("thinking").is_none());
                assert!(body.get("output_config").is_none());
                assert_eq!(warnings, json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (newer models with adaptive thinking) › should map reasoning "none" to thinking disabled
            #[tokio::test]
            async fn should_map_reasoning_none_to_thinking_disabled() {
                let (body, warnings) = generate_with_reasoning("claude-sonnet-4-6", "none").await;
                assert_eq!(body["thinking"], json!({ "type": "disabled" }));
                assert!(body.get("output_config").is_none());
                assert_eq!(warnings, json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (newer models with adaptive thinking) › should map reasoning "low" to adaptive thinking with effort "low"
            #[tokio::test]
            async fn should_map_reasoning_low_to_adaptive_thinking_with_effort_low() {
                let (body, warnings) = generate_with_reasoning("claude-sonnet-4-6", "low").await;
                assert_anthropic_match_object(
                    &body,
                    &json!({
                        "thinking": { "type": "adaptive", "display": "summarized" },
                        "output_config": { "effort": "low" },
                    }),
                );
                assert_eq!(warnings, json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (newer models with adaptive thinking) › should map reasoning "medium" to adaptive thinking with effort "medium"
            #[tokio::test]
            async fn should_map_reasoning_medium_to_adaptive_thinking_with_effort_medium() {
                let (body, warnings) = generate_with_reasoning("claude-sonnet-4-6", "medium").await;
                assert_anthropic_match_object(
                    &body,
                    &json!({
                        "thinking": { "type": "adaptive", "display": "summarized" },
                        "output_config": { "effort": "medium" },
                    }),
                );
                assert_eq!(warnings, json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (newer models with adaptive thinking) › should map reasoning "high" to adaptive thinking with effort "high"
            #[tokio::test]
            async fn should_map_reasoning_high_to_adaptive_thinking_with_effort_high() {
                let (body, warnings) = generate_with_reasoning("claude-sonnet-4-6", "high").await;
                assert_anthropic_match_object(
                    &body,
                    &json!({
                        "thinking": { "type": "adaptive", "display": "summarized" },
                        "output_config": { "effort": "high" },
                    }),
                );
                assert_eq!(warnings, json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (newer models with adaptive thinking) › should map reasoning "xhigh" to adaptive thinking with effort "max"
            #[tokio::test]
            async fn should_map_reasoning_xhigh_to_adaptive_thinking_with_effort_max() {
                let (body, warnings) = generate_with_reasoning("claude-sonnet-4-6", "xhigh").await;
                assert_anthropic_match_object(
                    &body,
                    &json!({
                        "thinking": { "type": "adaptive", "display": "summarized" },
                        "output_config": { "effort": "max" },
                    }),
                );
                assert_anthropic_contains(
                    &warnings,
                    &json!({
                        "type": "compatibility",
                        "feature": "reasoning",
                        "details": "reasoning \"xhigh\" is not directly supported by this model. mapped to effort \"max\".",
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (newer models with adaptive thinking) › should map reasoning "minimal" to adaptive thinking with effort "low" and emit compatibility warning
            #[tokio::test]
            async fn should_map_reasoning_minimal_to_adaptive_thinking_with_effort_low_and_warn() {
                let (body, warnings) = generate_with_reasoning("claude-opus-4-6", "minimal").await;
                assert_anthropic_match_object(
                    &body,
                    &json!({
                        "thinking": { "type": "adaptive", "display": "summarized" },
                        "output_config": { "effort": "low" },
                    }),
                );
                assert_anthropic_contains(
                    &warnings,
                    &json!({
                        "type": "compatibility",
                        "feature": "reasoning",
                        "details": "reasoning \"minimal\" is not directly supported by this model. mapped to effort \"low\".",
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (newer models with adaptive thinking) › should strip temperature, topP, topK when reasoning enables thinking
            #[tokio::test]
            async fn should_strip_temperature_top_p_top_k_when_reasoning_enables_thinking() {
                let (result, seen) = anthropic_generate(
                    "claude-sonnet-4-6",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "reasoning": "high",
                        "temperature": 0.5,
                        "topP": 0.7,
                        "topK": 10,
                    }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert!(body.get("temperature").is_none());
                assert!(body.get("top_p").is_none());
                assert!(body.get("top_k").is_none());
                let warnings = anthropic_to_json(&result.warnings);
                for (feature, details) in [
                    (
                        "temperature",
                        "temperature is not supported when thinking is enabled",
                    ),
                    ("topK", "topK is not supported when thinking is enabled"),
                    ("topP", "topP is not supported when thinking is enabled"),
                ] {
                    assert_anthropic_contains(
                        &warnings,
                        &json!({ "type": "unsupported", "feature": feature, "details": details }),
                    );
                }
            }
        }

        mod top_level_reasoning_older_models_with_budget_based_thinking {
            use super::*;

            async fn generate_with(model_id: &str, options: Value) -> (Value, Value) {
                let mut options = options;
                options["prompt"] = anthropic_test_prompt();
                let (result, seen) = anthropic_generate(
                    model_id,
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    options,
                )
                .await;
                (
                    anthropic_request_body(&seen),
                    anthropic_to_json(&result.warnings),
                )
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should not set thinking config when reasoning is "provider-default"
            #[tokio::test]
            async fn should_not_set_thinking_config_when_reasoning_is_provider_default() {
                let (body, warnings) = generate_with(
                    "claude-sonnet-4-5",
                    json!({ "reasoning": "provider-default" }),
                )
                .await;
                assert!(body.get("thinking").is_none());
                assert_eq!(warnings, json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should map reasoning "none" to thinking disabled
            #[tokio::test]
            async fn should_map_reasoning_none_to_thinking_disabled() {
                let (body, warnings) =
                    generate_with("claude-sonnet-4-5", json!({ "reasoning": "none" })).await;
                assert_eq!(body["thinking"], json!({ "type": "disabled" }));
                assert_eq!(warnings, json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should map reasoning "minimal" to enabled thinking with ~2% budget
            #[tokio::test]
            async fn should_map_reasoning_minimal_to_enabled_thinking_with_2_percent_budget() {
                let (body, _) =
                    generate_with("claude-sonnet-4-5", json!({ "reasoning": "minimal" })).await;
                assert_anthropic_match_object(
                    &body,
                    &json!({ "thinking": { "type": "enabled", "budget_tokens": 1280 } }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should map reasoning "low" to enabled thinking with ~10% budget
            #[tokio::test]
            async fn should_map_reasoning_low_to_enabled_thinking_with_10_percent_budget() {
                let (body, _) =
                    generate_with("claude-sonnet-4-5", json!({ "reasoning": "low" })).await;
                assert_anthropic_match_object(
                    &body,
                    &json!({ "thinking": { "type": "enabled", "budget_tokens": 6400 } }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should map reasoning "medium" to enabled thinking with ~30% budget
            #[tokio::test]
            async fn should_map_reasoning_medium_to_enabled_thinking_with_30_percent_budget() {
                let (body, _) =
                    generate_with("claude-sonnet-4-5", json!({ "reasoning": "medium" })).await;
                assert_anthropic_match_object(
                    &body,
                    &json!({ "thinking": { "type": "enabled", "budget_tokens": 19200 } }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should map reasoning "high" to enabled thinking with ~60% budget
            #[tokio::test]
            async fn should_map_reasoning_high_to_enabled_thinking_with_60_percent_budget() {
                let (body, _) =
                    generate_with("claude-sonnet-4-5", json!({ "reasoning": "high" })).await;
                assert_anthropic_match_object(
                    &body,
                    &json!({ "thinking": { "type": "enabled", "budget_tokens": 38400 } }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should map reasoning "xhigh" to enabled thinking with ~90% budget
            #[tokio::test]
            async fn should_map_reasoning_xhigh_to_enabled_thinking_with_90_percent_budget() {
                let (body, _) =
                    generate_with("claude-sonnet-4-5", json!({ "reasoning": "xhigh" })).await;
                assert_anthropic_match_object(
                    &body,
                    &json!({ "thinking": { "type": "enabled", "budget_tokens": 57600 } }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should clamp budget to minimum 1024 tokens
            #[tokio::test]
            async fn should_clamp_budget_to_minimum_1024_tokens() {
                let (body, _) =
                    generate_with("claude-3-haiku-20240307", json!({ "reasoning": "minimal" }))
                        .await;
                assert_anthropic_match_object(
                    &body,
                    &json!({ "thinking": { "type": "enabled", "budget_tokens": 1024 } }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should adjust max_tokens to include thinking budget
            #[tokio::test]
            async fn should_adjust_max_tokens_to_include_thinking_budget() {
                let (body, _) = generate_with(
                    "claude-sonnet-4-5",
                    json!({ "reasoning": "low", "maxOutputTokens": 10000 }),
                )
                .await;
                assert_eq!(body["max_tokens"], json!(16400));
                assert_eq!(body["thinking"]["budget_tokens"], json!(6400));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (older models with budget-based thinking) › should strip temperature, topP, topK when reasoning enables thinking
            #[tokio::test]
            async fn should_strip_temperature_top_p_top_k_when_reasoning_enables_thinking() {
                let (body, warnings) = generate_with(
                    "claude-sonnet-4-5",
                    json!({ "reasoning": "medium", "temperature": 0.5, "topP": 0.7, "topK": 10 }),
                )
                .await;
                assert!(body.get("temperature").is_none());
                assert!(body.get("top_p").is_none());
                assert!(body.get("top_k").is_none());
                assert_anthropic_contains(
                    &warnings,
                    &json!({
                        "type": "unsupported",
                        "feature": "temperature",
                        "details": "temperature is not supported when thinking is enabled",
                    }),
                );
            }
        }

        mod forward_compatible_defaults_for_unknown_claude_models {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › forward-compatible defaults for unknown Claude models › should map xhigh reasoning to adaptive thinking with xhigh effort
            #[tokio::test]
            async fn should_map_xhigh_reasoning_to_adaptive_thinking_with_xhigh_effort() {
                let (result, seen) = anthropic_generate(
            "claude-future-9",
            anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
            json!({ "prompt": anthropic_test_prompt(), "maxOutputTokens": 100, "reasoning": "xhigh" }),
        )
        .await;

                let body = anthropic_request_body(&seen);
                assert_eq!(
                    body["thinking"],
                    json!({ "type": "adaptive", "display": "summarized" })
                );
                assert!(body["thinking"].get("budget_tokens").is_none());
                assert_eq!(body["output_config"], json!({ "effort": "xhigh" }));
                assert_eq!(anthropic_to_json(&result.warnings), json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › forward-compatible defaults for unknown Claude models › should strip unsupported sampling parameters with warnings
            #[tokio::test]
            async fn should_strip_unsupported_sampling_parameters_with_warnings() {
                let (result, seen) = anthropic_generate(
                    "claude-future-9",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "maxOutputTokens": 100,
                        "temperature": 0.5,
                        "topP": 0.7,
                        "topK": 10,
                    }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert!(body.get("temperature").is_none());
                assert!(body.get("top_p").is_none());
                assert!(body.get("top_k").is_none());
                assert_eq!(
                    anthropic_to_json(&result.warnings),
                    json!([
                        {
                            "type": "unsupported",
                            "feature": "temperature",
                            "details": "temperature is not supported by claude-future-9 and will be ignored",
                        },
                        {
                            "type": "unsupported",
                            "feature": "topK",
                            "details": "topK is not supported by claude-future-9 and will be ignored",
                        },
                        {
                            "type": "unsupported",
                            "feature": "topP",
                            "details": "topP is not supported by claude-future-9 and will be ignored",
                        },
                    ])
                );
            }

            // `should use native structured output without a JSON tool fallback`:
            // tests/anthropic_upstream_e2e_test.rs `generate_native_structured_output`.
        }

        mod legacy_claude_model_defaults {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › legacy Claude model defaults › should retain sampling parameters and the JSON tool fallback
            #[tokio::test]
            async fn should_retain_sampling_parameters_and_the_json_tool_fallback() {
                let (_, seen) = anthropic_generate(
                    "claude-3-5-sonnet-20241022",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-json-tool.1")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "maxOutputTokens": 100,
                        "temperature": 0.5,
                        "topK": 10,
                        "responseFormat": {
                            "type": "json",
                            "schema": {
                                "type": "object",
                                "properties": { "name": { "type": "string" } },
                                "required": ["name"],
                                "additionalProperties": false,
                            },
                        },
                    }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert_eq!(body["temperature"], json!(0.5));
                assert_eq!(body["top_k"], json!(10));
                assert!(body.get("output_config").is_none());
                assert_eq!(body["tools"].as_array().unwrap().len(), 1);
                assert_eq!(body["tools"][0]["name"], json!("json"));
            }
        }

        mod top_level_reasoning_provider_options_precedence {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (providerOptions precedence) › should let anthropic.thinking take precedence over top-level reasoning
            #[tokio::test]
            async fn should_let_anthropic_thinking_take_precedence_over_top_level_reasoning() {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-6",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "reasoning": "high",
                        "providerOptions": { CANONICAL: { "thinking": { "type": "disabled" } } },
                    }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert_eq!(body["thinking"], json!({ "type": "disabled" }));
                assert!(body.get("output_config").is_none());
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (providerOptions precedence) › should let anthropic.effort take precedence over top-level reasoning
            #[tokio::test]
            async fn should_let_anthropic_effort_take_precedence_over_top_level_reasoning() {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-6",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "reasoning": "high",
                        "providerOptions": { CANONICAL: { "effort": "low" } },
                    }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert!(body.get("thinking").is_none());
                assert_anthropic_match_object(
                    &body,
                    &json!({ "output_config": { "effort": "low" } }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › top-level reasoning (providerOptions precedence) › should not apply top-level reasoning when anthropic.thinking is set
            #[tokio::test]
            async fn should_not_apply_top_level_reasoning_when_anthropic_thinking_is_set() {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "reasoning": "xhigh",
                        "providerOptions": {
                            CANONICAL: { "thinking": { "type": "enabled", "budgetTokens": 5000 } }
                        },
                    }),
                )
                .await;

                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({ "thinking": { "type": "enabled", "budget_tokens": 5000 } }),
                );
            }
        }
        fn anthropic_name_schema_with_draft() -> Value {
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
                "additionalProperties": false,
                "$schema": "http://json-schema.org/draft-07/schema#",
            })
        }

        const JSON_TOOL_FIXTURE_TEXT: &str = r#"{"elements":[{"location":"San Francisco","temperature":-5,"condition":"snowy"},{"location":"London","temperature":0,"condition":"snowy"},{"location":"Paris","temperature":23,"condition":"cloudy"},{"location":"Berlin","temperature":-9,"condition":"snowy"}]}"#;

        const JSON_OUTPUT_FORMAT_FIXTURE_TEXT: &str = r#"{"recipe":{"name":"Classic Lasagna","ingredients":[{"name":"lasagna noodles","amount":"12 sheets"},{"name":"ground beef","amount":"1 pound"},{"name":"Italian sausage","amount":"1/2 pound"},{"name":"onion","amount":"1 medium, diced"},{"name":"garlic","amount":"3 cloves, minced"},{"name":"crushed tomatoes","amount":"28 oz can"},{"name":"tomato paste","amount":"6 oz can"},{"name":"water","amount":"1/2 cup"},{"name":"sugar","amount":"2 tablespoons"},{"name":"dried basil","amount":"1 1/2 teaspoons"},{"name":"Italian seasoning","amount":"1 teaspoon"},{"name":"salt","amount":"1 teaspoon"},{"name":"black pepper","amount":"1/2 teaspoon"},{"name":"ricotta cheese","amount":"15 oz"},{"name":"egg","amount":"1 large"},{"name":"fresh parsley","amount":"2 tablespoons, chopped"},{"name":"mozzarella cheese","amount":"3 cups, shredded"},{"name":"parmesan cheese","amount":"3/4 cup, grated"}],"steps":["Cook lasagna noodles according to package directions, drain and set aside","In a large skillet, cook ground beef, sausage, onion, and garlic over medium heat until meat is browned, drain excess fat","Add crushed tomatoes, tomato paste, water, sugar, basil, Italian seasoning, salt, and pepper to the meat mixture, simmer for 30 minutes, stirring occasionally","In a bowl, combine ricotta cheese, egg, and parsley, mix well","Preheat oven to 375°F (190°C)","Spread 1 cup of meat sauce in the bottom of a 9x13 inch baking dish","Layer 4 lasagna noodles over the sauce","Spread half of the ricotta mixture over the noodles","Sprinkle with 1 cup mozzarella and 1/4 cup parmesan cheese","Top with 1 1/2 cups meat sauce","Repeat layers once more: 4 noodles, remaining ricotta mixture, 1 cup mozzarella, 1/4 cup parmesan, 1 1/2 cups meat sauce","Add final layer of 4 noodles, remaining meat sauce, remaining mozzarella and parmesan cheese","Cover with aluminum foil and bake for 25 minutes","Remove foil and bake an additional 25 minutes until cheese is golden and bubbly","Let stand for 15 minutes before serving"]}}"#;

        mod json_schema_response_format_with_json_tool_response_unsupported_model {
            use super::*;

            async fn before_each() -> (GenerateResult, Seen) {
                anthropic_generate_default(
            anthropic_json_response(&anthropic_json_fixture!("anthropic-json-tool.1")),
            json!({
                "prompt": anthropic_test_prompt(),
                "responseFormat": { "type": "json", "schema": anthropic_name_schema_with_draft() },
            }),
        )
        .await
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with json tool response (unsupported model) › should pass json schema response format as a tool
            #[tokio::test]
            async fn should_pass_json_schema_response_format_as_a_tool() {
                let (_, seen) = before_each().await;
                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                        "model": "claude-3-haiku-20240307",
                        "tool_choice": { "disable_parallel_tool_use": true, "type": "any" },
                        "tools": [{
                            "description": "Respond with a JSON object.",
                            "input_schema": anthropic_name_schema_with_draft(),
                            "name": "json",
                        }],
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with json tool response (unsupported model) › should return the json response
            #[tokio::test]
            async fn should_return_the_json_response() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{ "text": JSON_TOOL_FIXTURE_TEXT, "type": "text" }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with json tool response (unsupported model) › should send stop finish reason
            #[tokio::test]
            async fn should_send_stop_finish_reason() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "raw": "tool_use", "unified": "stop" })
                );
            }
        }

        mod json_schema_response_format_with_json_tool_response_supported_model_tool_mode {
            use super::*;

            async fn before_each() -> (GenerateResult, Seen) {
                anthropic_generate(
            "claude-sonnet-4-5",
            anthropic_json_response(&anthropic_json_fixture!("anthropic-json-tool.1")),
            json!({
                "prompt": anthropic_test_prompt(),
                "providerOptions": {
                    CANONICAL: { "structuredOutputMode": "jsonTool", "disableParallelToolUse": false }
                },
                "responseFormat": { "type": "json", "schema": anthropic_name_schema_with_draft() },
            }),
        )
        .await
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with json tool response (supported model, tool mode) › should pass json schema response format as a tool
            #[tokio::test]
            async fn should_pass_json_schema_response_format_as_a_tool() {
                let (_, seen) = before_each().await;
                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 64000,
                        "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                        "model": "claude-sonnet-4-5",
                        "tool_choice": { "disable_parallel_tool_use": true, "type": "any" },
                        "tools": [{
                            "description": "Respond with a JSON object.",
                            "input_schema": anthropic_name_schema_with_draft(),
                            "name": "json",
                        }],
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with json tool response (supported model, tool mode) › should warn when parallel tool use is requested
            #[tokio::test]
            async fn should_warn_when_parallel_tool_use_is_requested() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.warnings),
                    json!([{
                        "type": "unsupported",
                        "feature": "providerOptions.anthropic.disableParallelToolUse",
                        "details": "`disableParallelToolUse: false` is ignored when using the JSON response tool. Parallel tool use is disabled to ensure a single coherent JSON tool call.",
                    }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with json tool response (supported model, tool mode) › should return the json response
            #[tokio::test]
            async fn should_return_the_json_response() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{ "text": JSON_TOOL_FIXTURE_TEXT, "type": "text" }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with json tool response (supported model, tool mode) › should send stop finish reason
            #[tokio::test]
            async fn should_send_stop_finish_reason() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "raw": "tool_use", "unified": "stop" })
                );
            }
        }

        mod json_schema_response_format_with_other_tool_response_unsupported_model {
            use super::*;

            fn weather_tool_schema() -> Value {
                json!({
                    "type": "object",
                    "properties": { "location": { "type": "string" } },
                    "required": ["location"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#",
                })
            }

            fn response_schema() -> Value {
                json!({
                    "type": "object",
                    "properties": { "weather": { "type": "string" }, "temperature": { "type": "number" } },
                    "required": ["weather", "temperature"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#",
                })
            }

            async fn before_each() -> (GenerateResult, Seen) {
                anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!(
                        "anthropic-json-other-tool.1"
                    )),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "tools": [{
                            "type": "function",
                            "name": "get-weather",
                            "description": "Get the weather in a location",
                            "inputSchema": weather_tool_schema(),
                        }],
                        "responseFormat": { "type": "json", "schema": response_schema() },
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with other tool response (unsupported model) › should pass the tool and the json schema response format as tools
            #[tokio::test]
            async fn should_pass_the_tool_and_the_json_schema_response_format_as_tools() {
                let (_, seen) = before_each().await;
                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                        "model": "claude-3-haiku-20240307",
                        "tool_choice": { "disable_parallel_tool_use": true, "type": "any" },
                        "tools": [
                            {
                                "description": "Get the weather in a location",
                                "input_schema": weather_tool_schema(),
                                "name": "get-weather",
                            },
                            {
                                "description": "Respond with a JSON object.",
                                "input_schema": response_schema(),
                                "name": "json",
                            },
                        ],
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with other tool response (unsupported model) › should return the tool call
            #[tokio::test]
            async fn should_return_the_tool_call() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{
                        "input": r#"{"location":"San Francisco"}"#,
                        "toolCallId": "toolu_01PQjhxo3eirCdKNvCJrKc8f",
                        "toolName": "weather",
                        "type": "tool-call",
                    }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with other tool response (unsupported model) › should send tool-calls finish reason
            #[tokio::test]
            async fn should_send_tool_calls_finish_reason() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "raw": "tool_use", "unified": "tool-calls" })
                );
            }
        }

        mod json_schema_response_format_with_output_format_supported_model {
            use super::*;

            async fn before_each() -> (GenerateResult, Seen) {
                anthropic_generate(
            "claude-sonnet-4-5",
            anthropic_json_response(&anthropic_json_fixture!("anthropic-json-output-format.1")),
            json!({
                "prompt": anthropic_test_prompt(),
                "responseFormat": { "type": "json", "schema": anthropic_name_schema_with_draft() },
            }),
        )
        .await
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with output format (supported model) › should pass json schema response format as output_config.format
            #[tokio::test]
            async fn should_pass_json_schema_response_format_as_output_config_format() {
                let (_, seen) = before_each().await;
                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 64000,
                        "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                        "model": "claude-sonnet-4-5",
                        "output_config": {
                            "format": { "schema": anthropic_name_schema_with_draft(), "type": "json_schema" }
                        },
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with output format (supported model) › should return the json response
            #[tokio::test]
            async fn should_return_the_json_response() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{ "text": JSON_OUTPUT_FORMAT_FIXTURE_TEXT, "type": "text" }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with output format (supported model) › should send stop finish reason
            #[tokio::test]
            async fn should_send_stop_finish_reason() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "raw": "end_turn", "unified": "stop" })
                );
            }
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should use native structured output by default for claude-fable-5
        #[tokio::test]
        async fn should_use_native_structured_output_by_default_for_claude_fable_5() {
            let schema = json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
                "additionalProperties": false,
            });
            let (_, seen) = anthropic_generate(
                "claude-fable-5",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-json-output-format.1")),
                json!({
                    "prompt": anthropic_test_prompt(),
                    "responseFormat": { "type": "json", "schema": schema },
                }),
            )
            .await;

            let body = anthropic_request_body(&seen);
            assert_eq!(
                body["output_config"]["format"],
                json!({ "type": "json_schema", "schema": schema })
            );
            assert!(body.get("tools").is_none());
            assert!(body.get("tool_choice").is_none());
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should sanitize unsupported JSON schema keywords for output format
        #[tokio::test]
        async fn should_sanitize_unsupported_json_schema_keywords_for_output_format() {
            let (_, seen) = anthropic_generate(
        "claude-sonnet-4-5",
        anthropic_json_response(&anthropic_json_fixture!("anthropic-json-output-format.1")),
        json!({
            "prompt": anthropic_test_prompt(),
            "responseFormat": {
                "type": "json",
                "schema": {
                    "type": "object",
                    "properties": {
                        "recurringIntervalMinutes": { "type": "number", "exclusiveMinimum": 0 },
                        "tags": {
                            "type": "array",
                            "minItems": 2,
                            "maxItems": 4,
                            "items": { "type": "string", "minLength": 1 },
                        },
                    },
                    "required": ["recurringIntervalMinutes", "tags"],
                    "additionalProperties": false,
                },
            },
        }),
    )
    .await;

            assert_eq!(
                anthropic_request_body(&seen)["output_config"]["format"]["schema"],
                json!({
                    "additionalProperties": false,
                    "properties": {
                        "recurringIntervalMinutes": {
                            "description": "exclusive minimum: 0.",
                            "type": "number",
                        },
                        "tags": {
                            "description": "min items: 2; max items: 4.",
                            "items": { "description": "min length: 1.", "type": "string" },
                            "type": "array",
                        },
                    },
                    "required": ["recurringIntervalMinutes", "tags"],
                    "type": "object",
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should pass sanitized zod output schema as output_config.format
        #[tokio::test]
        async fn should_pass_sanitized_zod_output_schema_as_output_config_format() {
            // `asSchema(z.object(...)).jsonSchema`: the draft-07 schema zod 4 emits.
            let schema = json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": {
                    "recipe": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "ingredients": {
                                "minItems": 10,
                                "maxItems": 12,
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string" },
                                        "amount": { "type": "string" },
                                    },
                                    "required": ["name", "amount"],
                                    "additionalProperties": false,
                                },
                            },
                            "steps": { "type": "array", "items": { "type": "string" } },
                        },
                        "required": ["name", "ingredients", "steps"],
                        "additionalProperties": false,
                    },
                },
                "required": ["recipe"],
                "additionalProperties": false,
            });
            let (_, seen) = anthropic_generate(
                "claude-sonnet-4-5",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-json-output-format.1")),
                json!({
                    "prompt": anthropic_test_prompt(),
                    "responseFormat": { "type": "json", "schema": schema },
                }),
            )
            .await;

            assert_eq!(
                anthropic_request_body(&seen)["output_config"],
                json!({
                    "format": {
                        "schema": {
                            "$schema": "http://json-schema.org/draft-07/schema#",
                            "additionalProperties": false,
                            "properties": {
                                "recipe": {
                                    "additionalProperties": false,
                                    "properties": {
                                        "ingredients": {
                                            "description": "min items: 10; max items: 12.",
                                            "items": {
                                                "additionalProperties": false,
                                                "properties": {
                                                    "amount": { "type": "string" },
                                                    "name": { "type": "string" },
                                                },
                                                "required": ["name", "amount"],
                                                "type": "object",
                                            },
                                            "type": "array",
                                        },
                                        "name": { "type": "string" },
                                        "steps": { "items": { "type": "string" }, "type": "array" },
                                    },
                                    "required": ["name", "ingredients", "steps"],
                                    "type": "object",
                                },
                            },
                            "required": ["recipe"],
                            "type": "object",
                        },
                        "type": "json_schema",
                    },
                })
            );
        }

        mod json_schema_response_format_with_output_format_unknown_model_forced {
            use super::*;

            async fn before_each() -> (GenerateResult, Seen) {
                anthropic_generate(
            "claude-unknown",
            anthropic_json_response(&anthropic_json_fixture!("anthropic-json-output-format.1")),
            json!({
                "prompt": anthropic_test_prompt(),
                "providerOptions": { CANONICAL: { "structuredOutputMode": "outputFormat" } },
                "responseFormat": { "type": "json", "schema": anthropic_name_schema_with_draft() },
            }),
        )
        .await
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with output format (unknown model, forced) › should pass json schema response format as output_config.format
            #[tokio::test]
            async fn should_pass_json_schema_response_format_as_output_config_format() {
                let (_, seen) = before_each().await;
                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 128000,
                        "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                        "model": "claude-unknown",
                        "output_config": {
                            "format": { "schema": anthropic_name_schema_with_draft(), "type": "json_schema" }
                        },
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with output format (unknown model, forced) › should return the json response
            #[tokio::test]
            async fn should_return_the_json_response() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{ "text": JSON_OUTPUT_FORMAT_FIXTURE_TEXT, "type": "text" }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › json schema response format with output format (unknown model, forced) › should send stop finish reason
            #[tokio::test]
            async fn should_send_stop_finish_reason() {
                let (result, _) = before_each().await;
                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "raw": "end_turn", "unified": "stop" })
                );
            }
        }

        mod structured_outputs_2025_11_13_beta_header {
            use super::*;

            fn headers_without_beta() -> Value {
                json!({
                    "anthropic-version": "2023-06-01",
                    "content-type": "application/json",
                    "x-api-key": "test-api-key",
                })
            }

            fn headers_with_beta() -> Value {
                json!({
                    "anthropic-beta": "structured-outputs-2025-11-13",
                    "anthropic-version": "2023-06-01",
                    "content-type": "application/json",
                    "x-api-key": "test-api-key",
                })
            }

            fn name_schema() -> Value {
                json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"],
                })
            }

            fn test_tool(strict: Option<bool>) -> Value {
                let mut tool = json!({
                    "type": "function",
                    "name": "testTool",
                    "description": "A test tool",
                    "inputSchema": { "type": "object", "properties": { "value": { "type": "string" } } },
                });
                if let Some(strict) = strict {
                    tool["strict"] = json!(strict);
                }
                tool
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › structured-outputs-2025-11-13 beta header › should NOT include beta header for simple text generation with supported model
            #[tokio::test]
            async fn should_not_include_beta_header_for_simple_text_generation_with_supported_model()
             {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;
                assert_eq!(anthropic_request_headers(&seen), headers_without_beta());
                assert_anthropic_user_agent(&seen);
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › structured-outputs-2025-11-13 beta header › should NOT include beta header when using json schema response format (structured outputs are GA)
            #[tokio::test]
            async fn should_not_include_beta_header_when_using_json_schema_response_format_ga() {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!(
                        "anthropic-json-output-format.1"
                    )),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "responseFormat": { "type": "json", "schema": name_schema() },
                    }),
                )
                .await;
                assert_eq!(anthropic_request_headers(&seen), headers_without_beta());
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › structured-outputs-2025-11-13 beta header › should NOT include beta header when using json schema response format with unsupported model
            #[tokio::test]
            async fn should_not_include_beta_header_when_using_json_schema_with_unsupported_model()
            {
                let (_, seen) = anthropic_generate(
                    "claude-3-haiku-20240307",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "responseFormat": { "type": "json", "schema": name_schema() },
                    }),
                )
                .await;
                assert_eq!(anthropic_request_headers(&seen), headers_without_beta());
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › structured-outputs-2025-11-13 beta header › should include beta header when using tools with strict: true on supported model
            #[tokio::test]
            async fn should_include_beta_header_when_using_tools_with_strict_true_on_supported_model()
             {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "tools": [test_tool(Some(true))] }),
                )
                .await;
                assert_eq!(anthropic_request_headers(&seen), headers_with_beta());
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › structured-outputs-2025-11-13 beta header › should include beta header when using tools with strict: false on supported model
            #[tokio::test]
            async fn should_include_beta_header_when_using_tools_with_strict_false_on_supported_model()
             {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "tools": [test_tool(Some(false))] }),
                )
                .await;
                assert_eq!(anthropic_request_headers(&seen), headers_with_beta());
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › structured-outputs-2025-11-13 beta header › should include beta header when using tools without strict on supported model
            #[tokio::test]
            async fn should_include_beta_header_when_using_tools_without_strict_on_supported_model()
            {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "tools": [test_tool(None)] }),
                )
                .await;
                assert_eq!(anthropic_request_headers(&seen), headers_with_beta());
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › structured-outputs-2025-11-13 beta header › should NOT include beta header when using json response tool (jsonTool mode)
            #[tokio::test]
            async fn should_not_include_beta_header_when_using_json_response_tool_json_tool_mode() {
                let (_, seen) = anthropic_generate(
                    "claude-sonnet-4-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-json-tool.1")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "providerOptions": { CANONICAL: { "structuredOutputMode": "jsonTool" } },
                        "responseFormat": { "type": "json", "schema": name_schema() },
                    }),
                )
                .await;
                assert_eq!(anthropic_request_headers(&seen), headers_without_beta());
            }
        }
        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should extract text response
        #[tokio::test]
        async fn should_extract_text_response() {
            let (result, _) = anthropic_generate_default(
                anthropic_json_response(&json!({
                    "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Hello, World!" }],
                    "model": "claude-3-haiku-20240307",
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": { "input_tokens": 4, "output_tokens": 30 },
                })),
                json!({ "prompt": anthropic_test_prompt() }),
            )
            .await;

            assert_eq!(
                anthropic_to_json(&result.content),
                json!([{ "text": "Hello, World!", "type": "text" }])
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should extract usage
        #[tokio::test]
        async fn should_extract_usage() {
            let (result, _) = anthropic_generate_default(
                anthropic_json_response(&json!({
                    "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "" }],
                    "model": "claude-3-haiku-20240307",
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": { "input_tokens": 20, "output_tokens": 5 },
                })),
                json!({ "prompt": anthropic_test_prompt() }),
            )
            .await;

            assert_eq!(
                anthropic_to_json(&result.usage),
                json!({
                    "inputTokens": { "cacheRead": 0, "cacheWrite": 0, "noCache": 20, "total": 20 },
                    "outputTokens": { "total": 5 },
                    "raw": { "input_tokens": 20, "output_tokens": 5 },
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should send additional response information
        #[tokio::test]
        async fn should_send_additional_response_information() {
            let (result, _) = anthropic_generate_default(
                anthropic_json_response(&json!({
                    "id": "test-id",
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "" }],
                    "model": "test-model",
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": { "input_tokens": 4, "output_tokens": 30 },
                })),
                json!({ "prompt": anthropic_test_prompt() }),
            )
            .await;

            assert_eq!(
                anthropic_to_json(&result.response),
                json!({
                    "body": {
                        "content": [{ "text": "", "type": "text" }],
                        "id": "test-id",
                        "model": "test-model",
                        "role": "assistant",
                        "stop_reason": "end_turn",
                        "stop_sequence": null,
                        "type": "message",
                        "usage": { "input_tokens": 4, "output_tokens": 30 },
                    },
                    "headers": { "content-length": "203", "content-type": "application/json" },
                    "id": "test-id",
                    "modelId": "test-model",
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should include stop_sequence in provider metadata
        #[tokio::test]
        async fn should_include_stop_sequence_in_provider_metadata() {
            let (result, _) = anthropic_generate_default(
                anthropic_json_response(&json!({
                    "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Hello, World!" }],
                    "model": "claude-3-haiku-20240307",
                    "stop_reason": "stop_sequence",
                    "stop_sequence": "STOP",
                    "usage": { "input_tokens": 4, "output_tokens": 30 },
                })),
                json!({ "prompt": anthropic_test_prompt(), "stopSequences": ["STOP"] }),
            )
            .await;

            assert_eq!(
                anthropic_to_json(&result.provider_metadata),
                json!({
                    CANONICAL: {
                        "container": null,
                        "contextManagement": null,
                        "iterations": null,
                        "stopSequence": "STOP",
                        "usage": { "input_tokens": 4, "output_tokens": 30 },
                    },
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should omit preserved thinking input transformations when absent
        #[tokio::test]
        async fn should_omit_preserved_thinking_input_transformations_when_absent() {
            let (result, _) = anthropic_generate(
                "claude-fable-5-1",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                json!({ "prompt": anthropic_test_prompt() }),
            )
            .await;

            let metadata = anthropic_to_json(&result.provider_metadata);
            assert!(metadata[CANONICAL].get("inputTransformations").is_none());
        }

        mod refusal_stop_reason {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › refusal stop reason › should map a classifier refusal to content-filter and expose stop details
            #[tokio::test]
            async fn should_map_a_classifier_refusal_to_content_filter_and_expose_stop_details() {
                let (result, _) = anthropic_generate(
                    "claude-fable-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-refusal")),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "raw": "refusal", "unified": "content-filter" })
                );
                assert_eq!(
                    anthropic_to_json(&result.provider_metadata)[CANONICAL]["stopDetails"],
                    json!({
                        "category": "cyber",
                        "explanation": "This request triggered restrictions on violative cyber content and was blocked under Anthropic's Usage Policy.",
                        "recommendedModel": "claude-fable-5",
                        "type": "refusal",
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › refusal stop reason › should map a refusal without stop details to content-filter and omit stop details
            #[tokio::test]
            async fn should_map_a_refusal_without_stop_details_to_content_filter_and_omit_details()
            {
                let (result, _) = anthropic_generate(
                    "claude-fable-5",
                    anthropic_json_response(&anthropic_json_fixture!(
                        "anthropic-refusal-no-details"
                    )),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "raw": "refusal", "unified": "content-filter" })
                );
                assert!(
                    anthropic_to_json(&result.provider_metadata)[CANONICAL]
                        .get("stopDetails")
                        .is_none()
                );
            }
        }

        mod fallbacks {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › fallbacks › should pass fallbacks to the request body and add the beta header
            #[tokio::test]
            async fn should_pass_fallbacks_to_the_request_body_and_add_the_beta_header() {
                let (_, seen) = anthropic_generate(
                    "claude-fable-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "maxOutputTokens": 1024,
                        "providerOptions": {
                            CANONICAL: {
                                "fallbacks": [{
                                    "model": "claude-opus-4-8",
                                    "max_tokens": 8192,
                                    "thinking": { "type": "disabled" },
                                    "speed": "fast",
                                }],
                            },
                        },
                    }),
                )
                .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "fallbacks": [{
                            "max_tokens": 8192,
                            "model": "claude-opus-4-8",
                            "speed": "fast",
                            "thinking": { "type": "disabled" },
                        }],
                        "max_tokens": 1024,
                        "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                        "model": "claude-fable-5",
                    })
                );
                assert_eq!(
                    seen.headers.get("anthropic-beta").map(String::as_str),
                    Some("server-side-fallback-2026-06-01")
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › fallbacks › should pass fallbacks "default" through and add the 2026-07-01 beta header
            #[tokio::test]
            async fn should_pass_fallbacks_default_through_and_add_the_2026_07_01_beta_header() {
                let (_, seen) = anthropic_generate(
                    "claude-fable-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "providerOptions": { CANONICAL: { "fallbacks": "default" } },
                    }),
                )
                .await;

                assert_eq!(anthropic_request_body(&seen)["fallbacks"], json!("default"));
                assert_eq!(
                    seen.headers.get("anthropic-beta").map(String::as_str),
                    Some("server-side-fallback-2026-07-01")
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › fallbacks › should not add the beta header when fallbacks is an empty array
            #[tokio::test]
            async fn should_not_add_the_beta_header_when_fallbacks_is_an_empty_array() {
                let (_, seen) = anthropic_generate(
                    "claude-fable-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "providerOptions": { CANONICAL: { "fallbacks": [] } },
                    }),
                )
                .await;

                assert!(anthropic_request_body(&seen).get("fallbacks").is_none());
                assert!(!seen.headers.contains_key("anthropic-beta"));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › fallbacks › should preserve the fallback content block and surface the fallback iteration
            #[tokio::test]
            async fn should_preserve_the_fallback_content_block_and_surface_the_fallback_iteration()
            {
                let (result, _) = anthropic_generate(
                    "claude-fable-5",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-fallback")),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        {
                            "kind": "anthropic.fallback",
                            "providerMetadata": {
                                CANONICAL: {
                                    "from": { "model": "claude-fable-5" },
                                    "to": { "model": "claude-opus-4-8" },
                                    "type": "fallback",
                                },
                            },
                            "type": "custom",
                        },
                        {
                            "text": "The printing press was invented by Johannes Gutenberg around 1440.",
                            "type": "text",
                        },
                    ])
                );
                // The fallback hop is preserved in the Anthropic-specific iterations.
                assert_eq!(
                    anthropic_to_json(&result.provider_metadata)[CANONICAL]["iterations"],
                    json!([
                        { "inputTokens": 408, "model": "claude-fable-5", "outputTokens": 0, "type": "message" },
                        {
                            "inputTokens": 412,
                            "model": "claude-opus-4-8",
                            "outputTokens": 264,
                            "type": "fallback_message",
                        },
                    ])
                );
                // Top-level usage reflects the served (fallback) answer, not the
                // blocked primary attempt (which had 0 output tokens).
                assert_eq!(result.usage.input_tokens.total, Some(412));
                assert_eq!(result.usage.output_tokens.total, Some(264));
            }
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should expose the raw response headers
        #[tokio::test]
        async fn should_expose_the_raw_response_headers() {
            let (result, seen) = anthropic_generate_default(
                anthropic_json_response_with_headers(
                    &json!({
                        "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "text", "text": "" }],
                        "model": "claude-3-haiku-20240307",
                        "stop_reason": "end_turn",
                        "stop_sequence": null,
                        "usage": { "input_tokens": 4, "output_tokens": 30 },
                    }),
                    &[("test-header", "test-value")],
                ),
                json!({ "prompt": anthropic_test_prompt() }),
            )
            .await;

            assert_eq!(
                anthropic_to_json(&result.response.unwrap().headers),
                json!({
                    // default headers:
                    "content-length": "237",
                    "content-type": "application/json",
                    // custom header
                    "test-header": "test-value",
                })
            );
            // Upstream checks `ai-sdk-anthropic/0.0.0-test` (mocked version); aimux
            // pins the released version.
            assert!(seen.headers["user-agent"].contains("ai-sdk-anthropic/4.0.71"));
            assert_anthropic_user_agent(&seen);
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should send the model id and settings
        #[tokio::test]
        async fn should_send_the_model_id_and_settings() {
            let (result, seen) = anthropic_generate_default(
                anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                json!({
                    "prompt": anthropic_test_prompt(),
                    "temperature": 0.5,
                    "maxOutputTokens": 100,
                    "topK": 0.1,
                    "stopSequences": ["abc", "def"],
                    "frequencyPenalty": 0.15,
                }),
            )
            .await;

            assert_eq!(
                anthropic_request_body(&seen),
                json!({
                    "max_tokens": 100,
                    "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                    "model": "claude-3-haiku-20240307",
                    "stop_sequences": ["abc", "def"],
                    "temperature": 0.5,
                    "top_k": 0.1,
                })
            );
            assert_eq!(
                anthropic_to_json(&result.warnings),
                json!([{ "feature": "frequencyPenalty", "type": "unsupported" }])
            );
        }

        mod temperature_and_top_p_mutual_exclusivity {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › temperature and topP mutual exclusivity › should only send temperature when both temperature and topP are provided
            #[tokio::test]
            async fn should_only_send_temperature_when_both_temperature_and_top_p_are_provided() {
                let (result, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "temperature": 0.7, "topP": 0.9 }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert_eq!(body["temperature"], json!(0.7));
                assert!(body.get("top_p").is_none());
                assert_anthropic_contains(
                    &anthropic_to_json(&result.warnings),
                    &json!({
                        "type": "unsupported",
                        "feature": "topP",
                        "details": "topP is not supported when temperature is set. topP is ignored.",
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › temperature and topP mutual exclusivity › should send temperature when only temperature is provided
            #[tokio::test]
            async fn should_send_temperature_when_only_temperature_is_provided() {
                let (_, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "temperature": 0.7 }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert_eq!(body["temperature"], json!(0.7));
                assert!(body.get("top_p").is_none());
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › temperature and topP mutual exclusivity › should send topP when only topP is provided
            #[tokio::test]
            async fn should_send_top_p_when_only_top_p_is_provided() {
                let (_, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "topP": 0.9 }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert!(body.get("temperature").is_none());
                assert_eq!(body["top_p"], json!(0.9));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › temperature and topP mutual exclusivity › should not send temperature or topP when neither is provided
            #[tokio::test]
            async fn should_not_send_temperature_or_top_p_when_neither_is_provided() {
                let (_, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert!(body.get("temperature").is_none());
                assert!(body.get("top_p").is_none());
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › temperature and topP mutual exclusivity › should send both temperature and topP for non-Anthropic models
            #[tokio::test]
            async fn should_send_both_temperature_and_top_p_for_non_anthropic_models() {
                let (result, seen) = anthropic_generate(
                    "MiniMax-M2.7",
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "temperature": 0.7, "topP": 0.9 }),
                )
                .await;

                let body = anthropic_request_body(&seen);
                assert_eq!(body["temperature"], json!(0.7));
                assert_eq!(body["top_p"], json!(0.9));
                let warnings = anthropic_to_json(&result.warnings);
                assert!(
                    !warnings
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|warning| warning.get("feature") == Some(&json!("topP"))),
                    "{warnings:#}"
                );
            }
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should limit max output tokens to the model max and warn
        #[tokio::test]
        async fn should_limit_max_output_tokens_to_the_model_max_and_warn() {
            let (result, seen) = anthropic_generate(
                "claude-haiku-4-5",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                json!({ "prompt": anthropic_test_prompt(), "maxOutputTokens": 999999 }),
            )
            .await;

            assert_eq!(
                anthropic_request_body(&seen),
                json!({
                    "max_tokens": 64000,
                    "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                    "model": "claude-haiku-4-5",
                })
            );
            assert_eq!(
                anthropic_to_json(&result.warnings),
                json!([{
                    "details": "999999 (maxOutputTokens + thinkingBudget) is greater than claude-haiku-4-5 64000 max output tokens. The max output tokens have been limited to 64000.",
                    "feature": "maxOutputTokens",
                    "type": "unsupported",
                }])
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should not limit max output tokens for unknown models
        #[tokio::test]
        async fn should_not_limit_max_output_tokens_for_unknown_models() {
            let (result, seen) = anthropic_generate(
                "future-model",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                json!({ "prompt": anthropic_test_prompt(), "maxOutputTokens": 123456 }),
            )
            .await;

            assert_eq!(
                anthropic_request_body(&seen),
                json!({
                    "max_tokens": 123456,
                    "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                    "model": "future-model",
                })
            );
            assert_eq!(anthropic_to_json(&result.warnings), json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should use default thinking budget when it is not set
        #[tokio::test]
        async fn should_use_default_thinking_budget_when_it_is_not_set() {
            let (result, seen) = anthropic_generate(
                "claude-haiku-4-5",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                json!({
                    "prompt": anthropic_test_prompt(),
                    "providerOptions": { CANONICAL: { "thinking": { "type": "enabled" } } },
                }),
            )
            .await;

            assert_eq!(
                anthropic_request_body(&seen),
                json!({
                    "max_tokens": 64000,
                    "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                    "model": "claude-haiku-4-5",
                    "thinking": { "budget_tokens": 1024, "type": "enabled" },
                })
            );
            assert_eq!(
                anthropic_to_json(&result.warnings),
                json!([{
                    "details": "thinking budget is required when thinking is enabled. using default budget of 1024 tokens.",
                    "feature": "extended thinking",
                    "type": "compatibility",
                }])
            );
        }

        fn anthropic_test_tool_value_schema() -> Value {
            json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"],
                "additionalProperties": false,
                "$schema": "http://json-schema.org/draft-07/schema#",
            })
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should pass tools and toolChoice
        #[tokio::test]
        async fn should_pass_tools_and_tool_choice() {
            let (_, seen) = anthropic_generate_default(
                anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                json!({
                    "tools": [{
                        "type": "function",
                        "name": "test-tool",
                        "inputSchema": anthropic_test_tool_value_schema(),
                    }],
                    "toolChoice": { "type": "tool", "toolName": "test-tool" },
                    "prompt": anthropic_test_prompt(),
                }),
            )
            .await;

            assert_eq!(
                anthropic_request_body(&seen),
                json!({
                    "model": "claude-3-haiku-20240307",
                    "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }],
                    "max_tokens": 4096,
                    "tools": [{ "name": "test-tool", "input_schema": anthropic_test_tool_value_schema() }],
                    "tool_choice": { "type": "tool", "name": "test-tool" },
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should pass disableParallelToolUse
        #[tokio::test]
        async fn should_pass_disable_parallel_tool_use() {
            let (_, seen) = anthropic_generate_default(
                anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                json!({
                    "tools": [{
                        "type": "function",
                        "name": "test-tool",
                        "inputSchema": anthropic_test_tool_value_schema(),
                    }],
                    "prompt": anthropic_test_prompt(),
                    "providerOptions": { CANONICAL: { "disableParallelToolUse": true } },
                }),
            )
            .await;

            assert_anthropic_match_object(
                &anthropic_request_body(&seen),
                &json!({ "tool_choice": { "type": "auto", "disable_parallel_tool_use": true } }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let mock = MockFetch::new(vec![anthropic_json_response(&anthropic_json_fixture!(
                "anthropic-text"
            ))]);
            let provider = create_anthropic(AnthropicProviderSettings {
                api_key: Some("test-api-key".into()),
                headers: Some(
                    [(
                        "Custom-Provider-Header".to_string(),
                        Some("provider-header-value".to_string()),
                    )]
                    .into_iter()
                    .collect(),
                ),
                fetch: Some(mock.transport()),
                ..Default::default()
            })
            .unwrap();

            provider
                .messages("claude-3-haiku-20240307")
                .do_generate(&anthropic_call_options(json!({
                    "prompt": anthropic_test_prompt(),
                    "headers": { "Custom-Request-Header": "request-header-value" },
                })))
                .await
                .unwrap();

            let seen = &mock.seen()[0];
            assert_eq!(
                anthropic_request_headers(seen),
                json!({
                    "anthropic-version": "2023-06-01",
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                    "x-api-key": "test-api-key",
                })
            );
            assert_anthropic_user_agent(seen);
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should support cache control
        #[tokio::test]
        async fn should_support_cache_control() {
            let (result, seen) = anthropic_generate_default(
        anthropic_json_response(&json!({
            "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "" }],
            "model": "claude-3-haiku-20240307",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 20,
                "output_tokens": 50,
                "cache_creation_input_tokens": 10,
                "cache_read_input_tokens": 5,
            },
        })),
        json!({
            "prompt": [{
                "role": "user",
                "content": [{ "type": "text", "text": "Hello" }],
                "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
            }],
        }),
    )
    .await;

            assert_eq!(
                anthropic_request_body(&seen),
                json!({
                    "model": "claude-3-haiku-20240307",
                    "messages": [{
                        "role": "user",
                        "content": [{ "type": "text", "text": "Hello", "cache_control": { "type": "ephemeral" } }],
                    }],
                    "max_tokens": 4096,
                })
            );
            assert_eq!(
                anthropic_to_json(&result),
                json!({
                    "content": [{ "text": "", "type": "text" }],
                    "finishReason": { "raw": "end_turn", "unified": "stop" },
                    "providerMetadata": {
                        CANONICAL: {
                            "container": null,
                            "contextManagement": null,
                            "iterations": null,
                            "stopSequence": null,
                            "usage": {
                                "cache_creation_input_tokens": 10,
                                "cache_read_input_tokens": 5,
                                "input_tokens": 20,
                                "output_tokens": 50,
                            },
                        },
                    },
                    "request": {
                        "body": {
                            "max_tokens": 4096,
                            "messages": [{
                                "content": [{
                                    "cache_control": { "type": "ephemeral" },
                                    "text": "Hello",
                                    "type": "text",
                                }],
                                "role": "user",
                            }],
                            "model": "claude-3-haiku-20240307",
                        },
                    },
                    "response": {
                        "body": {
                            "content": [{ "text": "", "type": "text" }],
                            "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                            "model": "claude-3-haiku-20240307",
                            "role": "assistant",
                            "stop_reason": "end_turn",
                            "stop_sequence": null,
                            "type": "message",
                            "usage": {
                                "cache_creation_input_tokens": 10,
                                "cache_read_input_tokens": 5,
                                "input_tokens": 20,
                                "output_tokens": 50,
                            },
                        },
                        "headers": { "content-length": "299", "content-type": "application/json" },
                        "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                        "modelId": "claude-3-haiku-20240307",
                    },
                    "usage": {
                        "inputTokens": { "cacheRead": 5, "cacheWrite": 10, "noCache": 20, "total": 35 },
                        "outputTokens": { "total": 50 },
                        "raw": {
                            "cache_creation_input_tokens": 10,
                            "cache_read_input_tokens": 5,
                            "input_tokens": 20,
                            "output_tokens": 50,
                        },
                    },
                    "warnings": [],
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should support cache control and return extra fields in provider metadata
        #[tokio::test]
        async fn should_support_cache_control_and_return_extra_fields_in_provider_metadata() {
            let usage = json!({
                "input_tokens": 20,
                "output_tokens": 50,
                "cache_creation_input_tokens": 10,
                "cache_read_input_tokens": 5,
                "cache_creation": { "ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 10 },
            });
            let (result, seen) = anthropic_generate_default(
                anthropic_json_response(&json!({
                    "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "" }],
                    "model": "claude-3-haiku-20240307",
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": usage,
                })),
                json!({
                    "prompt": [{
                        "role": "user",
                        "content": [{ "type": "text", "text": "Hello" }],
                        "providerOptions": {
                            CANONICAL: { "cacheControl": { "type": "ephemeral", "ttl": "1h" } }
                        },
                    }],
                }),
            )
            .await;

            assert_eq!(
                anthropic_request_body(&seen),
                json!({
                    "model": "claude-3-haiku-20240307",
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "text",
                            "text": "Hello",
                            "cache_control": { "type": "ephemeral", "ttl": "1h" },
                        }],
                    }],
                    "max_tokens": 4096,
                })
            );
            assert_eq!(
                anthropic_to_json(&result),
                json!({
                    "content": [{ "text": "", "type": "text" }],
                    "finishReason": { "raw": "end_turn", "unified": "stop" },
                    "providerMetadata": {
                        CANONICAL: {
                            "container": null,
                            "contextManagement": null,
                            "iterations": null,
                            "stopSequence": null,
                            "usage": usage,
                        },
                    },
                    "request": {
                        "body": {
                            "max_tokens": 4096,
                            "messages": [{
                                "content": [{
                                    "cache_control": { "ttl": "1h", "type": "ephemeral" },
                                    "text": "Hello",
                                    "type": "text",
                                }],
                                "role": "user",
                            }],
                            "model": "claude-3-haiku-20240307",
                        },
                    },
                    "response": {
                        "body": {
                            "content": [{ "text": "", "type": "text" }],
                            "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                            "model": "claude-3-haiku-20240307",
                            "role": "assistant",
                            "stop_reason": "end_turn",
                            "stop_sequence": null,
                            "type": "message",
                            "usage": usage,
                        },
                        "headers": { "content-length": "379", "content-type": "application/json" },
                        "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                        "modelId": "claude-3-haiku-20240307",
                    },
                    "usage": {
                        "inputTokens": { "cacheRead": 5, "cacheWrite": 10, "noCache": 20, "total": 35 },
                        "outputTokens": { "total": 50 },
                        "raw": usage,
                    },
                    "warnings": [],
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should send request body
        #[tokio::test]
        async fn should_send_request_body() {
            let (result, _) = anthropic_generate_default(
                anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                json!({ "prompt": anthropic_test_prompt() }),
            )
            .await;

            assert_eq!(
                anthropic_to_json(&result.request),
                json!({
                    "body": {
                        "max_tokens": 4096,
                        "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                        "model": "claude-3-haiku-20240307",
                    },
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should process PDF citation responses
        #[tokio::test]
        async fn should_process_pdf_citation_responses() {
            let (result, _) = anthropic_generate_default(
        anthropic_json_response(&json!({
            "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Based on the document, the results show positive growth.",
                "citations": [{
                    "type": "page_location",
                    "cited_text": "Revenue increased by 25% year over year",
                    "document_index": 0,
                    "document_title": "Financial Report 2023",
                    "start_page_number": 5,
                    "end_page_number": 6,
                }],
            }],
            "model": "claude-3-haiku-20240307",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": { "input_tokens": 4, "output_tokens": 30 },
        })),
        json!({
            "prompt": [{
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "base64PDFdata" },
                        "mediaType": "application/pdf",
                        "filename": "financial-report.pdf",
                        "providerOptions": { CANONICAL: { "citations": { "enabled": true } } },
                    },
                    { "type": "text", "text": "What do the results show?" },
                ],
            }],
        }),
    )
    .await;

            assert_eq!(
                anthropic_to_json(&result.content),
                json!([
                    {
                        "text": "Based on the document, the results show positive growth.",
                        "type": "text",
                    },
                    {
                        "filename": "financial-report.pdf",
                        "id": "id-0",
                        "mediaType": "application/pdf",
                        "providerMetadata": {
                            CANONICAL: {
                                "citedText": "Revenue increased by 25% year over year",
                                "endPageNumber": 6,
                                "startPageNumber": 5,
                            },
                        },
                        "sourceType": "document",
                        "title": "Financial Report 2023",
                        "type": "source",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should process text citation responses
        #[tokio::test]
        async fn should_process_text_citation_responses() {
            let mock = MockFetch::new(vec![anthropic_json_response(&json!({
                "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "The text shows important information.",
                    "citations": [{
                        "type": "char_location",
                        "cited_text": "important information",
                        "document_index": 0,
                        "document_title": "Test Document",
                        "start_char_index": 15,
                        "end_char_index": 35,
                    }],
                }],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 4, "output_tokens": 30 },
            }))]);
            let mock_provider = create_anthropic(AnthropicProviderSettings {
                api_key: Some("test-api-key".into()),
                fetch: Some(mock.transport()),
                generate_id: Some(Arc::new(|| "test-text-citation-id".to_string())),
                ..Default::default()
            })
            .unwrap();

            let result = mock_provider
        .messages("claude-3-haiku-20240307")
        .do_generate(&anthropic_call_options(json!({
            "prompt": [{
                "role": "user",
                "content": [
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "VGVzdCBkb2N1bWVudCBjb250ZW50" },
                        "mediaType": "text/plain",
                        "filename": "test.txt",
                        "providerOptions": { CANONICAL: { "citations": { "enabled": true } } },
                    },
                    { "type": "text", "text": "What does this say?" },
                ],
            }],
        })))
        .await
        .unwrap();

            assert_eq!(
                anthropic_to_json(&result.content),
                json!([
                    { "text": "The text shows important information.", "type": "text" },
                    {
                        "filename": "test.txt",
                        "id": "test-text-citation-id",
                        "mediaType": "text/plain",
                        "providerMetadata": {
                            CANONICAL: {
                                "citedText": "important information",
                                "endCharIndex": 35,
                                "startCharIndex": 15,
                            },
                        },
                        "sourceType": "document",
                        "title": "Test Document",
                        "type": "source",
                    },
                ])
            );
        }
        mod function_tool {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › function tool › should extract tool calls
            #[tokio::test]
            async fn should_extract_tool_calls() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&json!({
                        "id": "msg_017TfcQ4AgGxKyBduUpqYPZn",
                        "type": "message",
                        "role": "assistant",
                        "content": [
                            { "type": "text", "text": "Some text\n\n" },
                            {
                                "type": "tool_use",
                                "id": "toolu_1",
                                "name": "test-tool",
                                "input": { "value": "example value" },
                            },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stop_reason": "tool_use",
                        "stop_sequence": null,
                        "usage": { "input_tokens": 4, "output_tokens": 30 },
                    })),
                    json!({
                        "tools": [{
                            "type": "function",
                            "name": "test-tool",
                            "inputSchema": anthropic_test_tool_value_schema(),
                        }],
                        "prompt": anthropic_test_prompt(),
                    }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        { "text": "Some text\n\n", "type": "text" },
                        {
                            "input": r#"{"value":"example value"}"#,
                            "toolCallId": "toolu_1",
                            "toolName": "test-tool",
                            "type": "tool-call",
                        },
                    ])
                );
                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "raw": "tool_use", "unified": "tool-calls" })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › function tool › should support tools with empty parameters
            #[tokio::test]
            async fn should_support_tools_with_empty_parameters() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-tool-no-args")),
                    json!({
                        "tools": [{
                            "type": "function",
                            "name": "test-tool",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                                "required": [],
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        }],
                        "prompt": anthropic_test_prompt(),
                    }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        {
                            "text": "<thinking>\nThe updateIssueList tool was provided in the list of available functions. The tool has no required parameters, so it can be called without any additional information needed from the user.\n</thinking>\n\nOkay, I will update the current issue list:",
                            "type": "text",
                        },
                        {
                            "input": "{}",
                            "toolCallId": "toolu_01LRmxn9vGM1d2DZSDBowdZ1",
                            "toolName": "updateIssueList",
                            "type": "tool-call",
                        },
                    ])
                );
            }
        }

        mod programmatic_tool_calling {
            use super::*;

            fn tool_use_response(tool_use: Value) -> Canned {
                anthropic_json_response(&json!({
                    "id": "msg_01Test",
                    "type": "message",
                    "role": "assistant",
                    "content": [tool_use],
                    "model": "claude-3-haiku-20240307",
                    "stop_reason": "tool_use",
                    "usage": { "input_tokens": 100, "output_tokens": 50 },
                }))
            }

            fn function_tool(name: &str, property: &str) -> Value {
                json!({
                    "type": "function",
                    "name": name,
                    "inputSchema": { "type": "object", "properties": { property: { "type": "string" } } },
                })
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › programmatic tool calling › should include caller info when tool_use has caller field from code_execution
            #[tokio::test]
            async fn should_include_caller_info_when_tool_use_has_caller_field_from_code_execution()
            {
                let (result, _) = anthropic_generate_default(
            tool_use_response(json!({
                "type": "tool_use",
                "id": "toolu_01Test",
                "name": "query_database",
                "input": { "sql": "SELECT * FROM users" },
                "caller": { "type": "code_execution_20250825", "tool_id": "srvtoolu_01CodeExec" },
            })),
            json!({
                "tools": [function_tool("query_database", "sql")],
                "prompt": anthropic_test_prompt(),
            }),
        )
        .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{
                        "input": r#"{"sql":"SELECT * FROM users"}"#,
                        "providerMetadata": {
                            CANONICAL: {
                                "caller": { "toolId": "srvtoolu_01CodeExec", "type": "code_execution_20250825" },
                            },
                        },
                        "toolCallId": "toolu_01Test",
                        "toolName": "query_database",
                        "type": "tool-call",
                    }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › programmatic tool calling › should include caller info when tool_use has direct caller type
            #[tokio::test]
            async fn should_include_caller_info_when_tool_use_has_direct_caller_type() {
                let (result, _) = anthropic_generate_default(
                    tool_use_response(json!({
                        "type": "tool_use",
                        "id": "toolu_01Test",
                        "name": "get_weather",
                        "input": { "city": "Tokyo" },
                        "caller": { "type": "direct" },
                    })),
                    json!({
                        "tools": [function_tool("get_weather", "city")],
                        "prompt": anthropic_test_prompt(),
                    }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{
                        "input": r#"{"city":"Tokyo"}"#,
                        "providerMetadata": { CANONICAL: { "caller": { "type": "direct" } } },
                        "toolCallId": "toolu_01Test",
                        "toolName": "get_weather",
                        "type": "tool-call",
                    }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › programmatic tool calling › should not include caller info when tool_use has no caller field
            #[tokio::test]
            async fn should_not_include_caller_info_when_tool_use_has_no_caller_field() {
                let (result, _) = anthropic_generate_default(
                    tool_use_response(json!({
                        "type": "tool_use",
                        "id": "toolu_01Test",
                        "name": "get_weather",
                        "input": { "city": "Tokyo" },
                        // No caller field
                    })),
                    json!({
                        "tools": [function_tool("get_weather", "city")],
                        "prompt": anthropic_test_prompt(),
                    }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{
                        "input": r#"{"city":"Tokyo"}"#,
                        "toolCallId": "toolu_01Test",
                        "toolName": "get_weather",
                        "type": "tool-call",
                    }])
                );
            }

            mod with_fixture_multi_turn_dice_game {
                use super::*;

                async fn before_each() -> Value {
                    let (result, _) = anthropic_generate_default(
                        anthropic_json_response(&anthropic_json_fixture!(
                            "anthropic-programmatic-tool-calling.1"
                        )),
                        json!({
                            "prompt": anthropic_test_prompt(),
                            "tools": [
                                {
                                    "type": "provider",
                                    "id": "anthropic.code_execution_20250825",
                                    "name": "code_execution",
                                    "args": {},
                                },
                                {
                                    "type": "function",
                                    "name": "rollDie",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": { "player": { "type": "string" } },
                                    },
                                },
                            ],
                        }),
                    )
                    .await;
                    anthropic_to_json(&result.content)
                }

                fn find<'a>(content: &'a Value, kind: &str, tool_name: &str) -> Option<&'a Value> {
                    content
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|part| part["type"] == kind && part["toolName"] == tool_name)
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › programmatic tool calling › with fixture (multi-turn dice game) › should parse content with text, server_tool_use, tool_use with caller, and code_execution_tool_result
                #[tokio::test]
                async fn should_parse_content_with_text_server_tool_use_tool_use_with_caller_and_result()
                 {
                    assert_anthropic_json_eq(
                        &before_each().await,
                        &anthropic_snapshot!("generate_programmatic_tool_calling_content"),
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › programmatic tool calling › with fixture (multi-turn dice game) › should extract caller metadata for programmatic tool calls
                #[tokio::test]
                async fn should_extract_caller_metadata_for_programmatic_tool_calls() {
                    let content = before_each().await;
                    // The rollDie calls should have caller metadata pointing to code_execution
                    let roll_die_calls: Vec<&Value> = content
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|part| part["type"] == "tool-call" && part["toolName"] == "rollDie")
                        .collect();
                    assert!(!roll_die_calls.is_empty());
                    for call in roll_die_calls {
                        assert_eq!(
                            call["providerMetadata"][CANONICAL]["caller"],
                            json!({
                                "type": "code_execution_20250825",
                                "toolId": "srvtoolu_01CberhXc9TgYXrCZU8bQoks",
                            })
                        );
                    }
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › programmatic tool calling › with fixture (multi-turn dice game) › should include code_execution as provider-executed tool call
                #[tokio::test]
                async fn should_include_code_execution_as_provider_executed_tool_call() {
                    let content = before_each().await;
                    let call = find(&content, "tool-call", "code_execution").expect("defined");
                    assert_anthropic_match_object(
                        call,
                        &json!({ "type": "tool-call", "toolName": "code_execution", "providerExecuted": true }),
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › programmatic tool calling › with fixture (multi-turn dice game) › should include code_execution_tool_result as provider-executed tool result
                #[tokio::test]
                async fn should_include_code_execution_tool_result_as_provider_executed_tool_result()
                 {
                    let content = before_each().await;
                    let result = find(&content, "tool-result", "code_execution").expect("defined");
                    assert_anthropic_match_object(
                        result,
                        &json!({
                            "type": "tool-result",
                            "toolName": "code_execution",
                            "result": { "stderr": "", "return_code": 0 },
                        }),
                    );
                    assert!(
                        result["result"]["stdout"]
                            .as_str()
                            .unwrap()
                            .contains("PLAYER 1 WINS THE GAME!")
                    );
                }
            }
        }

        mod web_search_tool {
            use super::*;

            /// The `TEST_PROMPT` this describe redefines.
            fn web_search_prompt() -> Value {
                json!([{ "role": "user", "content": [{ "type": "text", "text": "What is the latest news?" }] }])
            }

            fn short_response(content: Value) -> Canned {
                anthropic_json_response(&json!({
                    "type": "message",
                    "id": "msg_test",
                    "content": content,
                    "stop_reason": "end_turn",
                    "usage": { "input_tokens": 10, "output_tokens": 20 },
                }))
            }

            async fn sent_tools(tools: Value, content: Value) -> (Value, Seen) {
                let (_, seen) = anthropic_generate_default(
                    short_response(content),
                    json!({ "prompt": web_search_prompt(), "tools": tools }),
                )
                .await;
                (anthropic_request_body(&seen)["tools"].clone(), seen)
            }

            mod with_fixture {
                use super::*;

                async fn before_each() -> (GenerateResult, Seen) {
                    // Upstream's file-level provider shares one `mockId` across the
                    // file's tests; the snapshot was recorded with it at `id-14`.
                    anthropic_generate_with_first_id(
                "claude-3-haiku-20240307",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-web-search-tool.1")),
                json!({
                    "prompt": web_search_prompt(),
                    "tools": [{
                        "type": "provider",
                        "id": "anthropic.web_search_20250305",
                        "name": "web_search",
                        "args": { "maxUses": 1, "userLocation": { "type": "approximate", "country": "US" } },
                    }],
                }),
                14,
            )
            .await
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › with fixture › should send request body with include and tool
                #[tokio::test]
                async fn should_send_request_body_with_include_and_tool() {
                    let (_, seen) = before_each().await;
                    assert_eq!(
                        anthropic_request_body(&seen),
                        json!({
                            "max_tokens": 4096,
                            "messages": [{
                                "content": [{ "text": "What is the latest news?", "type": "text" }],
                                "role": "user",
                            }],
                            "model": "claude-3-haiku-20240307",
                            "tools": [{
                                "max_uses": 1,
                                "name": "web_search",
                                "type": "web_search_20250305",
                                "user_location": { "country": "US", "type": "approximate" },
                            }],
                        })
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › with fixture › should include web search tool call and result in content
                #[tokio::test]
                async fn should_include_web_search_tool_call_and_result_in_content() {
                    let (result, _) = before_each().await;
                    assert_anthropic_json_eq(
                        &anthropic_to_json(&result.content),
                        &anthropic_snapshot!("generate_web_search_content"),
                    );
                }
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should enable server-side web search when using anthropic.tools.webSearch_20250305
            #[tokio::test]
            async fn should_enable_server_side_web_search_when_using_web_search_20250305() {
                let (tools, _) = sent_tools(
            json!([{
                "type": "provider",
                "id": "anthropic.web_search_20250305",
                "name": "web_search",
                "args": { "maxUses": 3, "allowedDomains": ["arxiv.org", "nature.com", "mit.edu"] },
            }]),
            json!([{ "type": "text", "text": "Here are the latest quantum computing breakthroughs." }]),
        )
        .await;

                assert_eq!(tools.as_array().unwrap().len(), 1);
                assert_eq!(
                    tools[0],
                    json!({
                        "type": "web_search_20250305",
                        "name": "web_search",
                        "max_uses": 3,
                        "allowed_domains": ["arxiv.org", "nature.com", "mit.edu"],
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should enable server-side web search when using anthropic.tools.webSearch_20260209
            #[tokio::test]
            async fn should_enable_server_side_web_search_when_using_web_search_20260209() {
                let (tools, seen) = sent_tools(
            json!([{
                "type": "provider",
                "id": "anthropic.web_search_20260209",
                "name": "web_search",
                "args": { "maxUses": 3, "allowedDomains": ["arxiv.org", "nature.com", "mit.edu"] },
            }]),
            json!([{ "type": "text", "text": "Here are the latest quantum computing breakthroughs." }]),
        )
        .await;

                assert_eq!(tools.as_array().unwrap().len(), 1);
                assert_eq!(
                    tools[0],
                    json!({
                        "type": "web_search_20260209",
                        "name": "web_search",
                        "max_uses": 3,
                        "allowed_domains": ["arxiv.org", "nature.com", "mit.edu"],
                    })
                );
                assert_eq!(
                    seen.headers.get("anthropic-beta").map(String::as_str),
                    Some("code-execution-web-tools-2026-02-09")
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should use web_search_20260318 with response inclusion and map custom tool names
            #[tokio::test]
            async fn should_use_web_search_20260318_with_response_inclusion_and_map_custom_names() {
                let (result, seen) = anthropic_generate_default(
            short_response(json!([
                {
                    "type": "server_tool_use",
                    "id": "srvtoolu_code",
                    "name": "code_execution",
                    "input": { "code": "search(\"AI SDK\")" },
                    "caller": { "type": "direct" },
                },
                {
                    "type": "server_tool_use",
                    "id": "srvtoolu_search",
                    "name": "web_search",
                    "input": { "query": "AI SDK" },
                    "caller": { "type": "code_execution_20260120", "tool_id": "srvtoolu_code" },
                },
            ])),
            json!({
                "prompt": web_search_prompt(),
                "tools": [{
                    "type": "provider",
                    "id": "anthropic.web_search_20260318",
                    "name": "research",
                    "args": { "maxUses": 3, "responseInclusion": "excluded" },
                }],
            }),
        )
        .await;

                assert_eq!(
                    anthropic_request_body(&seen)["tools"],
                    json!([{
                        "type": "web_search_20260318",
                        "name": "web_search",
                        "max_uses": 3,
                        "response_inclusion": "excluded",
                    }])
                );
                assert!(!seen.headers.contains_key("anthropic-beta"));
                let content = anthropic_to_json(&result.content);
                assert_eq!(content.as_array().unwrap().len(), 2, "{content:#}");
                assert_anthropic_match_object(
                    &content[0],
                    &json!({ "type": "tool-call", "toolName": "code_execution", "dynamic": true }),
                );
                assert_anthropic_match_object(
                    &content[1],
                    &json!({ "type": "tool-call", "toolName": "research" }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should pass web search configuration with blocked domains
            #[tokio::test]
            async fn should_pass_web_search_configuration_with_blocked_domains() {
                let (tools, _) = sent_tools(
                    json!([{
                        "type": "provider",
                        "id": "anthropic.web_search_20250305",
                        "name": "web_search",
                        "args": { "maxUses": 2, "blockedDomains": ["reddit.com"] },
                    }]),
                    json!([{ "type": "text", "text": "Here are the latest stock market trends." }]),
                )
                .await;

                assert_eq!(tools.as_array().unwrap().len(), 1);
                assert_eq!(
                    tools[0],
                    json!({
                        "type": "web_search_20250305",
                        "name": "web_search",
                        "max_uses": 2,
                        "blocked_domains": ["reddit.com"],
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should handle web search with user location
            #[tokio::test]
            async fn should_handle_web_search_with_user_location() {
                let location = json!({
                    "type": "approximate",
                    "city": "New York",
                    "region": "New York",
                    "country": "US",
                    "timezone": "America/New_York",
                });
                let (tools, _) = sent_tools(
                    json!([{
                        "type": "provider",
                        "id": "anthropic.web_search_20250305",
                        "name": "web_search",
                        "args": { "maxUses": 1, "userLocation": location },
                    }]),
                    json!([{ "type": "text", "text": "Here are local tech events." }]),
                )
                .await;

                assert_eq!(tools.as_array().unwrap().len(), 1);
                assert_eq!(
                    tools[0],
                    json!({
                        "type": "web_search_20250305",
                        "name": "web_search",
                        "max_uses": 1,
                        "user_location": location,
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should handle web search with partial user location (city + country)
            #[tokio::test]
            async fn should_handle_web_search_with_partial_user_location_city_and_country() {
                let location = json!({ "type": "approximate", "city": "London", "country": "GB" });
                let (tools, _) = sent_tools(
                    json!([{
                        "type": "provider",
                        "id": "anthropic.web_search_20250305",
                        "name": "web_search",
                        "args": { "maxUses": 1, "userLocation": location },
                    }]),
                    json!([{ "type": "text", "text": "Here are local events." }]),
                )
                .await;

                assert_eq!(tools.as_array().unwrap().len(), 1);
                assert_eq!(
                    tools[0],
                    json!({
                        "type": "web_search_20250305",
                        "name": "web_search",
                        "max_uses": 1,
                        "user_location": location,
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should handle web search with minimal user location (country only)
            #[tokio::test]
            async fn should_handle_web_search_with_minimal_user_location_country_only() {
                let location = json!({ "type": "approximate", "country": "US" });
                let (tools, _) = sent_tools(
                    json!([{
                        "type": "provider",
                        "id": "anthropic.web_search_20250305",
                        "name": "web_search",
                        "args": { "maxUses": 1, "userLocation": location },
                    }]),
                    json!([{ "type": "text", "text": "Here are global events." }]),
                )
                .await;

                assert_eq!(tools.as_array().unwrap().len(), 1);
                assert_eq!(
                    tools[0],
                    json!({
                        "type": "web_search_20250305",
                        "name": "web_search",
                        "max_uses": 1,
                        "user_location": location,
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should handle server-side web search results with citations
            #[tokio::test]
            async fn should_handle_server_side_web_search_results_with_citations() {
                let (result, _) = anthropic_generate(
            "claude-3-5-sonnet-latest",
            anthropic_json_response(&json!({
                "type": "message",
                "id": "msg_test",
                "content": [
                    {
                        "type": "server_tool_use",
                        "id": "tool_1",
                        "name": "web_search",
                        "input": { "query": "latest AI news" },
                    },
                    {
                        "type": "web_search_tool_result",
                        "tool_use_id": "tool_1",
                        "content": [{
                            "type": "web_search_result",
                            "url": "https://example.com/ai-news",
                            "title": "Latest AI Developments",
                            "encrypted_content": "encrypted_content_123",
                            "page_age": "January 15, 2025",
                        }],
                    },
                    {
                        "type": "text",
                        "text": "Based on recent articles, AI continues to advance rapidly.",
                    },
                ],
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 20,
                    "server_tool_use": { "web_search_requests": 1 },
                },
            })),
            json!({
                "prompt": web_search_prompt(),
                "tools": [{
                    "type": "provider",
                    "id": "anthropic.web_search_20250305",
                    "name": "web_search",
                    "args": { "maxUses": 5 },
                }],
            }),
        )
        .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        {
                            "input": r#"{"query":"latest AI news"}"#,
                            "providerExecuted": true,
                            "toolCallId": "tool_1",
                            "toolName": "web_search",
                            "type": "tool-call",
                        },
                        {
                            "result": [{
                                "encryptedContent": "encrypted_content_123",
                                "pageAge": "January 15, 2025",
                                "title": "Latest AI Developments",
                                "type": "web_search_result",
                                "url": "https://example.com/ai-news",
                            }],
                            "toolCallId": "tool_1",
                            "toolName": "web_search",
                            "type": "tool-result",
                        },
                        {
                            "id": "id-0",
                            "providerMetadata": { CANONICAL: { "pageAge": "January 15, 2025" } },
                            "sourceType": "url",
                            "title": "Latest AI Developments",
                            "type": "source",
                            "url": "https://example.com/ai-news",
                        },
                        {
                            "text": "Based on recent articles, AI continues to advance rapidly.",
                            "type": "text",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should handle server-side web search errors
            #[tokio::test]
            async fn should_handle_server_side_web_search_errors() {
                let (result, _) = anthropic_generate_default(
                    short_response(json!([
                        {
                            "type": "web_search_tool_result",
                            "tool_use_id": "tool_1",
                            "content": {
                                "type": "web_search_tool_result_error",
                                "error_code": "max_uses_exceeded",
                            },
                        },
                        { "type": "text", "text": "I cannot search further due to limits." },
                    ])),
                    json!({
                        "prompt": web_search_prompt(),
                        "tools": [{
                            "type": "provider",
                            "id": "anthropic.web_search_20250305",
                            "name": "web_search",
                            "args": { "maxUses": 1 },
                        }],
                    }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        {
                            "isError": true,
                            "result": { "errorCode": "max_uses_exceeded", "type": "web_search_tool_result_error" },
                            "toolCallId": "tool_1",
                            "toolName": "web_search",
                            "type": "tool-result",
                        },
                        { "text": "I cannot search further due to limits.", "type": "text" },
                    ])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web search tool › should work alongside regular client-side tools
            #[tokio::test]
            async fn should_work_alongside_regular_client_side_tools() {
                let (tools, _) = sent_tools(
                    json!([
                        {
                            "type": "function",
                            "name": "calculator",
                            "description": "Calculate math",
                            "inputSchema": { "type": "object", "properties": {} },
                            "providerOptions": { CANONICAL: { "eagerInputStreaming": true } },
                        },
                        {
                            "type": "provider",
                            "id": "anthropic.web_search_20250305",
                            "name": "web_search",
                            "args": { "maxUses": 1 },
                        },
                    ]),
                    json!([{ "type": "text", "text": "I can search and calculate." }]),
                )
                .await;

                assert_eq!(tools.as_array().unwrap().len(), 2);
                assert_eq!(
                    tools[0],
                    json!({
                        "name": "calculator",
                        "description": "Calculate math",
                        "input_schema": { "type": "object", "properties": {} },
                        "eager_input_streaming": true,
                    })
                );
                assert_eq!(
                    tools[1],
                    json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": 1 })
                );
            }
        }

        mod web_fetch_tool {
            use super::*;

            fn web_fetch_tool(id: &str, args: Value) -> Value {
                json!({ "type": "provider", "id": id, "name": "web_fetch", "args": args })
            }

            mod text_response {
                use super::*;

                async fn before_each() -> (GenerateResult, Seen) {
                    anthropic_generate_default(
                anthropic_json_response(&anthropic_json_fixture!("anthropic-web-fetch-tool.1")),
                json!({
                    "prompt": anthropic_test_prompt(),
                    "tools": [web_fetch_tool("anthropic.web_fetch_20250910", json!({ "maxUses": 1 }))],
                }),
            )
            .await
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web fetch tool › text response › should send request body with include and tool
                #[tokio::test]
                async fn should_send_request_body_with_include_and_tool() {
                    let (_, seen) = before_each().await;
                    assert_eq!(
                        anthropic_request_body(&seen),
                        json!({
                            "max_tokens": 4096,
                            "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                            "model": "claude-3-haiku-20240307",
                            "tools": [{ "max_uses": 1, "name": "web_fetch", "type": "web_fetch_20250910" }],
                        })
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web fetch tool › text response › should include web fetch tool call and result in content
                #[tokio::test]
                async fn should_include_web_fetch_tool_call_and_result_in_content() {
                    let (result, _) = before_each().await;
                    assert_anthropic_json_eq(
                        &anthropic_to_json(&result.content),
                        &anthropic_snapshot!("generate_web_fetch_content"),
                    );
                }
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web fetch tool › should use web_fetch_20260209 for anthropic.tools.webFetch_20260209
            #[tokio::test]
            async fn should_use_web_fetch_20260209_for_web_fetch_20260209() {
                let (_, seen) = anthropic_generate_default(
            anthropic_json_response(&json!({
                "type": "message",
                "id": "msg_test",
                "content": [{ "type": "text", "text": "Fetched result." }],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 10, "output_tokens": 20 },
            })),
            json!({
                "prompt": anthropic_test_prompt(),
                "tools": [web_fetch_tool("anthropic.web_fetch_20260209", json!({ "maxUses": 1 }))],
            }),
        )
        .await;

                let tools = anthropic_request_body(&seen)["tools"].clone();
                assert_eq!(tools.as_array().unwrap().len(), 1);
                assert_eq!(
                    tools[0],
                    json!({ "type": "web_fetch_20260209", "name": "web_fetch", "max_uses": 1 })
                );
                assert_eq!(
                    seen.headers.get("anthropic-beta").map(String::as_str),
                    Some("code-execution-web-tools-2026-02-09")
                );
            }

            mod response_20260209_text {
                use super::*;

                async fn before_each() -> Value {
                    let (result, _) = anthropic_generate_default(
                        anthropic_json_response(&anthropic_json_fixture!(
                            "anthropic-web-fetch-tool-20260209.1"
                        )),
                        json!({
                            "prompt": anthropic_test_prompt(),
                            "tools": [
                                web_fetch_tool("anthropic.web_fetch_20260209", json!({})),
                                {
                                    "type": "provider",
                                    "id": "anthropic.code_execution_20260120",
                                    "name": "code_execution",
                                    "args": {},
                                },
                            ],
                        }),
                    )
                    .await;
                    anthropic_to_json(&result.content)
                }

                fn find<'a>(content: &'a Value, kind: &str, tool_name: &str) -> Option<&'a Value> {
                    content
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|part| part["type"] == kind && part["toolName"] == tool_name)
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web fetch tool › 20260209 text response › should include web fetch tool call with input in content
                #[tokio::test]
                async fn should_include_web_fetch_tool_call_with_input_in_content() {
                    let content = before_each().await;
                    let call = find(&content, "tool-call", "web_fetch").expect("defined");
                    assert_anthropic_match_object(
                        call,
                        &json!({
                            "type": "tool-call",
                            "toolName": "web_fetch",
                            "input": r#"{"url":"https://example.com"}"#,
                            "providerExecuted": true,
                        }),
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web fetch tool › 20260209 text response › should include web fetch 20260209 tool call and result in content
                #[tokio::test]
                async fn should_include_web_fetch_20260209_tool_call_and_result_in_content() {
                    assert_anthropic_json_eq(
                        &before_each().await,
                        &anthropic_snapshot!("generate_web_fetch_20260209_content"),
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web fetch tool › 20260209 text response › should preserve dynamic filtering callers
                #[tokio::test]
                async fn should_preserve_dynamic_filtering_callers() {
                    let content = before_each().await;
                    let caller = |part: Option<&Value>| {
                        part.map(|part| part["providerMetadata"][CANONICAL]["caller"].clone())
                    };
                    assert_eq!(
                        caller(find(&content, "tool-call", "code_execution")),
                        Some(json!({ "type": "direct" }))
                    );
                    let programmatic = json!({
                        "type": "code_execution_20260120",
                        "toolId": "srvtoolu_015CSHH7X69AhdK9gNzotEeh",
                    });
                    assert_eq!(
                        caller(find(&content, "tool-call", "web_fetch")),
                        Some(programmatic.clone())
                    );
                    assert_eq!(
                        caller(find(&content, "tool-result", "web_fetch")),
                        Some(programmatic)
                    );
                }
            }

            mod text_response_without_title {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web fetch tool › text response without title › should include web fetch tool call and result in content
                #[tokio::test]
                async fn should_include_web_fetch_tool_call_and_result_in_content() {
                    let (result, _) = anthropic_generate_default(
                anthropic_json_response(&anthropic_json_fixture!("anthropic-web-fetch-tool.2")),
                json!({
                    "prompt": anthropic_test_prompt(),
                    "tools": [web_fetch_tool("anthropic.web_fetch_20250910", json!({ "maxUses": 1 }))],
                }),
            )
            .await;
                    assert_anthropic_json_eq(
                        &anthropic_to_json(&result.content),
                        &anthropic_snapshot!("generate_web_fetch_without_title_content"),
                    );
                }
            }

            mod unavailable_error {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › web fetch tool › unavailable error › should include web fetch tool call and result in content
                #[tokio::test]
                async fn should_include_web_fetch_tool_call_and_result_in_content() {
                    let (result, _) = anthropic_generate_default(
                anthropic_json_response(&anthropic_json_fixture!("anthropic-web-fetch-tool.error")),
                json!({
                    "prompt": anthropic_test_prompt(),
                    "tools": [web_fetch_tool("anthropic.web_fetch_20250910", json!({ "maxUses": 1 }))],
                }),
            )
            .await;
                    assert_anthropic_json_eq(
                        &anthropic_to_json(&result.content),
                        &anthropic_snapshot!("generate_web_fetch_unavailable_error_content"),
                    );
                }
            }
        }
        mod tool_search_tool {
            use super::*;

            fn user_prompt(text: &str) -> Value {
                json!([{ "role": "user", "content": [{ "type": "text", "text": text }] }])
            }

            fn tool_search(id: &str) -> Value {
                json!({ "type": "provider", "id": id, "name": "tool_search", "args": {} })
            }

            fn deferred_tool(name: &str, description: &str, properties: Value) -> Value {
                json!({
                    "type": "function",
                    "name": name,
                    "description": description,
                    "inputSchema": { "type": "object", "properties": properties },
                    "providerOptions": { CANONICAL: { "deferLoading": true } },
                })
            }

            fn location_and_unit() -> Value {
                json!({
                    "location": { "type": "string" },
                    "unit": { "type": "string", "enum": ["celsius", "fahrenheit"] },
                })
            }

            fn structured_outputs_headers() -> Value {
                json!({
                    "anthropic-beta": "structured-outputs-2025-11-13",
                    "anthropic-version": "2023-06-01",
                    "content-type": "application/json",
                    "x-api-key": "test-api-key",
                })
            }

            mod regex_variant {
                use super::*;

                async fn before_each() -> (GenerateResult, Seen) {
                    anthropic_generate(
                "claude-sonnet-4-5",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-tool-search-regex.1")),
                json!({
                    "prompt": user_prompt("Find out weather data in SF"),
                    "tools": [
                        tool_search("anthropic.tool_search_regex_20251119"),
                        deferred_tool("get_temp_data", "For a location", location_and_unit()),
                    ],
                }),
            )
            .await
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › tool search tool › regex variant › should send request body with tool search tool and deferred tools
                #[tokio::test]
                async fn should_send_request_body_with_tool_search_tool_and_deferred_tools() {
                    let (_, seen) = before_each().await;
                    assert_eq!(
                        anthropic_request_body(&seen),
                        json!({
                            "max_tokens": 64000,
                            "messages": [{
                                "content": [{ "text": "Find out weather data in SF", "type": "text" }],
                                "role": "user",
                            }],
                            "model": "claude-sonnet-4-5",
                            "tools": [
                                { "name": "tool_search_tool_regex", "type": "tool_search_tool_regex_20251119" },
                                {
                                    "defer_loading": true,
                                    "description": "For a location",
                                    "input_schema": { "properties": location_and_unit(), "type": "object" },
                                    "name": "get_temp_data",
                                },
                            ],
                        })
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › tool search tool › regex variant › should include advanced-tool-use beta header
                #[tokio::test]
                async fn should_include_advanced_tool_use_beta_header() {
                    let (_, seen) = before_each().await;
                    assert_eq!(
                        anthropic_request_headers(&seen),
                        structured_outputs_headers()
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › tool search tool › regex variant › should include tool search tool call and result in content
                #[tokio::test]
                async fn should_include_tool_search_tool_call_and_result_in_content() {
                    let (result, _) = before_each().await;
                    assert_anthropic_json_eq(
                        &anthropic_to_json(&result.content),
                        &anthropic_snapshot!("generate_tool_search_regex_content"),
                    );
                }
            }

            mod bm25_variant {
                use super::*;

                async fn before_each() -> (GenerateResult, Seen) {
                    anthropic_generate(
                        "claude-sonnet-4-5",
                        anthropic_json_response(&anthropic_json_fixture!(
                            "anthropic-tool-search-bm25.1"
                        )),
                        json!({
                            "prompt": user_prompt("What is the weather in San Francisco?"),
                            "tools": [
                                tool_search("anthropic.tool_search_bm25_20251119"),
                                deferred_tool(
                                    "get_weather",
                                    "Get the current weather at a specific location",
                                    location_and_unit(),
                                ),
                            ],
                        }),
                    )
                    .await
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › tool search tool › bm25 variant › should send request body with tool search bm25 tool
                #[tokio::test]
                async fn should_send_request_body_with_tool_search_bm25_tool() {
                    let (_, seen) = before_each().await;
                    assert_eq!(
                        anthropic_request_body(&seen),
                        json!({
                            "max_tokens": 64000,
                            "messages": [{
                                "content": [{ "text": "What is the weather in San Francisco?", "type": "text" }],
                                "role": "user",
                            }],
                            "model": "claude-sonnet-4-5",
                            "tools": [
                                { "name": "tool_search_tool_bm25", "type": "tool_search_tool_bm25_20251119" },
                                {
                                    "defer_loading": true,
                                    "description": "Get the current weather at a specific location",
                                    "input_schema": { "properties": location_and_unit(), "type": "object" },
                                    "name": "get_weather",
                                },
                            ],
                        })
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › tool search tool › bm25 variant › should include advanced-tool-use beta header
                #[tokio::test]
                async fn should_include_advanced_tool_use_beta_header() {
                    let (_, seen) = before_each().await;
                    assert_eq!(
                        anthropic_request_headers(&seen),
                        structured_outputs_headers()
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › tool search tool › bm25 variant › should include tool search tool call and result in content
                #[tokio::test]
                async fn should_include_tool_search_tool_call_and_result_in_content() {
                    let (result, _) = before_each().await;
                    assert_anthropic_json_eq(
                        &anthropic_to_json(&result.content),
                        &anthropic_snapshot!("generate_tool_search_bm25_content"),
                    );
                }
            }

            mod deferred_result_bm25_variant {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › tool search tool › deferred result - bm25 variant › should correctly map tool_search_tool_result when result comes without server_tool_use in same response
                #[tokio::test]
                async fn should_map_tool_search_tool_result_without_server_tool_use_in_response() {
                    let (result, _) = anthropic_generate(
                        "claude-sonnet-4-5",
                        anthropic_json_response(&anthropic_json_fixture!(
                            "anthropic-tool-search-deferred-bm25.2"
                        )),
                        json!({
                            "prompt": user_prompt("What is the weather in San Francisco?"),
                            "tools": [
                                tool_search("anthropic.tool_search_bm25_20251119"),
                                deferred_tool(
                                    "get_weather",
                                    "Get the current weather at a specific location",
                                    json!({ "location": { "type": "string" } }),
                                ),
                            ],
                        }),
                    )
                    .await;

                    // The tool result should be correctly mapped to 'tool_search' (the user's custom name)
                    // even though serverToolCalls map is empty (no server_tool_use in this response)
                    let content = anthropic_to_json(&result.content);
                    let tool_result = content
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|part| part["type"] == "tool-result")
                        .expect("defined");
                    assert_eq!(tool_result["toolName"], json!("tool_search"));
                }
            }

            mod deferred_result_regex_variant {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › tool search tool › deferred result - regex variant › should correctly map tool_search_tool_result when result comes without server_tool_use in same response
                #[tokio::test]
                async fn should_map_tool_search_tool_result_without_server_tool_use_in_response() {
                    let (result, _) = anthropic_generate(
                        "claude-sonnet-4-5",
                        anthropic_json_response(&anthropic_json_fixture!(
                            "anthropic-tool-search-deferred-regex.2"
                        )),
                        json!({
                            "prompt": user_prompt("Find weather data in NYC"),
                            "tools": [
                                tool_search("anthropic.tool_search_regex_20251119"),
                                deferred_tool(
                                    "get_temp_data",
                                    "For a location",
                                    json!({ "location": { "type": "string" } }),
                                ),
                            ],
                        }),
                    )
                    .await;

                    let content = anthropic_to_json(&result.content);
                    let tool_result = content
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|part| part["type"] == "tool-result")
                        .expect("defined");
                    assert_eq!(tool_result["toolName"], json!("tool_search"));
                }
            }
        }

        mod advisor_tool {
            use super::*;

            fn advisor_tool(args: Value) -> Value {
                json!({ "type": "provider", "id": "anthropic.advisor_20260301", "name": "advisor", "args": args })
            }

            fn find<'a>(content: &'a Value, kind: &str, tool_name: &str) -> Option<&'a Value> {
                content
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|part| part["type"] == kind && part["toolName"] == tool_name)
            }

            mod with_fixture {
                use super::*;

                async fn before_each() -> (GenerateResult, Seen) {
                    anthropic_generate(
                "claude-sonnet-4-6",
                anthropic_json_response(&anthropic_json_fixture!("anthropic-advisor-20260301.1")),
                json!({
                    "prompt": anthropic_test_prompt(),
                    "tools": [advisor_tool(json!({ "model": "claude-opus-4-7", "maxTokens": 2048 }))],
                }),
            )
            .await
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › advisor tool › with fixture › should send the advisor tool in the request body
                #[tokio::test]
                async fn should_send_the_advisor_tool_in_the_request_body() {
                    let (_, seen) = before_each().await;
                    assert_eq!(
                        anthropic_request_body(&seen),
                        json!({
                            "max_tokens": 128000,
                            "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
                            "model": "claude-sonnet-4-6",
                            "tools": [{
                                "max_tokens": 2048,
                                "model": "claude-opus-4-7",
                                "name": "advisor",
                                "type": "advisor_20260301",
                            }],
                        })
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › advisor tool › with fixture › should parse advisor calls and results as provider-executed tool parts
                #[tokio::test]
                async fn should_parse_advisor_calls_and_results_as_provider_executed_tool_parts() {
                    let (result, _) = before_each().await;
                    let content = anthropic_to_json(&result.content);
                    assert_eq!(
                        find(&content, "tool-call", "advisor"),
                        Some(&json!({
                            "input": "{}",
                            "providerExecuted": true,
                            "toolCallId": "srvtoolu_01H51YYF9DTtdAHfcxxTs1NV",
                            "toolName": "advisor",
                            "type": "tool-call",
                        }))
                    );
                    let advisor_result = find(&content, "tool-result", "advisor").expect("defined");
                    assert_anthropic_match_object(
                        advisor_result,
                        &json!({
                            "type": "tool-result",
                            "toolCallId": "srvtoolu_01H51YYF9DTtdAHfcxxTs1NV",
                            "toolName": "advisor",
                            "result": { "type": "advisor_result" },
                        }),
                    );
                    assert!(
                        advisor_result["result"]["text"]
                            .as_str()
                            .unwrap()
                            .contains("Deliver design outline first")
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › advisor tool › with fixture › should expose advisor usage iterations in provider metadata
                #[tokio::test]
                async fn should_expose_advisor_usage_iterations_in_provider_metadata() {
                    let (result, _) = before_each().await;
                    assert_eq!(
                        anthropic_to_json(&result.provider_metadata)[CANONICAL]["iterations"],
                        json!([
                            { "inputTokens": 1051, "outputTokens": 35, "type": "message" },
                            {
                                "inputTokens": 2728,
                                "model": "claude-opus-4-7",
                                "outputTokens": 874,
                                "type": "advisor_message",
                            },
                            { "inputTokens": 1363, "outputTokens": 3165, "type": "message" },
                        ])
                    );
                }
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › advisor tool › should expose stopReason for plaintext and redacted advisor results
            #[tokio::test]
            async fn should_expose_stop_reason_for_plaintext_and_redacted_advisor_results() {
                let (result, _) = anthropic_generate(
            "claude-sonnet-4-6",
            anthropic_json_response(&anthropic_json_fixture!("anthropic-advisor-stop-reasons")),
            json!({
                "prompt": anthropic_test_prompt(),
                "tools": [advisor_tool(json!({ "model": "claude-opus-4-7", "maxTokens": 2048 }))],
            }),
        )
        .await;

                let results: Vec<Value> = anthropic_to_json(&result.content)
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|part| part["type"] == "tool-result")
                    .cloned()
                    .collect();
                assert_eq!(
                    Value::Array(results),
                    json!([
                        {
                            "result": {
                                "stopReason": "max_tokens",
                                "text": "Partial plaintext advice.",
                                "type": "advisor_result",
                            },
                            "toolCallId": "srvtoolu_advisor_plaintext",
                            "toolName": "advisor",
                            "type": "tool-result",
                        },
                        {
                            "result": {
                                "encryptedContent": "opaque-encrypted-advice",
                                "stopReason": "end_turn",
                                "type": "advisor_redacted_result",
                            },
                            "toolCallId": "srvtoolu_advisor_redacted",
                            "toolName": "advisor",
                            "type": "tool-result",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › advisor tool › should emit a tool-call for the advisor server_tool_use so it round-trips on follow-up turns
            #[tokio::test]
            async fn should_emit_a_tool_call_for_the_advisor_server_tool_use_for_follow_up_turns() {
                let (result, _) = anthropic_generate(
            "claude-sonnet-4-6",
            anthropic_json_response(&json!({
                "type": "message",
                "id": "msg_advisor",
                "model": "claude-sonnet-4-6",
                "role": "assistant",
                "content": [
                    { "type": "server_tool_use", "id": "srvtoolu_advisor_1", "name": "advisor", "input": {} },
                    {
                        "type": "advisor_tool_result",
                        "tool_use_id": "srvtoolu_advisor_1",
                        "content": {
                            "type": "advisor_result",
                            "text": "Outline the design first; pick Submit semantics upfront.",
                        },
                    },
                    { "type": "text", "text": "Here is the design outline..." },
                ],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 10, "output_tokens": 20 },
            })),
            json!({
                "prompt": anthropic_test_prompt(),
                "tools": [advisor_tool(json!({ "model": "claude-opus-4-7" }))],
            }),
        )
        .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        {
                            "input": "{}",
                            "providerExecuted": true,
                            "toolCallId": "srvtoolu_advisor_1",
                            "toolName": "advisor",
                            "type": "tool-call",
                        },
                        {
                            "result": {
                                "text": "Outline the design first; pick Submit semantics upfront.",
                                "type": "advisor_result",
                            },
                            "toolCallId": "srvtoolu_advisor_1",
                            "toolName": "advisor",
                            "type": "tool-result",
                        },
                        { "text": "Here is the design outline...", "type": "text" },
                    ])
                );
            }
        }
        fn anthropic_hello_messages() -> Value {
            json!([{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }])
        }

        fn anthropic_code_execution_tool(id: &str) -> Value {
            json!({ "type": "provider", "id": id, "name": "code_execution", "args": {} })
        }

        /// The recorded headers with an `anthropic-beta` value (or none).
        fn anthropic_headers_with_beta(beta: Option<&str>) -> Value {
            let mut headers = json!({
                "anthropic-version": "2023-06-01",
                "content-type": "application/json",
                "x-api-key": "test-api-key",
            });
            if let Some(beta) = beta {
                headers["anthropic-beta"] = json!(beta);
            }
            headers
        }

        mod mcp_servers {
            use super::*;

            fn options() -> Value {
                json!({
                    "prompt": anthropic_test_prompt(),
                    "providerOptions": {
                        CANONICAL: {
                            "mcpServers": [{
                                "type": "url",
                                "name": "echo",
                                "url": "https://echo.mcp.inevitable.fyi/mcp",
                            }],
                        },
                    },
                })
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › mcp servers › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let (_, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-mcp.1")),
                    options(),
                )
                .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "mcp_servers": [{
                            "name": "echo",
                            "type": "url",
                            "url": "https://echo.mcp.inevitable.fyi/mcp",
                        }],
                        "messages": anthropic_hello_messages(),
                        "model": "claude-3-haiku-20240307",
                    })
                );
                assert_eq!(
                    anthropic_request_headers(&seen),
                    anthropic_headers_with_beta(Some("mcp-client-2025-04-04"))
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › mcp servers › should include mcp tool call and result in content
            #[tokio::test]
            async fn should_include_mcp_tool_call_and_result_in_content() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-mcp.1")),
                    options(),
                )
                .await;

                assert_anthropic_json_eq(
                    &anthropic_to_json(&result.content),
                    &anthropic_snapshot!("generate_mcp_content"),
                );
            }
        }

        mod code_execution_file_uploads {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution file uploads › should send container upload content with code execution tool and parse results
            #[tokio::test]
            async fn should_send_container_upload_content_with_code_execution_tool_and_parse_results()
             {
                let (result, seen) = anthropic_generate_default(
            anthropic_json_response(&anthropic_json_fixture!(
                "anthropic-code-execution-file-upload.1"
            )),
            json!({
                "prompt": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Analyze this CSV data." },
                        {
                            "type": "file",
                            "mediaType": "text/csv",
                            "data": { "type": "reference", "reference": { CANONICAL: "file-csv-12345" } },
                            "providerOptions": { CANONICAL: { "containerUpload": true } },
                        },
                    ],
                }],
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250825")],
            }),
        )
        .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "messages": [{
                            "content": [
                                { "text": "Analyze this CSV data.", "type": "text" },
                                { "file_id": "file-csv-12345", "type": "container_upload" },
                            ],
                            "role": "user",
                        }],
                        "model": "claude-3-haiku-20240307",
                        "tools": [{ "name": "code_execution", "type": "code_execution_20250825" }],
                    })
                );
                assert_eq!(
                    anthropic_request_headers(&seen),
                    anthropic_headers_with_beta(Some(
                        "files-api-2025-04-14,code-execution-2025-08-25"
                    ))
                );

                // `expect.arrayContaining([expect.objectContaining(...), ...])`
                let content = anthropic_to_json(&result.content);
                let parts = content.as_array().unwrap();
                let contains = |expected: Value, check: &dyn Fn(&Value) -> bool| {
                    assert!(
                        parts.iter().any(|part| {
                            let mut matched = true;
                            for (key, value) in expected.as_object().unwrap() {
                                matched &= part.get(key) == Some(value);
                            }
                            matched && check(part)
                        }),
                        "no part matches {expected:#}\n{content:#}"
                    );
                };
                contains(
                    json!({
                        "type": "tool-call",
                        "toolCallId": "srvtoolu_01Bxt8MXPF5wYdD7KH2HLPzR",
                        "toolName": "code_execution",
                        "input": r#"{"type":"text_editor_code_execution","command":"view","path":"$INPUT_DIR/sample.csv"}"#,
                        "providerExecuted": true,
                    }),
                    &|_| true,
                );
                contains(
                    json!({
                        "type": "tool-result",
                        "toolCallId": "srvtoolu_01Bxt8MXPF5wYdD7KH2HLPzR",
                        "toolName": "code_execution",
                    }),
                    &|part| {
                        part["result"]["type"] == "text_editor_code_execution_view_result"
                            && part["result"]["content"]
                                .as_str()
                                .is_some_and(|text| text.contains("month,revenue,expenses,profit"))
                    },
                );
                contains(
                    json!({
                        "type": "tool-call",
                        "toolCallId": "srvtoolu_0142vFu9QyCKuFZ2X14zqBLR",
                        "toolName": "code_execution",
                        "providerExecuted": true,
                    }),
                    &|_| true,
                );
                contains(
                    json!({
                        "type": "tool-result",
                        "toolCallId": "srvtoolu_0142vFu9QyCKuFZ2X14zqBLR",
                        "toolName": "code_execution",
                    }),
                    &|part| {
                        part["result"]["type"] == "bash_code_execution_result"
                            && part["result"]["stdout"]
                                .as_str()
                                .is_some_and(|text| text.contains("Total Profit: $35400"))
                    },
                );

                assert_anthropic_match_object(
                    &anthropic_to_json(&result.provider_metadata)[CANONICAL]["container"],
                    &json!({ "id": "container_011CbJPykM9av9Vj1cQkVu87" }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution file uploads › should send container id for a follow-up code execution turn
            #[tokio::test]
            async fn should_send_container_id_for_a_follow_up_code_execution_turn() {
                let (_, seen) = anthropic_generate_default(
            anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
            json!({
                "prompt": [{
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": "Use the uploaded CSV from the existing container.",
                    }],
                }],
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250825")],
                "providerOptions": { CANONICAL: { "container": { "id": "container_12345" } } },
            }),
        )
        .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "container": "container_12345",
                        "max_tokens": 4096,
                        "messages": [{
                            "content": [{
                                "text": "Use the uploaded CSV from the existing container.",
                                "type": "text",
                            }],
                            "role": "user",
                        }],
                        "model": "claude-3-haiku-20240307",
                        "tools": [{ "name": "code_execution", "type": "code_execution_20250825" }],
                    })
                );
            }
        }

        mod agent_skills {
            use super::*;

            fn pptx_skill_response() -> Canned {
                anthropic_json_response(&anthropic_json_fixture!(
                    "anthropic-code-execution-20250825.pptx-skill"
                ))
            }

            fn both_skills() -> Value {
                json!([
                    { "type": CANONICAL, "skillId": "pptx", "version": "latest" },
                    {
                        "type": "custom",
                        "providerReference": { CANONICAL: "skill_01Xud7kLMsjLfc7Aa6RvigZf" },
                        "version": "1.0",
                    },
                ])
            }

            fn both_skills_sent() -> Value {
                json!({
                    "id": "test-container-id",
                    "skills": [
                        { "skill_id": "pptx", "type": CANONICAL, "version": "latest" },
                        { "skill_id": "skill_01Xud7kLMsjLfc7Aa6RvigZf", "type": "custom", "version": "1.0" },
                    ],
                })
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › agent skills › should send request body with skills in container
            #[tokio::test]
            async fn should_send_request_body_with_skills_in_container() {
                let (result, seen) = anthropic_generate_default(
            pptx_skill_response(),
            json!({
                "prompt": anthropic_test_prompt(),
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250825")],
                "providerOptions": {
                    CANONICAL: { "container": { "id": "test-container-id", "skills": both_skills() } },
                },
            }),
        )
        .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "container": both_skills_sent(),
                        "max_tokens": 4096,
                        "messages": anthropic_hello_messages(),
                        "model": "claude-3-haiku-20240307",
                        "tools": [{ "name": "code_execution", "type": "code_execution_20250825" }],
                    })
                );
                assert_eq!(anthropic_to_json(&result.warnings), json!([]));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › agent skills › should add a warning when the code execution tool is not present
            #[tokio::test]
            async fn should_add_a_warning_when_the_code_execution_tool_is_not_present() {
                let (result, seen) = anthropic_generate_default(
            pptx_skill_response(),
            json!({
                "prompt": anthropic_test_prompt(),
                "providerOptions": {
                    CANONICAL: { "container": { "id": "test-container-id", "skills": both_skills() } },
                },
            }),
        )
        .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "container": both_skills_sent(),
                        "max_tokens": 4096,
                        "messages": anthropic_hello_messages(),
                        "model": "claude-3-haiku-20240307",
                    })
                );
                assert_eq!(
                    anthropic_to_json(&result.warnings),
                    json!([{ "message": "code execution tool is required when using skills", "type": "other" }])
                );
            }

            fn pptx_only_options() -> Value {
                json!({
                    "prompt": anthropic_test_prompt(),
                    "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250825")],
                    "providerOptions": {
                        CANONICAL: {
                            "container": {
                                "skills": [{ "type": CANONICAL, "skillId": "pptx", "version": "latest" }],
                            },
                        },
                    },
                })
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › agent skills › should include beta headers when skills are configured
            #[tokio::test]
            async fn should_include_beta_headers_when_skills_are_configured() {
                let (_, seen) =
                    anthropic_generate_default(pptx_skill_response(), pptx_only_options()).await;

                assert_eq!(
                    anthropic_request_headers(&seen),
                    anthropic_headers_with_beta(Some(
                        "code-execution-2025-08-25,skills-2025-10-02,files-api-2025-04-14"
                    ))
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › agent skills › should expose container information as provider metadata
            #[tokio::test]
            async fn should_expose_container_information_as_provider_metadata() {
                let (result, _) =
                    anthropic_generate_default(pptx_skill_response(), pptx_only_options()).await;

                assert_anthropic_json_eq(
                    &anthropic_to_json(&result.provider_metadata),
                    &anthropic_snapshot!("generate_agent_skills_container_metadata"),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › agent skills › should resolve custom skill provider references at the Anthropic boundary
            #[tokio::test]
            async fn should_resolve_custom_skill_provider_references_at_the_anthropic_boundary() {
                let (_, seen) = anthropic_generate_default(
            pptx_skill_response(),
            json!({
                "prompt": anthropic_test_prompt(),
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250825")],
                "providerOptions": {
                    CANONICAL: {
                        "container": {
                            "skills": [{
                                "type": "custom",
                                "providerReference": { CANONICAL: "skill_01Xud7kLMsjLfc7Aa6RvigZf" },
                            }],
                        },
                    },
                },
            }),
        )
        .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "container": {
                            "skills": [{ "skill_id": "skill_01Xud7kLMsjLfc7Aa6RvigZf", "type": "custom" }],
                        },
                        "max_tokens": 4096,
                        "messages": anthropic_hello_messages(),
                        "model": "claude-3-haiku-20240307",
                        "tools": [{ "name": "code_execution", "type": "code_execution_20250825" }],
                    })
                );
            }

            // `should throw when a custom skill provider reference does not include
            // anthropic`: not ported. Upstream expects a `NoSuchProviderReferenceError`;
            // aimux has no such error (`AiMuxError` has no `NoSuchProviderReference`
            // variant).
        }

        mod memory_20250818 {
            use super::*;

            fn options() -> Value {
                json!({
                    "prompt": anthropic_test_prompt(),
                    "tools": [{ "type": "provider", "id": "anthropic.memory_20250818", "name": "memory", "args": {} }],
                })
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › memory 20250818 › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let (_, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!(
                        "anthropic-memory-20250818.1"
                    )),
                    options(),
                )
                .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "messages": anthropic_hello_messages(),
                        "model": "claude-3-haiku-20240307",
                        "tools": [{ "name": "memory", "type": "memory_20250818" }],
                    })
                );
                assert_eq!(
                    anthropic_request_headers(&seen),
                    anthropic_headers_with_beta(Some("context-management-2025-06-27"))
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › memory 20250818 › should include memory tool call and result in content
            #[tokio::test]
            async fn should_include_memory_tool_call_and_result_in_content() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!(
                        "anthropic-memory-20250818.1"
                    )),
                    options(),
                )
                .await;

                assert_anthropic_json_eq(
                    &anthropic_to_json(&result.content),
                    &anthropic_snapshot!("generate_memory_20250818_content"),
                );
            }
        }

        mod code_execution_20250825 {
            use super::*;

            async fn generate(fixture: Canned) -> (GenerateResult, Seen) {
                anthropic_generate_default(
            fixture,
            json!({
                "prompt": anthropic_test_prompt(),
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250825")],
            }),
        )
        .await
            }

            fn fixture_1() -> Canned {
                anthropic_json_response(&anthropic_json_fixture!(
                    "anthropic-code-execution-20250825.1"
                ))
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20250825 › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let (_, seen) = generate(fixture_1()).await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "messages": anthropic_hello_messages(),
                        "model": "claude-3-haiku-20240307",
                        "tools": [{ "name": "code_execution", "type": "code_execution_20250825" }],
                    })
                );
                assert_eq!(
                    anthropic_request_headers(&seen),
                    anthropic_headers_with_beta(Some("code-execution-2025-08-25"))
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20250825 › should include code execution tool call and result in content
            #[tokio::test]
            async fn should_include_code_execution_tool_call_and_result_in_content() {
                let (result, _) = generate(fixture_1()).await;
                assert_anthropic_json_eq(
                    &anthropic_to_json(&result.content),
                    &anthropic_snapshot!("generate_code_execution_20250825_content"),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20250825 › should expose container information as provider metadata
            #[tokio::test]
            async fn should_expose_container_information_as_provider_metadata() {
                let (result, _) = generate(fixture_1()).await;
                assert_anthropic_json_eq(
                    &anthropic_to_json(&result.provider_metadata),
                    &anthropic_snapshot!("generate_code_execution_20250825_container_metadata"),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20250825 › should include file id list in code execution tool generate call result.
            #[tokio::test]
            async fn should_include_file_id_list_in_code_execution_tool_generate_call_result() {
                let (result, _) = generate(anthropic_json_response(&anthropic_json_fixture!(
                    "anthropic-code-execution-20250825.2"
                )))
                .await;
                assert_anthropic_json_eq(
                    &anthropic_to_json(&result),
                    &anthropic_snapshot!("generate_code_execution_20250825_file_id_list"),
                );
            }
        }

        mod code_execution_20260120 {
            use super::*;

            async fn generate() -> (GenerateResult, Seen) {
                anthropic_generate_default(
            anthropic_json_response(&anthropic_json_fixture!("anthropic-code-execution-20250825.1")),
            json!({
                "prompt": anthropic_test_prompt(),
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20260120")],
            }),
        )
        .await
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20260120 › should send request body with tool and no beta header
            #[tokio::test]
            async fn should_send_request_body_with_tool_and_no_beta_header() {
                let (_, seen) = generate().await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "messages": anthropic_hello_messages(),
                        "model": "claude-3-haiku-20240307",
                        "tools": [{ "name": "code_execution", "type": "code_execution_20260120" }],
                    })
                );
                assert_eq!(
                    anthropic_request_headers(&seen),
                    anthropic_headers_with_beta(None)
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20260120 › should include code execution tool call and result in content
            #[tokio::test]
            async fn should_include_code_execution_tool_call_and_result_in_content() {
                let (result, _) = generate().await;
                assert_anthropic_json_eq(
                    &anthropic_to_json(&result.content),
                    &anthropic_snapshot!("generate_code_execution_20260120_content"),
                );
            }
        }

        mod code_execution_20250522 {
            use super::*;

            /// The `TEST_PROMPT` this describe redefines.
            fn factorial_prompt() -> Value {
                json!([{
                    "role": "user",
                    "content": [{ "type": "text", "text": "Write a Python function to calculate factorial" }],
                }])
            }

            fn short_response(content: Value, usage: Value) -> Canned {
                anthropic_json_response(&json!({
                    "type": "message",
                    "id": "msg_test",
                    "content": content,
                    "stop_reason": "end_turn",
                    "usage": usage,
                }))
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20250522 › should enable server-side code execution when using anthropic.tools.codeExecution_20250522
            #[tokio::test]
            async fn should_enable_server_side_code_execution_when_using_code_execution_20250522() {
                let (_, seen) = anthropic_generate_default(
            short_response(
                json!([{ "type": "text", "text": "Here is a Python function to calculate factorial" }]),
                json!({ "input_tokens": 10, "output_tokens": 20 }),
            ),
            json!({
                "prompt": factorial_prompt(),
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250522")],
            }),
        )
        .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "messages": [{
                            "content": [{ "text": "Write a Python function to calculate factorial", "type": "text" }],
                            "role": "user",
                        }],
                        "model": "claude-3-haiku-20240307",
                        "tools": [{ "name": "code_execution", "type": "code_execution_20250522" }],
                    })
                );
                assert_eq!(
                    anthropic_request_headers(&seen),
                    anthropic_headers_with_beta(Some("code-execution-2025-05-22"))
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20250522 › should handle server-side code execution results
            #[tokio::test]
            async fn should_handle_server_side_code_execution_results() {
                let (result, _) = anthropic_generate_default(
            short_response(
                json!([
                    {
                        "type": "server_tool_use",
                        "id": "tool_1",
                        "name": "code_execution",
                        "input": { "code": "print(\"Hello, World!\")" },
                    },
                    {
                        "type": "code_execution_tool_result",
                        "tool_use_id": "tool_1",
                        "content": {
                            "type": "code_execution_result",
                            "stdout": "Hello, World!\n",
                            "stderr": "",
                            "return_code": 0,
                        },
                    },
                    {
                        "type": "text",
                        "text": "The code executed successfully with output: Hello, World!",
                    },
                ]),
                json!({
                    "input_tokens": 15,
                    "output_tokens": 25,
                    "server_tool_use": { "code_execution_requests": 1 },
                }),
            ),
            json!({
                "prompt": factorial_prompt(),
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250522")],
            }),
        )
        .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        {
                            "input": r#"{"type":"programmatic-tool-call","code":"print(\"Hello, World!\")"}"#,
                            "providerExecuted": true,
                            "toolCallId": "tool_1",
                            "toolName": "code_execution",
                            "type": "tool-call",
                        },
                        {
                            "result": {
                                "content": [],
                                "return_code": 0,
                                "stderr": "",
                                "stdout": "Hello, World!\n",
                                "type": "code_execution_result",
                            },
                            "toolCallId": "tool_1",
                            "toolName": "code_execution",
                            "type": "tool-result",
                        },
                        {
                            "text": "The code executed successfully with output: Hello, World!",
                            "type": "text",
                        },
                    ])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20250522 › should handle server-side code execution errors
            #[tokio::test]
            async fn should_handle_server_side_code_execution_errors() {
                let (result, _) = anthropic_generate_default(
            short_response(
                json!([
                    {
                        "type": "code_execution_tool_result",
                        "tool_use_id": "tool_1",
                        "content": { "type": "code_execution_tool_result_error", "error_code": "unavailable" },
                    },
                    { "type": "text", "text": "The code execution service is currently unavailable." },
                ]),
                json!({ "input_tokens": 10, "output_tokens": 20 }),
            ),
            json!({
                "prompt": factorial_prompt(),
                "tools": [anthropic_code_execution_tool("anthropic.code_execution_20250522")],
            }),
        )
        .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([
                        {
                            "isError": true,
                            "result": { "errorCode": "unavailable", "type": "code_execution_tool_result_error" },
                            "toolCallId": "tool_1",
                            "toolName": "code_execution",
                            "type": "tool-result",
                        },
                        { "text": "The code execution service is currently unavailable.", "type": "text" },
                    ])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › code execution 20250522 › should work alongside regular client-side tools
            #[tokio::test]
            async fn should_work_alongside_regular_client_side_tools() {
                let (_, seen) = anthropic_generate_default(
                    short_response(
                        json!([{ "type": "text", "text": "I can execute code and calculate." }]),
                        json!({ "input_tokens": 10, "output_tokens": 20 }),
                    ),
                    json!({
                        "prompt": factorial_prompt(),
                        "tools": [
                            {
                                "type": "function",
                                "name": "calculator",
                                "description": "Calculate math expressions",
                                "inputSchema": { "type": "object", "properties": {} },
                            },
                            anthropic_code_execution_tool("anthropic.code_execution_20250522"),
                        ],
                    }),
                )
                .await;

                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "max_tokens": 4096,
                        "messages": [{
                            "content": [{ "text": "Write a Python function to calculate factorial", "type": "text" }],
                            "role": "user",
                        }],
                        "model": "claude-3-haiku-20240307",
                        "tools": [
                            {
                                "description": "Calculate math expressions",
                                "input_schema": { "properties": {}, "type": "object" },
                                "name": "calculator",
                            },
                            { "name": "code_execution", "type": "code_execution_20250522" },
                        ],
                    })
                );
                assert_eq!(
                    anthropic_request_headers(&seen),
                    anthropic_headers_with_beta(Some("code-execution-2025-05-22"))
                );
            }
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should throw an api error when the server is returning a 529 overloaded error
        #[tokio::test]
        async fn should_throw_an_api_error_when_the_server_is_returning_a_529_overloaded_error() {
            let body = r#"{"type":"error","error":{"details":null,"type":"overloaded_error","message":"Overloaded"}}"#;
            let (result, _) = anthropic_try_generate(
                "claude-3-haiku-20240307",
                anthropic_error_response(529, body),
                json!({ "prompt": anthropic_test_prompt() }),
            )
            .await;

            let Err(AiMuxError::ApiCall(error)) = result else {
                panic!("Expected an APICallError, got {result:?}");
            };
            assert_eq!(error.message, "Overloaded");
            assert_eq!(error.url, "https://api.anthropic.com/v1/messages");
            assert_eq!(error.status_code, Some(529));
            assert_eq!(error.response_body.as_deref(), Some(body));
            assert!(error.is_retryable);
        }

        mod temperature_clamping {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › temperature clamping › should clamp temperature above 1 to 1 and add warning
            #[tokio::test]
            async fn should_clamp_temperature_above_1_to_1_and_add_warning() {
                let (result, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "temperature": 1.5 }),
                )
                .await;

                assert_eq!(anthropic_request_body(&seen)["temperature"], json!(1));
                assert_eq!(
                    anthropic_to_json(&result.warnings),
                    json!([{
                        "details": "1.5 exceeds anthropic maximum of 1.0. clamped to 1.0",
                        "feature": "temperature",
                        "type": "unsupported",
                    }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › temperature clamping › should clamp temperature below 0 to 0 and add warning
            #[tokio::test]
            async fn should_clamp_temperature_below_0_to_0_and_add_warning() {
                let (result, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "temperature": -0.5 }),
                )
                .await;

                assert_eq!(anthropic_request_body(&seen)["temperature"], json!(0));
                assert_eq!(
                    anthropic_to_json(&result.warnings),
                    json!([{
                        "details": "-0.5 is below anthropic minimum of 0. clamped to 0",
                        "feature": "temperature",
                        "type": "unsupported",
                    }])
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › temperature clamping › should not clamp valid temperature between 0 and 1
            #[tokio::test]
            async fn should_not_clamp_valid_temperature_between_0_and_1() {
                let (result, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt(), "temperature": 0.7 }),
                )
                .await;

                assert_eq!(anthropic_request_body(&seen)["temperature"], json!(0.7));
                assert_eq!(anthropic_to_json(&result.warnings), json!([]));
            }
        }

        /// A `doGenerate` of the default model with these `anthropic` provider
        /// options; the body, headers and warnings.
        async fn anthropic_generate_with_options(anthropic: Value) -> (Value, Value, Value) {
            let (result, seen) = anthropic_generate_default(
        anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
        json!({ "prompt": anthropic_test_prompt(), "providerOptions": { CANONICAL: anthropic } }),
    )
    .await;
            (
                anthropic_request_body(&seen),
                anthropic_request_headers(&seen),
                anthropic_to_json(&result.warnings),
            )
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should set effort
        #[tokio::test]
        async fn should_set_effort() {
            let (body, headers, warnings) =
                anthropic_generate_with_options(json!({ "effort": "medium" })).await;

            assert_eq!(
                body,
                json!({
                    "max_tokens": 4096,
                    "messages": anthropic_hello_messages(),
                    "model": "claude-3-haiku-20240307",
                    "output_config": { "effort": "medium" },
                })
            );
            assert_eq!(headers, anthropic_headers_with_beta(None));
            assert_eq!(warnings, json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should set speed and fast-mode beta header
        #[tokio::test]
        async fn should_set_speed_and_fast_mode_beta_header() {
            let (body, headers, warnings) =
                anthropic_generate_with_options(json!({ "speed": "fast" })).await;

            assert_eq!(
                body,
                json!({
                    "max_tokens": 4096,
                    "messages": anthropic_hello_messages(),
                    "model": "claude-3-haiku-20240307",
                    "speed": "fast",
                })
            );
            assert_eq!(
                headers,
                anthropic_headers_with_beta(Some("fast-mode-2026-02-01"))
            );
            assert_eq!(warnings, json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should set speed=standard without fast-mode beta header
        #[tokio::test]
        async fn should_set_speed_standard_without_fast_mode_beta_header() {
            let (body, headers, warnings) =
                anthropic_generate_with_options(json!({ "speed": "standard" })).await;

            assert_eq!(
                body,
                json!({
                    "max_tokens": 4096,
                    "messages": anthropic_hello_messages(),
                    "model": "claude-3-haiku-20240307",
                    "speed": "standard",
                })
            );
            assert_eq!(headers, anthropic_headers_with_beta(None));
            assert_eq!(warnings, json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should set inference_geo in request body
        #[tokio::test]
        async fn should_set_inference_geo_in_request_body() {
            let (body, headers, warnings) =
                anthropic_generate_with_options(json!({ "inferenceGeo": "us" })).await;

            assert_eq!(
                body,
                json!({
                    "inference_geo": "us",
                    "max_tokens": 4096,
                    "messages": anthropic_hello_messages(),
                    "model": "claude-3-haiku-20240307",
                })
            );
            assert_eq!(headers, anthropic_headers_with_beta(None));
            assert_eq!(warnings, json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should pass cache_control to request body
        #[tokio::test]
        async fn should_pass_cache_control_to_request_body() {
            let (body, _, warnings) =
                anthropic_generate_with_options(json!({ "cacheControl": { "type": "ephemeral" } }))
                    .await;

            assert_eq!(
                body,
                json!({
                    "cache_control": { "type": "ephemeral" },
                    "max_tokens": 4096,
                    "messages": anthropic_hello_messages(),
                    "model": "claude-3-haiku-20240307",
                })
            );
            assert_eq!(warnings, json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should pass cache_control with ttl to request body
        #[tokio::test]
        async fn should_pass_cache_control_with_ttl_to_request_body() {
            let (body, _, warnings) = anthropic_generate_with_options(
                json!({ "cacheControl": { "type": "ephemeral", "ttl": "1h" } }),
            )
            .await;

            assert_eq!(
                body,
                json!({
                    "cache_control": { "ttl": "1h", "type": "ephemeral" },
                    "max_tokens": 4096,
                    "messages": anthropic_hello_messages(),
                    "model": "claude-3-haiku-20240307",
                })
            );
            assert_eq!(warnings, json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › should pass metadata with user_id to request body
        #[tokio::test]
        async fn should_pass_metadata_with_user_id_to_request_body() {
            let (body, _, warnings) = anthropic_generate_with_options(
                json!({ "metadata": { "userId": "test-user-id" } }),
            )
            .await;

            assert_eq!(
                body,
                json!({
                    "max_tokens": 4096,
                    "messages": anthropic_hello_messages(),
                    "metadata": { "user_id": "test-user-id" },
                    "model": "claude-3-haiku-20240307",
                })
            );
            assert_eq!(warnings, json!([]));
        }
        mod context_management {
            use super::*;

            fn text_response_with(extra: Value) -> Canned {
                let mut body = json!({
                    "id": "msg_123",
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Hello" }],
                    "model": "claude-3-haiku-20240307",
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": { "input_tokens": 100, "output_tokens": 50 },
                });
                for (key, value) in extra.as_object().unwrap() {
                    body[key] = value.clone();
                }
                anthropic_json_response(&body)
            }

            async fn sent_with_edits(edits: Value) -> Seen {
                let (_, seen) = anthropic_generate_default(
            anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
            json!({
                "prompt": anthropic_test_prompt(),
                "providerOptions": { CANONICAL: { "contextManagement": { "edits": edits } } },
            }),
        )
        .await;
                seen
            }

            fn context_management_metadata(result: &GenerateResult) -> Value {
                anthropic_to_json(&result.provider_metadata)[CANONICAL]["contextManagement"].clone()
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should send context_management in request body
            #[tokio::test]
            async fn should_send_context_management_in_request_body() {
                let seen = sent_with_edits(json!([{ "type": "clear_tool_uses_20250919" }])).await;
                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({ "context_management": { "edits": [{ "type": "clear_tool_uses_20250919" }] } }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should add context-management beta header
            #[tokio::test]
            async fn should_add_context_management_beta_header() {
                let seen = sent_with_edits(json!([{ "type": "clear_tool_uses_20250919" }])).await;
                assert!(seen.headers["anthropic-beta"].contains("context-management-2025-06-27"));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should parse context_management from response
            #[tokio::test]
            async fn should_parse_context_management_from_response() {
                let (result, _) = anthropic_generate_default(
                    text_response_with(json!({
                        "context_management": {
                            "applied_edits": [{
                                "type": "clear_tool_uses_20250919",
                                "cleared_tool_uses": 5,
                                "cleared_input_tokens": 10000,
                            }],
                        },
                    })),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert_eq!(
                    context_management_metadata(&result),
                    json!({
                        "appliedEdits": [{
                            "type": "clear_tool_uses_20250919",
                            "clearedToolUses": 5,
                            "clearedInputTokens": 10000,
                        }],
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should map clear_tool_uses_20250919 with all options to request body
            #[tokio::test]
            async fn should_map_clear_tool_uses_20250919_with_all_options_to_request_body() {
                let seen = sent_with_edits(json!([{
                    "type": "clear_tool_uses_20250919",
                    "trigger": { "type": "input_tokens", "value": 50000 },
                    "keep": { "type": "tool_uses", "value": 5 },
                    "clearAtLeast": { "type": "input_tokens", "value": 10000 },
                    "clearToolInputs": true,
                    "excludeTools": ["important_tool"],
                }]))
                .await;
                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({
                        "context_management": {
                            "edits": [{
                                "type": "clear_tool_uses_20250919",
                                "trigger": { "type": "input_tokens", "value": 50000 },
                                "keep": { "type": "tool_uses", "value": 5 },
                                "clear_at_least": { "type": "input_tokens", "value": 10000 },
                                "clear_tool_inputs": true,
                                "exclude_tools": ["important_tool"],
                            }],
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should map clear_thinking_20251015 with keep option to request body
            #[tokio::test]
            async fn should_map_clear_thinking_20251015_with_keep_option_to_request_body() {
                let seen = sent_with_edits(json!([{
                    "type": "clear_thinking_20251015",
                    "keep": { "type": "thinking_turns", "value": 3 },
                }]))
                .await;
                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({
                        "context_management": {
                            "edits": [{
                                "type": "clear_thinking_20251015",
                                "keep": { "type": "thinking_turns", "value": 3 },
                            }],
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should map multiple context management edits to request body
            #[tokio::test]
            async fn should_map_multiple_context_management_edits_to_request_body() {
                let seen = sent_with_edits(json!([
                    { "type": "clear_tool_uses_20250919" },
                    { "type": "clear_thinking_20251015" },
                ]))
                .await;
                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({
                        "context_management": {
                            "edits": [
                                { "type": "clear_tool_uses_20250919" },
                                { "type": "clear_thinking_20251015" },
                            ],
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should map compact_20260112 to request body
            #[tokio::test]
            async fn should_map_compact_20260112_to_request_body() {
                let seen = sent_with_edits(json!([{
                    "type": "compact_20260112",
                    "trigger": { "type": "input_tokens", "value": 50000 },
                }]))
                .await;
                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({
                        "context_management": {
                            "edits": [{
                                "type": "compact_20260112",
                                "trigger": { "type": "input_tokens", "value": 50000 },
                            }],
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should map compact_20260112 with all options to request body
            #[tokio::test]
            async fn should_map_compact_20260112_with_all_options_to_request_body() {
                let seen = sent_with_edits(json!([{
                    "type": "compact_20260112",
                    "trigger": { "type": "input_tokens", "value": 50000 },
                    "pauseAfterCompaction": true,
                    "instructions": "Summarize the conversation concisely.",
                }]))
                .await;
                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({
                        "context_management": {
                            "edits": [{
                                "type": "compact_20260112",
                                "trigger": { "type": "input_tokens", "value": 50000 },
                                "pause_after_compaction": true,
                                "instructions": "Summarize the conversation concisely.",
                            }],
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should add compact beta header when using compact edit
            #[tokio::test]
            async fn should_add_compact_beta_header_when_using_compact_edit() {
                let seen = sent_with_edits(json!([{ "type": "compact_20260112" }])).await;
                let beta = &seen.headers["anthropic-beta"];
                assert!(beta.contains("compact-2026-01-12"), "{beta}");
                assert!(beta.contains("context-management-2025-06-27"), "{beta}");
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should parse compaction response with iterations and compaction content
            #[tokio::test]
            async fn should_parse_compaction_response_with_iterations_and_compaction_content() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-compaction.1")),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                let content = anthropic_to_json(&result.content);
                assert_eq!(content.as_array().unwrap().len(), 2, "{content:#}");
                assert_eq!(content[0]["type"], json!("text"));
                assert!(
                    content[0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("## Summary of Conversation")
                );
                assert_eq!(
                    content[0]["providerMetadata"],
                    json!({ CANONICAL: { "type": "compaction" } })
                );
                assert_eq!(content[0].as_object().unwrap().len(), 3, "{:#}", content[0]);
                assert_eq!(content[1]["type"], json!("text"));
                assert!(
                    content[1]["text"]
                        .as_str()
                        .unwrap()
                        .contains("Based on our conversation history, you had asked me")
                );
                assert_eq!(content[1].as_object().unwrap().len(), 2, "{:#}", content[1]);

                assert_eq!(
                    anthropic_to_json(&result.provider_metadata)[CANONICAL]["iterations"],
                    json!([
                        { "type": "compaction", "inputTokens": 60385, "outputTokens": 592 },
                        { "type": "message", "inputTokens": 682, "outputTokens": 1320 },
                    ])
                );
                assert_anthropic_match_object(
                    &anthropic_to_json(&result.usage),
                    &json!({
                        "inputTokens": { "total": 60385 + 682 },
                        "outputTokens": { "total": 592 + 1320 },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should omit compaction response with %s content
            #[tokio::test]
            async fn should_omit_compaction_response_with_s_content() {
                for (_, content) in [("empty", json!("")), ("null", Value::Null)] {
                    let (result, _) = anthropic_generate_default(
                        anthropic_json_response(&json!({
                            "id": "msg_123",
                            "type": "message",
                            "role": "assistant",
                            "content": [
                                { "type": "compaction", "content": content },
                                { "type": "text", "text": "Hello" },
                            ],
                            "model": "claude-3-haiku-20240307",
                            "stop_reason": "end_turn",
                            "stop_sequence": null,
                            "usage": { "input_tokens": 100, "output_tokens": 50 },
                        })),
                        json!({ "prompt": anthropic_test_prompt() }),
                    )
                    .await;

                    assert_eq!(
                        anthropic_to_json(&result.content),
                        json!([{ "type": "text", "text": "Hello" }]),
                        "content {content}"
                    );
                }
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should parse context_management with compact_20260112 from response
            #[tokio::test]
            async fn should_parse_context_management_with_compact_20260112_from_response() {
                let (result, _) = anthropic_generate_default(
                    text_response_with(json!({
                        "context_management": { "applied_edits": [{ "type": "compact_20260112" }] },
                    })),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert_eq!(
                    context_management_metadata(&result),
                    json!({ "appliedEdits": [{ "type": "compact_20260112" }] })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should parse clear_tool_uses response with context_management
            #[tokio::test]
            async fn should_parse_clear_tool_uses_response_with_context_management() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!(
                        "anthropic-clear-tool-uses.1"
                    )),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                let content = anthropic_to_json(&result.content);
                assert_eq!(content.as_array().unwrap().len(), 1, "{content:#}");
                assert_eq!(content[0].as_object().unwrap().len(), 2, "{content:#}");
                assert_eq!(content[0]["type"], json!("text"));
                assert!(
                    content[0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("Here's a comparison of the weather in both cities")
                );
                assert_eq!(
                    context_management_metadata(&result),
                    json!({ "appliedEdits": [] })
                );
                assert_eq!(
                    anthropic_to_json(&result.usage),
                    json!({
                        "inputTokens": { "cacheRead": 0, "cacheWrite": 0, "noCache": 859, "total": 859 },
                        "outputTokens": { "total": 132 },
                        "raw": {
                            "cache_creation": { "ephemeral_1h_input_tokens": 0, "ephemeral_5m_input_tokens": 0 },
                            "cache_creation_input_tokens": 0,
                            "cache_read_input_tokens": 0,
                            "inference_geo": "not_available",
                            "input_tokens": 859,
                            "output_tokens": 132,
                            "service_tier": "standard",
                        },
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should parse clear_thinking response with thinking and context_management
            #[tokio::test]
            async fn should_parse_clear_thinking_response_with_thinking_and_context_management() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-clear-thinking.1")),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                let content = anthropic_to_json(&result.content);
                assert_eq!(content.as_array().unwrap().len(), 2, "{content:#}");
                // `signature: expect.any(String)`
                assert!(content[0]["providerMetadata"][CANONICAL]["signature"].is_string());
                let mut first = content[0].clone();
                first["providerMetadata"][CANONICAL]["signature"] = json!("<any string>");
                assert_eq!(
                    first,
                    json!({
                        "type": "reasoning",
                        "text": "925 divided by 5 = 185",
                        "providerMetadata": { CANONICAL: { "signature": "<any string>" } },
                    })
                );
                assert_eq!(
                    content[1],
                    json!({ "type": "text", "text": "925 ÷ 5 = 185" })
                );
                assert_eq!(
                    context_management_metadata(&result),
                    json!({ "appliedEdits": [] })
                );
                assert_eq!(
                    anthropic_to_json(&result.usage),
                    json!({
                        "inputTokens": { "cacheRead": 0, "cacheWrite": 0, "noCache": 69, "total": 69 },
                        "outputTokens": { "total": 33 },
                        "raw": {
                            "cache_creation": { "ephemeral_1h_input_tokens": 0, "ephemeral_5m_input_tokens": 0 },
                            "cache_creation_input_tokens": 0,
                            "cache_read_input_tokens": 0,
                            "inference_geo": "not_available",
                            "input_tokens": 69,
                            "output_tokens": 33,
                            "service_tier": "standard",
                        },
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should parse combined clear_thinking and clear_tool_uses response
            #[tokio::test]
            async fn should_parse_combined_clear_thinking_and_clear_tool_uses_response() {
                let edits = json!([
                    { "type": "clear_thinking_20251015", "keep": { "type": "thinking_turns", "value": 1 } },
                    {
                        "type": "clear_tool_uses_20250919",
                        "trigger": { "type": "tool_uses", "value": 4 },
                        "keep": { "type": "tool_uses", "value": 1 },
                    },
                ]);
                let (result, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!(
                        "anthropic-combined-context-editing.1"
                    )),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "providerOptions": {
                            CANONICAL: {
                                "thinking": { "type": "enabled", "budgetTokens": 5000 },
                                "contextManagement": { "edits": edits },
                            },
                        },
                    }),
                )
                .await;

                let content = anthropic_to_json(&result.content);
                assert_eq!(content.as_array().unwrap().len(), 2, "{content:#}");
                assert_eq!(content[0]["type"], json!("reasoning"));
                assert!(content[0]["text"].as_str().unwrap().contains("25 * 37"));
                assert!(content[0]["providerMetadata"][CANONICAL]["signature"].is_string());
                assert_eq!(
                    content[0]["providerMetadata"][CANONICAL]
                        .as_object()
                        .unwrap()
                        .len(),
                    1
                );
                assert_eq!(content[0].as_object().unwrap().len(), 3, "{:#}", content[0]);
                assert_eq!(content[1]["type"], json!("text"));
                assert!(content[1]["text"].as_str().unwrap().contains("925"));
                assert_eq!(content[1].as_object().unwrap().len(), 2, "{:#}", content[1]);
                assert_eq!(
                    context_management_metadata(&result),
                    json!({ "appliedEdits": [] })
                );
                assert_eq!(
                    anthropic_request_body(&seen),
                    json!({
                        "context_management": { "edits": edits },
                        "max_tokens": 4096,
                        "messages": anthropic_hello_messages(),
                        "model": "claude-3-haiku-20240307",
                        "thinking": { "budget_tokens": 5000, "type": "enabled" },
                    })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › context management › should return compaction stop reason as other
            #[tokio::test]
            async fn should_return_compaction_stop_reason_as_other() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&json!({
                        "id": "msg_123",
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "compaction", "content": "Compaction summary..." }],
                        "model": "claude-3-haiku-20240307",
                        "stop_reason": "compaction",
                        "stop_sequence": null,
                        "usage": { "input_tokens": 100, "output_tokens": 50 },
                    })),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "unified": "other", "raw": "compaction" })
                );
            }
        }

        mod on_demand_compaction {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › on-demand compaction › should send compaction in the request body and add the beta header
            #[tokio::test]
            async fn should_send_compaction_in_the_request_body_and_add_the_beta_header() {
                let compaction = json!({
                    "type": "summarize",
                    "instructions": "Preserve decisions and unresolved questions.",
                });
                let (_, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "providerOptions": { CANONICAL: { "compaction": compaction } },
                    }),
                )
                .await;

                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({ "compaction": compaction }),
                );
                assert!(seen.headers["anthropic-beta"].contains("compact-2026-09-04"));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › on-demand compaction › should reject compaction together with context management
            #[tokio::test]
            async fn should_reject_compaction_together_with_context_management() {
                let (result, mock) = anthropic_try_generate(
            "claude-3-haiku-20240307",
            anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
            json!({
                "prompt": anthropic_test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "compaction": { "type": "summarize" },
                        "contextManagement": { "edits": [{ "type": "clear_tool_uses_20250919" }] },
                    },
                },
            }),
        )
        .await;

                // `name: 'AI_InvalidArgumentError'` is the variant; aimux's
                // `InvalidArgument` carries no `argument` field to compare with
                // upstream's `argument: 'providerOptions'`.
                let Err(AiMuxError::InvalidArgument(message)) = result else {
                    panic!("expected an InvalidArgumentError, got {result:?}");
                };
                assert_eq!(
                    message,
                    "Anthropic provider options `compaction` and `contextManagement` cannot be used together."
                );
                assert_eq!(mock.seen().len(), 0);
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › on-demand compaction › should preserve the signature on compaction responses
            #[tokio::test]
            async fn should_preserve_the_signature_on_compaction_responses() {
                let (result, _) = anthropic_generate_default(
            anthropic_json_response(&json!({
                "id": "msg_compaction",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "compaction",
                    "content": "Summary of the conversation.",
                    "signature": "compaction-signature",
                }],
                "model": "claude-opus-5",
                "stop_reason": "compaction",
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "iterations": [{ "type": "compaction", "input_tokens": 120, "output_tokens": 30 }],
                },
            })),
            json!({ "prompt": anthropic_test_prompt() }),
        )
        .await;

                assert_eq!(
                    anthropic_to_json(&result.content),
                    json!([{
                        "type": "text",
                        "text": "Summary of the conversation.",
                        "providerMetadata": {
                            CANONICAL: { "type": "compaction", "signature": "compaction-signature" },
                        },
                    }])
                );
                assert_eq!(
                    anthropic_to_json(&result.finish_reason),
                    json!({ "unified": "other", "raw": "compaction" })
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › on-demand compaction › should replay signed compaction blocks and add the beta header
            #[tokio::test]
            async fn should_replay_signed_compaction_blocks_and_add_the_beta_header() {
                let (_, seen) = anthropic_generate_default(
            anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
            json!({
                "prompt": [
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "text",
                            "text": "Summary of the conversation.",
                            "providerOptions": {
                                CANONICAL: { "type": "compaction", "signature": "compaction-signature" },
                            },
                        }],
                    },
                    {
                        "role": "user",
                        "content": [{ "type": "text", "text": "Continue the conversation." }],
                    },
                ],
            }),
        )
        .await;

                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({
                        "messages": [
                            {
                                "role": "assistant",
                                "content": [{
                                    "type": "compaction",
                                    "content": "Summary of the conversation.",
                                    "signature": "compaction-signature",
                                }],
                            },
                            {
                                "role": "user",
                                "content": [{ "type": "text", "text": "Continue the conversation." }],
                            },
                        ],
                    }),
                );
                assert!(seen.headers["anthropic-beta"].contains("compact-2026-09-04"));
            }
        }

        mod safeguards {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › safeguards › should send safeguards in request body and add the beta
            #[tokio::test]
            async fn should_send_safeguards_in_request_body_and_add_the_beta() {
                let (_, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({
                        "prompt": anthropic_test_prompt(),
                        "providerOptions": {
                            CANONICAL: {
                                "safeguards": [{
                                    "type": "dangerous_tool_use",
                                    "classifierContext": { "v": 1, "permission_mode": "auto" },
                                }],
                            },
                        },
                    }),
                )
                .await;

                assert_anthropic_match_object(
                    &anthropic_request_body(&seen),
                    &json!({
                        "safeguards": [{
                            "type": "dangerous_tool_use",
                            "classifier_context": { "v": 1, "permission_mode": "auto" },
                        }],
                    }),
                );
                assert!(seen.headers["anthropic-beta"].contains("dangerous-tool-use-2026-09-03"));
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › safeguards › should not send safeguards or the beta when the option is absent
            #[tokio::test]
            async fn should_not_send_safeguards_or_the_beta_when_the_option_is_absent() {
                let (_, seen) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert!(anthropic_request_body(&seen).get("safeguards").is_none());
                assert!(
                    !seen
                        .headers
                        .get("anthropic-beta")
                        .map(String::as_str)
                        .unwrap_or_default()
                        .contains("dangerous-tool-use")
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › safeguards › should expose safeguard_results as provider metadata
            #[tokio::test]
            async fn should_expose_safeguard_results_as_provider_metadata() {
                let safeguard_results = json!([{
                    "type": "dangerous_tool_use",
                    "status": {
                        "type": "available",
                        "tool_uses": {
                            "toolu_01": {
                                "type": "evaluated",
                                "outcome": "flagged",
                                "explanation": "[Data Exfiltration]",
                            },
                        },
                    },
                }]);
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&json!({
                        "id": "msg_123",
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": "toolu_01",
                            "name": "Bash",
                            "input": { "command": "echo hello" },
                        }],
                        "model": "claude-3-haiku-20240307",
                        "stop_reason": "tool_use",
                        "stop_sequence": null,
                        "usage": { "input_tokens": 100, "output_tokens": 50 },
                        "safeguard_results": safeguard_results,
                    })),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert_eq!(
                    anthropic_to_json(&result.provider_metadata)[CANONICAL]["safeguardResults"],
                    safeguard_results
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doGenerate › safeguards › should omit safeguardResults when the response has none
            #[tokio::test]
            async fn should_omit_safeguard_results_when_the_response_has_none() {
                let (result, _) = anthropic_generate_default(
                    anthropic_json_response(&anthropic_json_fixture!("anthropic-text")),
                    json!({ "prompt": anthropic_test_prompt() }),
                )
                .await;

                assert!(
                    anthropic_to_json(&result.provider_metadata)[CANONICAL]
                        .get("safeguardResults")
                        .is_none()
                );
            }
        }
    }

    mod initial_per_message_effort {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › initial per-message effort › should send initial effort via $method with request effort $requestEffort
        #[tokio::test]
        async fn should_send_initial_effort_via_method_with_request_effort_request_effort() {
            for (method, request_effort) in [
                ("generate", None),
                ("generate", Some("high")),
                ("stream", None),
                ("stream", Some("high")),
            ] {
                let mut options = json!({
                    "prompt": [
                        { "role": "system", "content": "initial instructions" },
                        {
                            "role": "system",
                            "content": "",
                            "providerOptions": { CANONICAL: { "effort": "low" } },
                        },
                        {
                            "role": "system",
                            "content": "",
                            "providerOptions": { CANONICAL: { "effort": "medium" } },
                        },
                        test_prompt()[0],
                    ],
                });
                if let Some(effort) = request_effort {
                    options["providerOptions"] = json!({ CANONICAL: { "effort": effort } });
                }

                let mock;
                if method == "generate" {
                    mock = server(vec![json_fixture(include_str!(
                        "fixtures/anthropic-text.json"
                    ))]);
                    let result =
                        generate(&test_provider(&mock).messages("claude-fable-5-1"), options).await;
                    assert_eq!(warnings_json(&result.warnings), json!([]));
                } else {
                    mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-message-delta-input-tokens.chunks.txt"
                    ))]);
                    let chunks =
                        stream_parts(&test_provider(&mock).messages("claude-fable-5-1"), options)
                            .await;
                    assert_eq!(chunks[0], json!({ "type": "stream-start", "warnings": [] }));
                    assert!(
                        !chunks.iter().any(|chunk| chunk["type"] == "error"),
                        "{chunks:?}"
                    );
                }

                let body = request_body(&mock, 0);
                assert_eq!(
                    body["system"],
                    json!([{ "type": "text", "text": "initial instructions" }])
                );
                assert_eq!(
                    body["messages"],
                    json!([
                        { "role": "system", "content": [], "output_config": { "effort": "low" } },
                        { "role": "system", "content": [], "output_config": { "effort": "medium" } },
                        test_prompt()[0],
                    ])
                );
                assert_eq!(
                    body.pointer("/output_config/effort")
                        .and_then(Value::as_str),
                    request_effort,
                    "{method} {request_effort:?}"
                );
                assert!(
                    request_header(&mock, 0, "anthropic-beta")
                        .unwrap_or_default()
                        .contains("mid-conversation-output-config-2026-07-01"),
                    "{method} {request_effort:?}"
                );
            }
        }
    }

    mod pruned_programmatic_tool_history {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › pruned programmatic tool history › should omit orphaned callers and expose the warning for %s
        #[tokio::test]
        async fn should_omit_orphaned_callers_and_expose_the_warning_for_s() {
            for method in ["generate", "stream"] {
                let options = json!({
                    "prompt": [
                        {
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "tool-call",
                                    "toolCallId": "lookup-call",
                                    "toolName": "lookup",
                                    "input": { "ticker": "AAPL" },
                                    "providerOptions": {
                                        CANONICAL: {
                                            "caller": {
                                                "type": "code_execution_20250825",
                                                "toolId": "pruned-source",
                                            },
                                        },
                                    },
                                },
                            ],
                        },
                        {
                            "role": "tool",
                            "content": [
                                {
                                    "type": "tool-result",
                                    "toolCallId": "lookup-call",
                                    "toolName": "lookup",
                                    "output": { "type": "text", "value": "185.42" },
                                },
                            ],
                        },
                        test_prompt()[0],
                    ],
                });
                let warnings = json!([
                    {
                        "type": "other",
                        "message": "Omitted caller metadata for tool lookup-call because source code execution tool pruned-source is missing from the conversation history.",
                    },
                ]);

                let mock;
                if method == "generate" {
                    mock = server(vec![json_fixture(include_str!(
                        "fixtures/anthropic-text.json"
                    ))]);
                    let result = generate(&test_model(&mock), options).await;
                    assert_eq!(warnings_json(&result.warnings), warnings, "{method}");
                } else {
                    mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-text.chunks.txt"
                    ))]);
                    let chunks = stream_parts(&test_model(&mock), options).await;
                    assert_eq!(
                        chunks[0],
                        json!({ "type": "stream-start", "warnings": warnings })
                    );
                    assert!(
                        !chunks.iter().any(|chunk| chunk["type"] == "error"),
                        "{chunks:?}"
                    );
                }

                let body = request_body(&mock, 0);
                assert_eq!(
                    body["messages"][0]["content"],
                    json!([
                        {
                            "type": "tool_use",
                            "id": "lookup-call",
                            "name": "lookup",
                            "input": { "ticker": "AAPL" },
                        },
                    ]),
                    "{method}"
                );
                assert_eq!(
                    body["messages"][1]["content"][0],
                    json!({
                        "type": "tool_result",
                        "tool_use_id": "lookup-call",
                        "content": "185.42",
                    }),
                    "{method}"
                );
            }
        }
    }

    mod do_stream {
        use super::*;

        mod json_schema_response_format_unsupported_model {
            use super::*;

            /// The `beforeEach`: the call and its stream.
            async fn setup() -> (Arc<MockFetch>, Vec<Value>) {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-json-tool.1.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "responseFormat": {
                            "type": "json",
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "name": {
                                        "type": "string",
                                    },
                                },
                                "required": [
                                    "name",
                                ],
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        },
                    }),
                )
                .await;
                (mock, parts)
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › json schema response format (unsupported model) › should pass json schema response format as a tool
            #[tokio::test]
            async fn should_pass_json_schema_response_format_as_a_tool() {
                let (mock, _) = setup().await;
                assert_equals(
                    &request_body(&mock, 0),
                    &json!({
                        "max_tokens": 4096,
                        "messages": [
                            {
                                "content": [
                                    {
                                        "text": "Hello",
                                        "type": "text",
                                    },
                                ],
                                "role": "user",
                            },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stream": true,
                        "tool_choice": {
                            "disable_parallel_tool_use": true,
                            "type": "any",
                        },
                        "tools": [
                            {
                                "description": "Respond with a JSON object.",
                                "eager_input_streaming": true,
                                "input_schema": {
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                    "additionalProperties": false,
                                    "properties": {
                                        "name": {
                                            "type": "string",
                                        },
                                    },
                                    "required": [
                                        "name",
                                    ],
                                    "type": "object",
                                },
                                "name": "json",
                            },
                        ],
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › json schema response format (unsupported model) › should stream the response
            #[tokio::test]
            async fn should_stream_the_response() {
                let (_, result) = setup().await;
                assert_equals(
                    &json!(result),
                    &json!([
                        {
                            "type": "stream-start",
                            "warnings": [],
                        },
                        {
                            "id": "msg_01K2JbSUMYhez5RHoK9ZCj9U",
                            "modelId": "claude-haiku-4-5-20251001",
                            "type": "response-metadata",
                        },
                        {
                            "id": "0",
                            "type": "text-start",
                        },
                        {
                            "delta": r#"{"elements": [{"location": "San Francisco", "temperature": 58, "condition": "sunny"}]"#,
                            "id": "0",
                            "type": "text-delta",
                        },
                        {
                            "delta": "}",
                            "id": "0",
                            "type": "text-delta",
                        },
                        {
                            "id": "0",
                            "type": "text-end",
                        },
                        {
                            "finishReason": {
                                "raw": "tool_use",
                                "unified": "stop",
                            },
                            "providerMetadata": {
                                CANONICAL: {
                                    "container": null,
                                    "contextManagement": null,
                                    "iterations": null,
                                    "stopSequence": null,
                                    "usage": {
                                        "cache_creation": {
                                            "ephemeral_1h_input_tokens": 0,
                                            "ephemeral_5m_input_tokens": 0,
                                        },
                                        "cache_creation_input_tokens": 0,
                                        "cache_read_input_tokens": 0,
                                        "input_tokens": 849,
                                        "output_tokens": 47,
                                        "service_tier": "standard",
                                    },
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "cacheWrite": 0,
                                    "noCache": 849,
                                    "total": 849,
                                },
                                "outputTokens": {
                                    "total": 47,
                                },
                                "raw": {
                                    "cache_creation": {
                                        "ephemeral_1h_input_tokens": 0,
                                        "ephemeral_5m_input_tokens": 0,
                                    },
                                    "cache_creation_input_tokens": 0,
                                    "cache_read_input_tokens": 0,
                                    "input_tokens": 849,
                                    "output_tokens": 47,
                                    "service_tier": "standard",
                                },
                            },
                        },
                    ]),
                );
            }
        }

        mod json_schema_response_format_with_text_content_prefix {
            use super::*;

            async fn setup() -> (Arc<MockFetch>, Vec<Value>) {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-json-tool.2.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "responseFormat": {
                            "type": "json",
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "name": {
                                        "type": "string",
                                    },
                                },
                                "required": [
                                    "name",
                                ],
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        },
                    }),
                )
                .await;
                (mock, parts)
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › json schema response format with text content prefix › should pass json schema response format as a tool
            #[tokio::test]
            async fn should_pass_json_schema_response_format_as_a_tool() {
                let (mock, _) = setup().await;
                assert_equals(
                    &request_body(&mock, 0),
                    &json!({
                        "max_tokens": 4096,
                        "messages": [
                            {
                                "content": [
                                    {
                                        "text": "Hello",
                                        "type": "text",
                                    },
                                ],
                                "role": "user",
                            },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stream": true,
                        "tool_choice": {
                            "disable_parallel_tool_use": true,
                            "type": "any",
                        },
                        "tools": [
                            {
                                "description": "Respond with a JSON object.",
                                "eager_input_streaming": true,
                                "input_schema": {
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                    "additionalProperties": false,
                                    "properties": {
                                        "name": {
                                            "type": "string",
                                        },
                                    },
                                    "required": [
                                        "name",
                                    ],
                                    "type": "object",
                                },
                                "name": "json",
                            },
                        ],
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › json schema response format with text content prefix › should stream the response
            #[tokio::test]
            async fn should_stream_the_response() {
                let (_, result) = setup().await;
                assert_equals(
                    &json!(result),
                    &json!([
                        {
                            "type": "stream-start",
                            "warnings": [],
                        },
                        {
                            "id": "msg_01K2JbSUMYhez5RHoK9ZCj9U",
                            "modelId": "claude-haiku-4-5-20251001",
                            "type": "response-metadata",
                        },
                        {
                            "id": "1",
                            "type": "text-start",
                        },
                        {
                            "delta": r#"{"elements": [{"location": "San Francisco", "temperature": 58, "condition": "sunny"}]"#,
                            "id": "1",
                            "type": "text-delta",
                        },
                        {
                            "delta": "}",
                            "id": "1",
                            "type": "text-delta",
                        },
                        {
                            "id": "1",
                            "type": "text-end",
                        },
                        {
                            "finishReason": {
                                "raw": "tool_use",
                                "unified": "stop",
                            },
                            "providerMetadata": {
                                CANONICAL: {
                                    "container": null,
                                    "contextManagement": null,
                                    "iterations": null,
                                    "stopSequence": null,
                                    "usage": {
                                        "cache_creation": {
                                            "ephemeral_1h_input_tokens": 0,
                                            "ephemeral_5m_input_tokens": 0,
                                        },
                                        "cache_creation_input_tokens": 0,
                                        "cache_read_input_tokens": 0,
                                        "input_tokens": 849,
                                        "output_tokens": 47,
                                        "service_tier": "standard",
                                    },
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "cacheWrite": 0,
                                    "noCache": 849,
                                    "total": 849,
                                },
                                "outputTokens": {
                                    "total": 47,
                                },
                                "raw": {
                                    "cache_creation": {
                                        "ephemeral_1h_input_tokens": 0,
                                        "ephemeral_5m_input_tokens": 0,
                                    },
                                    "cache_creation_input_tokens": 0,
                                    "cache_read_input_tokens": 0,
                                    "input_tokens": 849,
                                    "output_tokens": 47,
                                    "service_tier": "standard",
                                },
                            },
                        },
                    ]),
                );
            }
        }

        mod json_schema_response_format_with_other_tool_response_unsupported_model {
            use super::*;

            async fn setup() -> (Arc<MockFetch>, Vec<Value>) {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-json-other-tool.1.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "function",
                                "name": "weather",
                                "description": "Get the weather in a location",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "location": {
                                            "type": "string",
                                        },
                                    },
                                    "required": [
                                        "location",
                                    ],
                                    "additionalProperties": false,
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                },
                            },
                        ],
                        "responseFormat": {
                            "type": "json",
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "weather": {
                                        "type": "string",
                                    },
                                    "temperature": {
                                        "type": "number",
                                    },
                                },
                                "required": [
                                    "weather",
                                    "temperature",
                                ],
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        },
                    }),
                )
                .await;
                (mock, parts)
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › json schema response format with other tool response (unsupported model) › should pass json schema response format as a tool
            #[tokio::test]
            async fn should_pass_json_schema_response_format_as_a_tool() {
                let (mock, _) = setup().await;
                assert_equals(
                    &request_body(&mock, 0),
                    &json!({
                        "max_tokens": 4096,
                        "messages": [
                            {
                                "content": [
                                    {
                                        "text": "Hello",
                                        "type": "text",
                                    },
                                ],
                                "role": "user",
                            },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stream": true,
                        "tool_choice": {
                            "disable_parallel_tool_use": true,
                            "type": "any",
                        },
                        "tools": [
                            {
                                "description": "Get the weather in a location",
                                "eager_input_streaming": true,
                                "input_schema": {
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                    "additionalProperties": false,
                                    "properties": {
                                        "location": {
                                            "type": "string",
                                        },
                                    },
                                    "required": [
                                        "location",
                                    ],
                                    "type": "object",
                                },
                                "name": "weather",
                            },
                            {
                                "description": "Respond with a JSON object.",
                                "eager_input_streaming": true,
                                "input_schema": {
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                    "additionalProperties": false,
                                    "properties": {
                                        "temperature": {
                                            "type": "number",
                                        },
                                        "weather": {
                                            "type": "string",
                                        },
                                    },
                                    "required": [
                                        "weather",
                                        "temperature",
                                    ],
                                    "type": "object",
                                },
                                "name": "json",
                            },
                        ],
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › json schema response format with other tool response (unsupported model) › should stream the tool call
            #[tokio::test]
            async fn should_stream_the_tool_call() {
                let (_, parts) = setup().await;
                assert_equals(
                    &json!(parts),
                    &json!([
                        {
                            "type": "stream-start",
                            "warnings": [],
                        },
                        {
                            "id": "msg_01CD3XaZfhNabxRt1SG5ybtK",
                            "modelId": "claude-haiku-4-5-20251001",
                            "type": "response-metadata",
                        },
                        {
                            "id": "toolu_019Zvehfe1XQWweT1pm7okyt",
                            "toolName": "weather",
                            "type": "tool-input-start",
                        },
                        {
                            "delta": r#"{"location": "San Francisco"#,
                            "id": "toolu_019Zvehfe1XQWweT1pm7okyt",
                            "type": "tool-input-delta",
                        },
                        {
                            "delta": r#""}"#,
                            "id": "toolu_019Zvehfe1XQWweT1pm7okyt",
                            "type": "tool-input-delta",
                        },
                        {
                            "id": "toolu_019Zvehfe1XQWweT1pm7okyt",
                            "type": "tool-input-end",
                        },
                        {
                            "input": r#"{"location": "San Francisco"}"#,
                            "toolCallId": "toolu_019Zvehfe1XQWweT1pm7okyt",
                            "toolName": "weather",
                            "type": "tool-call",
                        },
                        {
                            "finishReason": {
                                "raw": "tool_use",
                                "unified": "tool-calls",
                            },
                            "providerMetadata": {
                                CANONICAL: {
                                    "container": null,
                                    "contextManagement": null,
                                    "iterations": null,
                                    "stopSequence": null,
                                    "usage": {
                                        "cache_creation": {
                                            "ephemeral_1h_input_tokens": 0,
                                            "ephemeral_5m_input_tokens": 0,
                                        },
                                        "cache_creation_input_tokens": 0,
                                        "cache_read_input_tokens": 0,
                                        "input_tokens": 843,
                                        "output_tokens": 28,
                                        "service_tier": "standard",
                                    },
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "cacheWrite": 0,
                                    "noCache": 843,
                                    "total": 843,
                                },
                                "outputTokens": {
                                    "total": 28,
                                },
                                "raw": {
                                    "cache_creation": {
                                        "ephemeral_1h_input_tokens": 0,
                                        "ephemeral_5m_input_tokens": 0,
                                    },
                                    "cache_creation_input_tokens": 0,
                                    "cache_read_input_tokens": 0,
                                    "input_tokens": 843,
                                    "output_tokens": 28,
                                    "service_tier": "standard",
                                },
                            },
                        },
                    ]),
                );
            }
        }

        mod json_schema_response_format_with_output_format_supported_model {
            use super::*;

            async fn setup() -> (Arc<MockFetch>, Vec<Value>) {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-json-output-format.1.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_provider(&mock).messages("claude-sonnet-4-5"),
                    json!({
                        "prompt": test_prompt(),
                        "responseFormat": {
                            "type": "json",
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "characters": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "name": {
                                                    "type": "string",
                                                },
                                                "class": {
                                                    "type": "string",
                                                },
                                                "description": {
                                                    "type": "string",
                                                },
                                            },
                                            "required": [
                                                "name",
                                                "class",
                                                "description",
                                            ],
                                            "additionalProperties": false,
                                        },
                                    },
                                },
                                "required": [
                                    "characters",
                                ],
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        },
                    }),
                )
                .await;
                (mock, parts)
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › json schema response format with output format (supported model) › should pass json schema response format as output_config.format
            #[tokio::test]
            async fn should_pass_json_schema_response_format_as_output_config_format() {
                let (mock, _) = setup().await;
                assert_equals(
                    &request_body(&mock, 0),
                    &json!({
                        "max_tokens": 64000,
                        "messages": [
                            {
                                "content": [
                                    {
                                        "text": "Hello",
                                        "type": "text",
                                    },
                                ],
                                "role": "user",
                            },
                        ],
                        "model": "claude-sonnet-4-5",
                        "output_config": {
                            "format": {
                                "schema": {
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                    "additionalProperties": false,
                                    "properties": {
                                        "characters": {
                                            "items": {
                                                "additionalProperties": false,
                                                "properties": {
                                                    "class": {
                                                        "type": "string",
                                                    },
                                                    "description": {
                                                        "type": "string",
                                                    },
                                                    "name": {
                                                        "type": "string",
                                                    },
                                                },
                                                "required": [
                                                    "name",
                                                    "class",
                                                    "description",
                                                ],
                                                "type": "object",
                                            },
                                            "type": "array",
                                        },
                                    },
                                    "required": [
                                        "characters",
                                    ],
                                    "type": "object",
                                },
                                "type": "json_schema",
                            },
                        },
                        "stream": true,
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › json schema response format with output format (supported model) › should stream the text output
            #[tokio::test]
            async fn should_stream_the_text_output() {
                let (_, parts) = setup().await;
                assert_equals(
                    &json!(parts),
                    &snapshot(include_str!("snapshots/stream_json_output_format.json")),
                );
            }
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream text deltas
        #[tokio::test]
        async fn should_stream_text_deltas() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", "}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"World!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let parts = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "type": "text-start",
                    },
                    {
                        "delta": "Hello",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "delta": ", ",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "delta": "World!",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "id": "0",
                        "type": "text-end",
                    },
                    {
                        "finishReason": {
                            "raw": "end_turn",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "cacheWrite": 0,
                                "noCache": 17,
                                "total": 17,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should use input_tokens from message_delta when different from message_start
        #[tokio::test]
        async fn should_use_input_tokens_from_message_delta_when_different_from_message_start() {
            // Fixture has message_start.usage.input_tokens=43, message_delta.usage.input_tokens=61
            // The final usage should use the value from message_delta (61)
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-message-delta-input-tokens.chunks.txt"
            ))]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            let finish_part = find_part(&result, "finish").expect("finish part");
            assert_match_object(
                finish_part,
                &json!({
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "total": 61,
                        },
                        "outputTokens": {
                            "total": 2,
                        },
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should map a streamed classifier refusal to content-filter and expose stop details
        #[tokio::test]
        async fn should_map_a_streamed_classifier_refusal_to_content_filter_and_expose_stop_details()
         {
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-refusal.chunks.txt"
            ))]);
            let result = stream_parts(
                &test_provider(&mock).messages("claude-fable-5"),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            let finish_part = find_part(&result, "finish").expect("finish part");
            assert_match_object(
                finish_part,
                &json!({
                    "type": "finish",
                    "finishReason": {
                        "unified": "content-filter",
                        "raw": "refusal",
                    },
                }),
            );
            assert_equals(
                &finish_part["providerMetadata"][CANONICAL]["stopDetails"],
                &json!({
                    "category": "cyber",
                    "explanation": "This request triggered restrictions on violative cyber content and was blocked under Anthropic's Usage Policy.",
                    "recommendedModel": "claude-fable-5",
                    "type": "refusal",
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should preserve the streamed fallback content block and surface the fallback iteration
        #[tokio::test]
        async fn should_preserve_the_streamed_fallback_content_block_and_surface_the_fallback_iteration()
         {
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-fallback.chunks.txt"
            ))]);
            let result = stream_parts(
                &test_provider(&mock).messages("claude-fable-5"),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            assert_contains_equal(
                &result,
                &json!({
                    "type": "custom",
                    "kind": "anthropic.fallback",
                    "providerMetadata": {
                        CANONICAL: {
                            "type": "fallback",
                            "from": {
                                "model": "claude-fable-5",
                            },
                            "to": {
                                "model": "claude-opus-4-8",
                            },
                        },
                    },
                }),
            );
            assert_eq!(
                join_deltas(&result, "text-delta", None),
                "The printing press was invented by Johannes Gutenberg around 1440."
            );
            let finish_part = find_part(&result, "finish").expect("finish part");
            assert_equals(
                &finish_part["providerMetadata"][CANONICAL]["iterations"],
                &json!([
                    {
                        "inputTokens": 408,
                        "model": "claude-fable-5",
                        "outputTokens": 0,
                        "type": "message",
                    },
                    {
                        "inputTokens": 412,
                        "model": "claude-opus-4-8",
                        "outputTokens": 264,
                        "type": "fallback_message",
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should preserve a mid-output fallback boundary between reasoning blocks
        #[tokio::test]
        async fn should_preserve_a_mid_output_fallback_boundary_between_reasoning_blocks() {
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-fallback-mid-output.chunks.txt"
            ))]);
            let result = stream_parts(
                &test_provider(&mock).messages("claude-opus-5-5"),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            let index = |pred: &dyn Fn(&Value) -> bool| result.iter().position(pred);
            let first_reasoning_end =
                index(&|part| part["type"] == "reasoning-end" && part["id"] == "0");
            let fallback =
                index(&|part| part["type"] == "custom" && part["kind"] == "anthropic.fallback");
            let second_reasoning_start =
                index(&|part| part["type"] == "reasoning-start" && part["id"] == "2");

            assert!(first_reasoning_end.is_some(), "{result:#?}");
            assert!(fallback > first_reasoning_end, "{result:#?}");
            assert!(second_reasoning_start > fallback, "{result:#?}");
            assert_equals(
                &result[fallback.unwrap()],
                &json!({
                    "type": "custom",
                    "kind": "anthropic.fallback",
                    "providerMetadata": {
                        CANONICAL: {
                            "type": "fallback",
                            "from": {
                                "model": "claude-opus-5-5",
                            },
                            "to": {
                                "model": "claude-opus-4-8",
                            },
                        },
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream reasoning deltas
        #[tokio::test]
        async fn should_stream_reasoning_deltas() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I am"}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"thinking..."}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"1234567890"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hello, World!"}}"#,
                r#"{"type":"content_block_stop","index":1}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let parts = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "type": "reasoning-start",
                    },
                    {
                        "delta": "I am",
                        "id": "0",
                        "type": "reasoning-delta",
                    },
                    {
                        "delta": "thinking...",
                        "id": "0",
                        "type": "reasoning-delta",
                    },
                    {
                        "delta": "",
                        "id": "0",
                        "providerMetadata": {
                            CANONICAL: {
                                "signature": "1234567890",
                            },
                        },
                        "type": "reasoning-delta",
                    },
                    {
                        "id": "0",
                        "type": "reasoning-end",
                    },
                    {
                        "id": "1",
                        "type": "text-start",
                    },
                    {
                        "delta": "Hello, World!",
                        "id": "1",
                        "type": "text-delta",
                    },
                    {
                        "id": "1",
                        "type": "text-end",
                    },
                    {
                        "finishReason": {
                            "raw": "end_turn",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "cacheWrite": 0,
                                "noCache": 17,
                                "total": 17,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream thinking updates between tool calls
        #[tokio::test]
        async fn should_stream_thinking_updates_between_tool_calls() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_updates","type":"message","role":"assistant","content":[],"model":"claude-fable-5","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"First update"}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-1"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool-1","name":"lookup","input":{}}}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
                r#"{"type":"content_block_stop","index":1}"#,
                r#"{"type":"content_block_start","index":2,"content_block":{"type":"thinking","thinking":""}}"#,
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"thinking_delta","thinking":"Second update"}}"#,
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"signature_delta","signature":"sig-2"}}"#,
                r#"{"type":"content_block_stop","index":2}"#,
                r#"{"type":"content_block_start","index":3,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":3,"delta":{"type":"text_delta","text":"Done"}}"#,
                r#"{"type":"content_block_stop","index":3}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":20}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let result = stream_parts(
                &test_provider(&mock).messages("claude-fable-5"),
                json!({
                    "prompt": test_prompt(),
                    "tools": [
                        {
                            "type": "function",
                            "name": "lookup",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                            },
                        },
                    ],
                    "providerOptions": {
                        CANONICAL: {
                            "thinking": {
                                "type": "adaptive",
                                "display": "updates",
                            },
                        },
                    },
                }),
            )
            .await;
            let reasoning: Vec<Value> = result
                .iter()
                .filter_map(|part| match part["type"].as_str() {
                    Some("reasoning-start") => {
                        Some(json!({ "type": part["type"], "id": part["id"] }))
                    }
                    Some("reasoning-delta") if part["delta"] != "" => Some(
                        json!({ "type": part["type"], "id": part["id"], "delta": part["delta"] }),
                    ),
                    _ => None,
                })
                .collect();
            assert_equals(
                &json!(reasoning),
                &json!([
                    {
                        "type": "reasoning-start",
                        "id": "0",
                    },
                    {
                        "type": "reasoning-delta",
                        "id": "0",
                        "delta": "First update",
                    },
                    {
                        "type": "reasoning-start",
                        "id": "2",
                    },
                    {
                        "type": "reasoning-delta",
                        "id": "2",
                        "delta": "Second update",
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream redacted reasoning
        #[tokio::test]
        async fn should_stream_redacted_reasoning() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"redacted-thinking-data"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hello, World!"}}"#,
                r#"{"type":"content_block_stop","index":1}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let parts = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "providerMetadata": {
                            CANONICAL: {
                                "redactedData": "redacted-thinking-data",
                            },
                        },
                        "type": "reasoning-start",
                    },
                    {
                        "id": "0",
                        "type": "reasoning-end",
                    },
                    {
                        "id": "1",
                        "type": "text-start",
                    },
                    {
                        "delta": "Hello, World!",
                        "id": "1",
                        "type": "text-delta",
                    },
                    {
                        "id": "1",
                        "type": "text-end",
                    },
                    {
                        "finishReason": {
                            "raw": "end_turn",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "cacheWrite": 0,
                                "noCache": 17,
                                "total": 17,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should ignore signatures on text deltas
        #[tokio::test]
        async fn should_ignore_signatures_on_text_deltas() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, World!"}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"1234567890"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let parts = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "type": "text-start",
                    },
                    {
                        "delta": "Hello, World!",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "id": "0",
                        "type": "text-end",
                    },
                    {
                        "finishReason": {
                            "raw": "end_turn",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "cacheWrite": 0,
                                "noCache": 17,
                                "total": 17,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should parse context_management from streaming response
        #[tokio::test]
        async fn should_parse_context_management_from_streaming_response() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":50},"context_management":{"applied_edits":[{"type":"clear_tool_uses_20250919","cleared_tool_uses":5,"cleared_input_tokens":10000}]}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            let finish_part = find_part(&result, "finish").expect("finish part");
            assert_equals(
                &finish_part["providerMetadata"][CANONICAL]["contextManagement"],
                &json!({
                    "appliedEdits": [
                        {
                            "type": "clear_tool_uses_20250919",
                            "clearedToolUses": 5,
                            "clearedInputTokens": 10000,
                        },
                    ],
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should expose the last non-null safeguard_results from message_delta
        #[tokio::test]
        async fn should_expose_the_last_non_null_safeguard_results_from_message_delta() {
            let safeguard_results = json!([
                {
                    "type": "dangerous_tool_use",
                    "status": {
                        "type": "available",
                        "tool_uses": {
                            "toolu_01": {
                                "type": "evaluated",
                                "outcome": "not_flagged",
                            },
                        },
                    },
                },
            ]);
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01","name":"Bash","input":{}}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"echo hello\"}"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null,"safeguard_results":null},"usage":{"output_tokens":50}}"#,
                &format!(
                    r#"{{"type":"message_delta","delta":{{"stop_reason":"tool_use","stop_sequence":null,"safeguard_results":{}}},"usage":{{"output_tokens":50}}}}"#,
                    serde_json::to_string(&safeguard_results).unwrap()
                ),
                r#"{"type":"message_stop"}"#,
            ])]);
            let result = stream_parts(
                &test_model(&mock),
                json!({
                    "prompt": test_prompt(),
                    "providerOptions": {
                        CANONICAL: {
                            "safeguards": [
                                {
                                    "type": "dangerous_tool_use",
                                },
                            ],
                        },
                    },
                }),
            )
            .await;
            let finish_part = find_part(&result, "finish").expect("finish part");

            assert_match_object(
                &request_body(&mock, 0),
                &json!({
                    "safeguards": [
                        {
                            "type": "dangerous_tool_use",
                        },
                    ],
                }),
            );
            assert_equals(
                &finish_part["finishReason"],
                &json!({
                    "unified": "tool-calls",
                    "raw": "tool_use",
                }),
            );
            assert_equals(
                &finish_part["providerMetadata"][CANONICAL]["safeguardResults"],
                &safeguard_results,
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should omit safeguardResults when no message_delta carries them
        #[tokio::test]
        async fn should_omit_safeguard_results_when_no_message_delta_carries_them() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null,"safeguard_results":null},"usage":{"output_tokens":50}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            let finish_part = find_part(&result, "finish").expect("finish part");
            assert!(
                finish_part["providerMetadata"][CANONICAL]
                    .get("safeguardResults")
                    .is_none(),
                "{finish_part:#}"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream compaction content blocks with provider metadata
        #[tokio::test]
        async fn should_stream_compaction_content_blocks_with_provider_metadata() {
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-compaction.1.chunks.txt"
            ))]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;

            let compaction_text_start = result
                .iter()
                .find(|part| {
                    part["type"] == "text-start"
                        && part["id"] == "0"
                        && part["providerMetadata"][CANONICAL]["type"] == "compaction"
                })
                .expect("compaction text-start");
            assert_equals(
                &compaction_text_start["providerMetadata"],
                &json!({
                    CANONICAL: {
                        "type": "compaction",
                    },
                }),
            );

            let compaction_deltas: Vec<_> = result
                .iter()
                .filter(|part| part["type"] == "text-delta" && part["id"] == "0")
                .collect();
            assert!(!compaction_deltas.is_empty());
            let all_compaction_text = join_deltas(&result, "text-delta", Some("0"));
            assert!(all_compaction_text.contains("## Summary of Conversation"));
            assert!(all_compaction_text.contains("React vs Vue.js"));

            let regular_text_start = result.iter().find(|part| {
                part["type"] == "text-start"
                    && part["id"] == "1"
                    && part.get("providerMetadata").is_none()
            });
            assert!(regular_text_start.is_some(), "{result:#?}");
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream complete signed on-demand compaction blocks
        #[tokio::test]
        async fn should_stream_complete_signed_on_demand_compaction_blocks() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_compaction","type":"message","role":"assistant","content":[],"model":"claude-opus-5","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"compaction","content":"Summary of the conversation.","signature":"compaction-signature"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"compaction","stop_sequence":null},"usage":{"output_tokens":0,"iterations":[{"type":"compaction","input_tokens":120,"output_tokens":30}]}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_contains_equal(
                &result,
                &json!({
                    "type": "text-start",
                    "id": "0",
                    "providerMetadata": {
                        CANONICAL: {
                            "type": "compaction",
                            "signature": "compaction-signature",
                        },
                    },
                }),
            );
            assert_contains_equal(
                &result,
                &json!({
                    "type": "text-delta",
                    "id": "0",
                    "delta": "Summary of the conversation.",
                }),
            );
            assert_contains_equal(
                &result,
                &json!({
                    "type": "text-end",
                    "id": "0",
                }),
            );
            assert_contains_equal(
                &result,
                &json!({
                    "$objectContaining": {
                        "type": "finish",
                        "finishReason": {
                            "unified": "other",
                            "raw": "compaction",
                        },
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should parse iterations from streaming compaction response
        #[tokio::test]
        async fn should_parse_iterations_from_streaming_compaction_response() {
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-compaction.1.chunks.txt"
            ))]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            let finish_part = find_part(&result, "finish").expect("finish part");
            assert_equals(
                &finish_part["providerMetadata"][CANONICAL]["iterations"],
                &json!([
                    {
                        "type": "compaction",
                        "inputTokens": 60385,
                        "outputTokens": 522,
                    },
                    {
                        "type": "message",
                        "inputTokens": 612,
                        "outputTokens": 2819,
                    },
                ]),
            );
            assert_match_object(
                &finish_part["usage"],
                &json!({
                    "inputTokens": { "total": 60385 + 612 },
                    "outputTokens": { "total": 522 + 2819 },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should handle compaction_delta with null content
        #[tokio::test]
        async fn should_handle_compaction_delta_with_null_content() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_test","type":"message","role":"assistant","model":"claude-3-haiku-20240307","stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":0},"content":[],"stop_reason":null}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"compaction","content":null}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":null}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":"Summary of conversation."}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":10}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;

            // No parsing errors should occur (schema must accept null content)
            let errors = filter_parts(&result, "error");
            assert_eq!(errors.len(), 0, "{errors:#?}");

            let compaction_deltas: Vec<_> = result
                .iter()
                .filter(|part| part["type"] == "text-delta" && part["id"] == "0")
                .collect();
            // null content delta should be skipped, only the string delta comes through
            assert_eq!(compaction_deltas.len(), 1, "{compaction_deltas:#?}");
            assert_eq!(compaction_deltas[0]["delta"], "Summary of conversation.");
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream clear_tool_uses response with context_management
        #[tokio::test]
        async fn should_stream_clear_tool_uses_response_with_context_management() {
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-clear-tool-uses.1.chunks.txt"
            ))]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;

            let text_deltas: Vec<_> = result
                .iter()
                .filter(|part| part["type"] == "text-delta" && part["id"] == "0")
                .collect();
            assert!(!text_deltas.is_empty());
            let full_text = join_deltas(&result, "text-delta", Some("0"));
            assert!(full_text.contains("Here's a comparison of the weather in both cities"));

            let finish_part = find_part(&result, "finish").expect("finish part");
            assert_equals(
                &finish_part["providerMetadata"][CANONICAL]["contextManagement"],
                &json!({
                    "appliedEdits": [],
                }),
            );
            // The property matchers, then the snapshot.
            assert_match_object(
                &finish_part["usage"],
                &json!({
                    "inputTokens": {
                        "total": 859,
                    },
                    "outputTokens": {
                        "total": 122,
                    },
                }),
            );
            assert_equals(
                &finish_part["usage"],
                &json!({
                    "inputTokens": {
                        "cacheRead": 0,
                        "cacheWrite": 0,
                        "noCache": 859,
                        "total": 859,
                    },
                    "outputTokens": {
                        "total": 122,
                    },
                    "raw": {
                        "cache_creation": {
                            "ephemeral_1h_input_tokens": 0,
                            "ephemeral_5m_input_tokens": 0,
                        },
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "inference_geo": "not_available",
                        "input_tokens": 859,
                        "output_tokens": 122,
                        "service_tier": "standard",
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream clear_thinking response with thinking and text blocks
        #[tokio::test]
        async fn should_stream_clear_thinking_response_with_thinking_and_text_blocks() {
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-clear-thinking.1.chunks.txt"
            ))]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;

            assert!(find_part(&result, "reasoning-start").is_some());
            let reasoning_deltas = filter_parts(&result, "reasoning-delta");
            assert!(!reasoning_deltas.is_empty());
            let full_reasoning = join_deltas(&result, "reasoning-delta", None);
            assert!(full_reasoning.contains("925"));
            assert!(full_reasoning.contains("185"));

            let signature_parts: Vec<_> = reasoning_deltas
                .iter()
                .filter(|part| !part["providerMetadata"][CANONICAL].is_null())
                .collect();
            assert_eq!(signature_parts.len(), 1);
            assert!(
                signature_parts[0]["providerMetadata"][CANONICAL]["signature"]
                    .as_str()
                    .is_some_and(|signature| !signature.is_empty())
            );

            let text_deltas: Vec<_> = result
                .iter()
                .filter(|part| part["type"] == "text-delta" && part["id"] == "1")
                .collect();
            assert!(!text_deltas.is_empty());
            assert_eq!(
                join_deltas(&result, "text-delta", Some("1")),
                "925 ÷ 5 = 185"
            );

            let finish_part = find_part(&result, "finish").expect("finish part");
            assert_equals(
                &finish_part["providerMetadata"][CANONICAL]["contextManagement"],
                &json!({
                    "appliedEdits": [],
                }),
            );
            assert_match_object(
                &finish_part["usage"],
                &json!({
                    "inputTokens": {
                        "total": 69,
                    },
                    "outputTokens": {
                        "total": 53,
                    },
                }),
            );
            assert_equals(
                &finish_part["usage"],
                &json!({
                    "inputTokens": {
                        "cacheRead": 0,
                        "cacheWrite": 0,
                        "noCache": 69,
                        "total": 69,
                    },
                    "outputTokens": {
                        "total": 53,
                    },
                    "raw": {
                        "cache_creation": {
                            "ephemeral_1h_input_tokens": 0,
                            "ephemeral_5m_input_tokens": 0,
                        },
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "inference_geo": "not_available",
                        "input_tokens": 69,
                        "output_tokens": 53,
                        "service_tier": "standard",
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream combined clear_thinking and clear_tool_uses response
        #[tokio::test]
        async fn should_stream_combined_clear_thinking_and_clear_tool_uses_response() {
            let mock = server(vec![sse_fixture(include_str!(
                "fixtures/anthropic-combined-context-editing.1.chunks.txt"
            ))]);
            let result = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;

            let reasoning_deltas: Vec<_> = result
                .iter()
                .filter(|part| {
                    part["type"] == "reasoning-delta"
                        && part["providerMetadata"][CANONICAL].is_null()
                })
                .collect();
            assert!(!reasoning_deltas.is_empty());
            let full_reasoning: String = reasoning_deltas
                .iter()
                .filter_map(|part| part["delta"].as_str())
                .collect();
            assert!(full_reasoning.contains("25 * 37"));
            assert!(full_reasoning.contains("925"));

            let signature_parts: Vec<_> = result
                .iter()
                .filter(|part| {
                    part["type"] == "reasoning-delta"
                        && !part["providerMetadata"][CANONICAL].is_null()
                })
                .collect();
            assert_eq!(signature_parts.len(), 1);

            let text_deltas: Vec<_> = result
                .iter()
                .filter(|part| part["type"] == "text-delta" && part["id"] == "1")
                .collect();
            assert!(!text_deltas.is_empty());
            assert!(join_deltas(&result, "text-delta", Some("1")).contains("925"));

            let finish_part = find_part(&result, "finish").expect("finish part");
            assert_equals(
                &finish_part["providerMetadata"][CANONICAL]["contextManagement"],
                &json!({
                    "appliedEdits": [],
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should stream tool deltas
        #[tokio::test]
        async fn should_stream_tool_deltas() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01GouTqNCGXzrj5LQ5jEkw67","type":"message","role":"assistant","model":"claude-3-haiku-20240307","stop_sequence":null,"usage":{"input_tokens":441,"output_tokens":2},"content":[],"stop_reason":null}            }"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}      }"#,
                r#"{"type": "ping"}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Okay"}    }"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}   }"#,
                r#"{"type":"content_block_stop","index":0    }"#,
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01DBsB4vvYLnBDzZ5rBSxSLs","name":"test-tool","input":{}}      }"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}           }"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"value"}              }"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\":"}      }"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"Spark"}          }"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"le"}          }"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" Day\"}"}               }"#,
                r#"{"type":"content_block_stop","index":1              }"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":65}           }"#,
                r#"{"type":"message_stop"           }"#,
            ])]);
            let parts = stream_parts(
                &test_model(&mock),
                json!({
                    "tools": [
                        {
                            "type": "function",
                            "name": "test-tool",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "value": {
                                        "type": "string",
                                    },
                                },
                                "required": [
                                    "value",
                                ],
                                "additionalProperties": false,
                                "$schema": "http://json-schema.org/draft-07/schema#",
                            },
                        },
                    ],
                    "prompt": test_prompt(),
                }),
            )
            .await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01GouTqNCGXzrj5LQ5jEkw67",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "type": "text-start",
                    },
                    {
                        "delta": "Okay",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "delta": "!",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "id": "0",
                        "type": "text-end",
                    },
                    {
                        "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                        "toolName": "test-tool",
                        "type": "tool-input-start",
                    },
                    {
                        "delta": r#"{"value"#,
                        "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                        "type": "tool-input-delta",
                    },
                    {
                        "delta": r#"":"#,
                        "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                        "type": "tool-input-delta",
                    },
                    {
                        "delta": r#""Spark"#,
                        "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                        "type": "tool-input-delta",
                    },
                    {
                        "delta": "le",
                        "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                        "type": "tool-input-delta",
                    },
                    {
                        "delta": r#" Day"}"#,
                        "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                        "type": "tool-input-delta",
                    },
                    {
                        "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                        "type": "tool-input-end",
                    },
                    {
                        "input": r#"{"value":"Sparkle Day"}"#,
                        "toolCallId": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                        "toolName": "test-tool",
                        "type": "tool-call",
                    },
                    {
                        "finishReason": {
                            "raw": "tool_use",
                            "unified": "tool-calls",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "input_tokens": 441,
                                    "output_tokens": 65,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "cacheWrite": 0,
                                "noCache": 441,
                                "total": 441,
                            },
                            "outputTokens": {
                                "total": 65,
                            },
                            "raw": {
                                "input_tokens": 441,
                                "output_tokens": 65,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should forward error chunks
        #[tokio::test]
        async fn should_forward_error_chunks() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}      }"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}          }"#,
                r#"{"type": "ping"}"#,
                r#"{"type":"error","error":{"type":"error","message":"test error"}}"#,
            ])]);
            let parts = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "type": "text-start",
                    },
                    {
                        "error": {
                            "data": {
                                "message": "test error",
                                "type": "error",
                            },
                            "message": "test error",
                            "type": "error",
                        },
                        "type": "error",
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should expose the raw response headers
        #[tokio::test]
        async fn should_expose_the_raw_response_headers() {
            let mut response = sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, World!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ]);
            response
                .headers
                .push(("test-header".into(), "test-value".into()));
            let mock = server(vec![response]);
            let result = test_model(&mock)
                .do_stream(&call_options(json!({ "prompt": test_prompt() })))
                .await
                .expect("doStream");
            assert_eq!(
                serde_json::to_value(result.response.and_then(|response| response.headers))
                    .unwrap(),
                json!({
                    // default headers:
                    "content-type": "text/event-stream",
                    "cache-control": "no-cache",
                    "connection": "keep-alive",

                    // custom header
                    "test-header": "test-value",
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should pass the messages and the model
        #[tokio::test]
        async fn should_pass_the_messages_and_the_model() {
            let mut response = sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, World!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ]);
            response
                .headers
                .push(("test-header".into(), "test-value".into()));
            let mock = server(vec![response]);
            stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_eq!(
                request_body(&mock, 0),
                json!({
                    "stream": true,
                    "model": "claude-3-haiku-20240307",
                    "max_tokens": 4096,
                    "messages": [
                        {
                            "role": "user",
                            "content": [
                                {
                                    "type": "text",
                                    "text": "Hello",
                                },
                            ],
                        },
                    ],
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let mut response = sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, World!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ]);
            response
                .headers
                .push(("test-header".into(), "test-value".into()));
            let mock = server(vec![response]);
            let provider = create_anthropic(AnthropicProviderSettings {
                api_key: Some("test-api-key".into()),
                headers: Some(
                    [(
                        "Custom-Provider-Header".to_string(),
                        Some("provider-header-value".to_string()),
                    )]
                    .into_iter()
                    .collect(),
                ),
                fetch: Some(mock.transport()),
                ..Default::default()
            })
            .unwrap();
            stream_parts(
                &provider.messages("claude-3-haiku-20240307"),
                json!({
                    "prompt": test_prompt(),
                    "headers": { "Custom-Request-Header": "request-header-value" },
                }),
            )
            .await;
            assert_equals(
                &request_headers(&mock, 0),
                &json!({
                    "anthropic-version": "2023-06-01",
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                    "x-api-key": "test-api-key",
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should merge custom anthropic-beta headers without legacy fine-grained-tool-streaming beta
        #[tokio::test]
        async fn should_merge_custom_anthropic_beta_headers_without_legacy_fine_grained_tool_streaming_beta()
         {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, World!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let provider = create_anthropic(AnthropicProviderSettings {
                api_key: Some("test-api-key".into()),
                headers: Some(
                    [(
                        "anthropic-beta".to_string(),
                        Some("CONFIG-beta1,config-beta2".to_string()),
                    )]
                    .into_iter()
                    .collect(),
                ),
                fetch: Some(mock.transport()),
                ..Default::default()
            })
            .unwrap();
            stream_parts(
                &provider.messages("claude-3-haiku-20240307"),
                json!({
                    "prompt": test_prompt(),
                    "headers": { "anthropic-beta": "REQUEST-beta1,request-beta2" },
                }),
            )
            .await;
            assert_eq!(
                request_header(&mock, 0, "anthropic-beta").as_deref(),
                Some("config-beta1,config-beta2,request-beta1,request-beta2")
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should default to per-tool eager_input_streaming on streaming requests and not send the legacy beta header
        #[tokio::test]
        async fn should_default_to_per_tool_eager_input_streaming_on_streaming_requests_and_not_send_the_legacy_beta_header()
         {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            stream_parts(
                &plain_provider(&mock).messages("claude-3-haiku-20240307"),
                json!({
                    "prompt": test_prompt(),
                    "tools": [
                        {
                            "type": "function",
                            "name": "get_weather",
                            "description": "Get weather",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                            },
                        },
                    ],
                }),
            )
            .await;
            let body = request_body(&mock, 0);
            assert_eq!(body["tools"][0]["eager_input_streaming"], json!(true));
            assert_eq!(request_header(&mock, 0, "anthropic-beta"), None);
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should not add eager_input_streaming when toolStreaming is explicitly false
        #[tokio::test]
        async fn should_not_add_eager_input_streaming_when_tool_streaming_is_explicitly_false() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            stream_parts(
                &plain_provider(&mock).messages("claude-3-haiku-20240307"),
                json!({
                    "prompt": test_prompt(),
                    "providerOptions": {
                        CANONICAL: {
                            "toolStreaming": false,
                        },
                    },
                    "tools": [
                        {
                            "type": "function",
                            "name": "get_weather",
                            "description": "Get weather",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                            },
                        },
                    ],
                }),
            )
            .await;
            let body = request_body(&mock, 0);
            assert!(
                body["tools"][0].get("eager_input_streaming").is_none(),
                "{body:#}"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should not default eager_input_streaming on non-streaming (generate) calls
        #[tokio::test]
        async fn should_not_default_eager_input_streaming_on_non_streaming_generate_calls() {
            let mock = server(vec![json_response(json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "ok",
                    },
                ],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 1,
                    "output_tokens": 1,
                },
            }))]);
            generate(
                &plain_provider(&mock).messages("claude-3-haiku-20240307"),
                json!({
                    "prompt": test_prompt(),
                    "tools": [
                        {
                            "type": "function",
                            "name": "get_weather",
                            "description": "Get weather",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                            },
                        },
                    ],
                }),
            )
            .await;
            let body = request_body(&mock, 0);
            assert!(
                body["tools"][0].get("eager_input_streaming").is_none(),
                "{body:#}"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should include providerOptions.anthropic.anthropicBeta in anthropic-beta header
        #[tokio::test]
        async fn should_include_provider_options_anthropic_anthropic_beta_in_anthropic_beta_header()
        {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":5}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            stream_parts(
                &plain_provider(&mock).messages("claude-3-haiku-20240307"),
                json!({
                    "prompt": test_prompt(),
                    "providerOptions": {
                        CANONICAL: {
                            "anthropicBeta": [
                                "my-beta-2025-01-01",
                                "another-beta-2025-06-01",
                            ],
                        },
                    },
                }),
            )
            .await;
            let beta = request_header(&mock, 0, "anthropic-beta").unwrap_or_default();
            assert!(beta.contains("my-beta-2025-01-01"), "{beta}");
            assert!(beta.contains("another-beta-2025-06-01"), "{beta}");
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should support cache control
        #[tokio::test]
        async fn should_support_cache_control() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}      }"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}          }"#,
                r#"{"type": "ping"}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}              }"#,
                r#"{"type":"content_block_stop","index":0             }"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}          }"#,
                r#"{"type":"message_stop"           }"#,
            ])]);
            let parts = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "type": "text-start",
                    },
                    {
                        "delta": "Hello",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "id": "0",
                        "type": "text-end",
                    },
                    {
                        "finishReason": {
                            "raw": "end_turn",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "cache_creation_input_tokens": 10,
                                    "cache_read_input_tokens": 5,
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 5,
                                "cacheWrite": 10,
                                "noCache": 17,
                                "total": 32,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "cache_creation_input_tokens": 10,
                                "cache_read_input_tokens": 5,
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should support cache control and return extra fields in provider metadata
        #[tokio::test]
        async fn should_support_cache_control_and_return_extra_fields_in_provider_metadata() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1,"cache_creation_input_tokens":10,"cache_read_input_tokens":5,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":10}}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}          }"#,
                r#"{"type": "ping"}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}              }"#,
                r#"{"type":"content_block_stop","index":0             }"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}          }"#,
                r#"{"type":"message_stop"           }"#,
            ])]);
            let parts = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "type": "text-start",
                    },
                    {
                        "delta": "Hello",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "id": "0",
                        "type": "text-end",
                    },
                    {
                        "finishReason": {
                            "raw": "end_turn",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "cache_creation": {
                                        "ephemeral_1h_input_tokens": 10,
                                        "ephemeral_5m_input_tokens": 0,
                                    },
                                    "cache_creation_input_tokens": 10,
                                    "cache_read_input_tokens": 5,
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 5,
                                "cacheWrite": 10,
                                "noCache": 17,
                                "total": 32,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "cache_creation": {
                                    "ephemeral_1h_input_tokens": 10,
                                    "ephemeral_5m_input_tokens": 0,
                                },
                                "cache_creation_input_tokens": 10,
                                "cache_read_input_tokens": 5,
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should support cache tokens in message_delta event
        #[tokio::test]
        async fn should_support_cache_tokens_in_message_delta_event() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type": "ping"}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let parts = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(parts),
                &json!([
                    {
                        "type": "stream-start",
                        "warnings": [],
                    },
                    {
                        "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                        "modelId": "claude-3-haiku-20240307",
                        "type": "response-metadata",
                    },
                    {
                        "id": "0",
                        "type": "text-start",
                    },
                    {
                        "delta": "Hello",
                        "id": "0",
                        "type": "text-delta",
                    },
                    {
                        "id": "0",
                        "type": "text-end",
                    },
                    {
                        "finishReason": {
                            "raw": "end_turn",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "cache_creation_input_tokens": 10,
                                    "cache_read_input_tokens": 5,
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 5,
                                "cacheWrite": 10,
                                "noCache": 17,
                                "total": 32,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "cache_creation_input_tokens": 10,
                                "cache_read_input_tokens": 5,
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should send request body
        #[tokio::test]
        async fn should_send_request_body() {
            let mut response = sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, World!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ]);
            response
                .headers
                .push(("test-header".into(), "test-value".into()));
            let mock = server(vec![response]);
            let result = test_model(&mock)
                .do_stream(&call_options(json!({ "prompt": test_prompt() })))
                .await
                .expect("doStream");
            assert_equals(
                &serde_json::to_value(&result.request).unwrap(),
                &json!({
                    "body": {
                        "max_tokens": 4096,
                        "messages": [
                            {
                                "content": [
                                    {
                                        "text": "Hello",
                                        "type": "text",
                                    },
                                ],
                                "role": "user",
                            },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stream": true,
                    },
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should handle handle stop_reason:pause_turn
        #[tokio::test]
        async fn should_handle_handle_stop_reason_pause_turn() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", "}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"World!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"pause_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let chunks = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            assert_equals(
                &json!(filter_parts(&chunks, "finish")),
                &json!([
                    {
                        "finishReason": {
                            "raw": "pause_turn",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": null,
                                "usage": {
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "cacheWrite": 0,
                                "noCache": 17,
                                "total": 17,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should include stop_sequence in provider metadata
        #[tokio::test]
        async fn should_include_stop_sequence_in_provider_metadata() {
            let mock = server(vec![sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"stop_sequence","stop_sequence":"STOP"},"usage":{"output_tokens":227}}"#,
                r#"{"type":"message_stop"}"#,
            ])]);
            let chunks = stream_parts(
                &test_model(&mock),
                json!({ "prompt": test_prompt(), "stopSequences": ["STOP"] }),
            )
            .await;
            assert_equals(
                &json!(filter_parts(&chunks, "finish")),
                &json!([
                    {
                        "finishReason": {
                            "raw": "stop_sequence",
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            CANONICAL: {
                                "container": null,
                                "contextManagement": null,
                                "iterations": null,
                                "stopSequence": "STOP",
                                "usage": {
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "cacheWrite": 0,
                                "noCache": 17,
                                "total": 17,
                            },
                            "outputTokens": {
                                "total": 227,
                            },
                            "raw": {
                                "input_tokens": 17,
                                "output_tokens": 227,
                            },
                        },
                    },
                ]),
            );
        }

        mod raw_chunks {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › raw chunks › should include raw chunks when includeRawChunks is enabled
            #[tokio::test]
            async fn should_include_raw_chunks_when_include_raw_chunks_is_enabled() {
                let mock = server(vec![sse_data(&[
                    r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                    r#"{"type":"content_block_stop","index":0}"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                    r#"{"type":"message_stop"}"#,
                ])]);
                let chunks = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "includeRawChunks": true,
                    }),
                )
                .await;
                assert_equals(
                    &json!(filter_parts(&chunks, "raw")),
                    &json!([
                        {
                            "rawValue": {
                                "message": {
                                    "content": [],
                                    "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                                    "model": "claude-3-haiku-20240307",
                                    "role": "assistant",
                                    "stop_reason": null,
                                    "stop_sequence": null,
                                    "type": "message",
                                    "usage": {
                                        "input_tokens": 17,
                                        "output_tokens": 1,
                                    },
                                },
                                "type": "message_start",
                            },
                            "type": "raw",
                        },
                        {
                            "rawValue": {
                                "content_block": {
                                    "text": "",
                                    "type": "text",
                                },
                                "index": 0,
                                "type": "content_block_start",
                            },
                            "type": "raw",
                        },
                        {
                            "rawValue": {
                                "delta": {
                                    "text": "Hello",
                                    "type": "text_delta",
                                },
                                "index": 0,
                                "type": "content_block_delta",
                            },
                            "type": "raw",
                        },
                        {
                            "rawValue": {
                                "index": 0,
                                "type": "content_block_stop",
                            },
                            "type": "raw",
                        },
                        {
                            "rawValue": {
                                "delta": {
                                    "stop_reason": "end_turn",
                                    "stop_sequence": null,
                                },
                                "type": "message_delta",
                                "usage": {
                                    "output_tokens": 227,
                                },
                            },
                            "type": "raw",
                        },
                        {
                            "rawValue": {
                                "type": "message_stop",
                            },
                            "type": "raw",
                        },
                    ]),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › raw chunks › should not include raw chunks when includeRawChunks is false
            #[tokio::test]
            async fn should_not_include_raw_chunks_when_include_raw_chunks_is_false() {
                let mock = server(vec![sse_data(&[
                    r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                    r#"{"type":"content_block_stop","index":0}"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                    r#"{"type":"message_stop"}"#,
                ])]);
                let chunks = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                    }),
                )
                .await;
                assert_eq!(filter_parts(&chunks, "raw").len(), 0);
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › raw chunks › should process PDF citation responses in streaming
            #[tokio::test]
            async fn should_process_pdf_citation_responses_in_streaming() {
                // Create a model with predictable ID generation for testing
                let mock = server(vec![sse_data(&[
                    r#"{"type":"message_start","message":{"id":"msg_01KfpJoAEabmH2iHRRFjQMAG","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":17,"output_tokens":1}}}"#,
                    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Based on the document"}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", results show growth."}}"#,
                    r#"{"type":"content_block_stop","index":0}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta","citation":{"type":"page_location","cited_text":"Revenue increased by 25% year over year","document_index":0,"document_title":"Financial Report 2023","start_page_number":5,"end_page_number":6}}}"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":227}}"#,
                    r#"{"type":"message_stop"}"#,
                ])]);
                let model_with_mock_id = test_provider(&mock).messages("claude-3-haiku-20240307");
                let result = stream_parts(
                    &model_with_mock_id,
                    json!({
                        "prompt": [
                            {
                                "role": "user",
                                "content": [
                                    {
                                        "type": "file",
                                        "data": {
                                            "type": "data",
                                            "data": "base64PDFdata",
                                        },
                                        "mediaType": "application/pdf",
                                        "filename": "financial-report.pdf",
                                        "providerOptions": {
                                            CANONICAL: {
                                                "citations": {
                                                    "enabled": true,
                                                },
                                            },
                                        },
                                    },
                                    {
                                        "type": "text",
                                        "text": "What do the results show?",
                                    },
                                ],
                            },
                        ],
                    }),
                )
                .await;
                assert_equals(
                    &json!(result),
                    &json!([
                        {
                            "type": "stream-start",
                            "warnings": [],
                        },
                        {
                            "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                            "modelId": "claude-3-haiku-20240307",
                            "type": "response-metadata",
                        },
                        {
                            "id": "0",
                            "type": "text-start",
                        },
                        {
                            "delta": "Based on the document",
                            "id": "0",
                            "type": "text-delta",
                        },
                        {
                            "delta": ", results show growth.",
                            "id": "0",
                            "type": "text-delta",
                        },
                        {
                            "id": "0",
                            "type": "text-end",
                        },
                        {
                            "filename": "financial-report.pdf",
                            "id": "id-0",
                            "mediaType": "application/pdf",
                            "providerMetadata": {
                                CANONICAL: {
                                    "citedText": "Revenue increased by 25% year over year",
                                    "endPageNumber": 6,
                                    "startPageNumber": 5,
                                },
                            },
                            "sourceType": "document",
                            "title": "Financial Report 2023",
                            "type": "source",
                        },
                        {
                            "finishReason": {
                                "raw": "end_turn",
                                "unified": "stop",
                            },
                            "providerMetadata": {
                                CANONICAL: {
                                    "container": null,
                                    "contextManagement": null,
                                    "iterations": null,
                                    "stopSequence": null,
                                    "usage": {
                                        "input_tokens": 17,
                                        "output_tokens": 227,
                                    },
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "cacheWrite": 0,
                                    "noCache": 17,
                                    "total": 17,
                                },
                                "outputTokens": {
                                    "total": 227,
                                },
                                "raw": {
                                    "input_tokens": 17,
                                    "output_tokens": 227,
                                },
                            },
                        },
                    ]),
                );
            }
        }

        mod code_execution_file_uploads {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › code execution file uploads › should stream container upload code execution results
            #[tokio::test]
            async fn should_stream_container_upload_code_execution_results() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-code-execution-file-upload.1.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": [
                            {
                                "role": "user",
                                "content": [
                                    {
                                        "type": "text",
                                        "text": "Analyze this CSV data.",
                                    },
                                    {
                                        "type": "file",
                                        "mediaType": "text/csv",
                                        "data": {
                                            "type": "reference",
                                            "reference": {
                                                CANONICAL: "file-csv-12345",
                                            },
                                        },
                                        "providerOptions": {
                                            CANONICAL: {
                                                "containerUpload": true,
                                            },
                                        },
                                    },
                                ],
                            },
                        ],
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.code_execution_20250825",
                                "name": "code_execution",
                                "args": {},
                            },
                        ],
                    }),
                )
                .await;
                assert_equals(
                    &request_body(&mock, 0),
                    &json!({
                        "max_tokens": 4096,
                        "messages": [
                            {
                                "content": [
                                    {
                                        "text": "Analyze this CSV data.",
                                        "type": "text",
                                    },
                                    {
                                        "file_id": "file-csv-12345",
                                        "type": "container_upload",
                                    },
                                ],
                                "role": "user",
                            },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stream": true,
                        "tools": [
                            {
                                "name": "code_execution",
                                "type": "code_execution_20250825",
                            },
                        ],
                    }),
                );
                assert_equals(
                    &request_headers(&mock, 0),
                    &json!({
                        "anthropic-beta": "files-api-2025-04-14,code-execution-2025-08-25",
                        "anthropic-version": "2023-06-01",
                        "content-type": "application/json",
                        "x-api-key": "test-api-key",
                    }),
                );
                assert_equals(
                    &json!(parts),
                    &json!({
                        "$arrayContaining": [
                            {
                                "$objectContaining": {
                                    "type": "tool-call",
                                    "toolCallId": "srvtoolu_01UAM7DM8XEfNwyddFNKpVp2",
                                    "toolName": "code_execution",
                                    "input": {
                                        "$stringContaining": "$INPUT_DIR/sample.csv",
                                    },
                                    "providerExecuted": true,
                                },
                            },
                            {
                                "$objectContaining": {
                                    "type": "tool-result",
                                    "toolCallId": "srvtoolu_01UAM7DM8XEfNwyddFNKpVp2",
                                    "toolName": "code_execution",
                                    "result": {
                                        "$objectContaining": {
                                            "type": "text_editor_code_execution_view_result",
                                            "content": {
                                                "$stringContaining": "month,revenue,expenses,profit",
                                            },
                                        },
                                    },
                                },
                            },
                            {
                                "$objectContaining": {
                                    "type": "tool-call",
                                    "toolCallId": "srvtoolu_01P2RuXQdkVngtqpdr2dQhv2",
                                    "toolName": "code_execution",
                                    "input": {
                                        "$stringContaining": "python /tmp/analyze_data.py",
                                    },
                                    "providerExecuted": true,
                                },
                            },
                            {
                                "$objectContaining": {
                                    "type": "tool-result",
                                    "toolCallId": "srvtoolu_01P2RuXQdkVngtqpdr2dQhv2",
                                    "toolName": "code_execution",
                                    "result": {
                                        "$objectContaining": {
                                            "type": "bash_code_execution_result",
                                            "stdout": {
                                                "$stringContaining": "Total Profit: $35,400.00",
                                            },
                                        },
                                    },
                                },
                            },
                            {
                                "$objectContaining": {
                                    "type": "finish",
                                    "providerMetadata": {
                                        CANONICAL: {
                                            "$objectContaining": {
                                                "container": {
                                                    "$objectContaining": {
                                                        "id": "container_011CbJQ7DqpL337rdwQ76jnu",
                                                    },
                                                },
                                            },
                                        },
                                    },
                                },
                            },
                        ],
                    }),
                );
            }
        }

        mod mcp_servers {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › mcp servers › should stream code execution tool results
            #[tokio::test]
            async fn should_stream_code_execution_tool_results() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-mcp.1.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "providerOptions": {
                            CANONICAL: {
                                "mcpServers": [
                                    {
                                        "type": "url",
                                        "name": "echo",
                                        "url": "https://echo.mcp.inevitable.fyi/mcp",
                                    },
                                ],
                            },
                        },
                    }),
                )
                .await;
                assert_equals(
                    &json!(parts),
                    &snapshot(include_str!("snapshots/stream_mcp_servers.json")),
                );
            }
        }

        mod agent_skills {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › agent skills › should stream code execution tool results
            #[tokio::test]
            async fn should_stream_code_execution_tool_results() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-code-execution-20250825.pptx-skill.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.code_execution_20250825",
                                "name": "code_execution",
                                "args": {},
                            },
                        ],
                        "providerOptions": {
                            CANONICAL: {
                                "container": {
                                    "skills": [
                                        {
                                            "type": CANONICAL,
                                            "skillId": "pptx",
                                        },
                                    ],
                                },
                            },
                        },
                    }),
                )
                .await;
                assert_equals(
                    &json!(parts),
                    &snapshot(include_str!(
                        "snapshots/stream_agent_skills_code_execution.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › agent skills › should preserve the text editor discriminator for streamed skill tool calls
            #[tokio::test]
            async fn should_preserve_the_text_editor_discriminator_for_streamed_skill_tool_calls() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-code-execution-20250825.pptx-skill.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.code_execution_20250825",
                                "name": "code_execution",
                                "args": {},
                            },
                        ],
                        "providerOptions": {
                            CANONICAL: {
                                "container": {
                                    "skills": [
                                        {
                                            "type": CANONICAL,
                                            "skillId": "pptx",
                                        },
                                    ],
                                },
                            },
                        },
                    }),
                )
                .await;
                let skill_read_tool_call = parts
                    .iter()
                    .find(|part| {
                        part["type"] == "tool-call"
                            && part["toolName"] == "code_execution"
                            && part["input"]
                                .as_str()
                                .is_some_and(|input| input.contains("/skills/pptx/SKILL.md"))
                    })
                    .expect("skill read tool call");
                assert_match_object(
                    skill_read_tool_call,
                    &json!({
                        "type": "tool-call",
                        "toolName": "code_execution",
                        "input": r#"{"type": "text_editor_code_execution","command": "view", "path": "/skills/pptx/SKILL.md"}"#,
                        "providerExecuted": true,
                    }),
                );
            }
        }

        mod function_tool {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › function tool › should stream tool deltas
            #[tokio::test]
            async fn should_stream_tool_deltas() {
                let mock = server(vec![sse_data(&[
                    r#"{"type":"message_start","message":{"id":"msg_01GouTqNCGXzrj5LQ5jEkw67","type":"message","role":"assistant","model":"claude-3-haiku-20240307","stop_sequence":null,"usage":{"input_tokens":441,"output_tokens":2},"content":[],"stop_reason":null}            }"#,
                    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}      }"#,
                    r#"{"type": "ping"}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Okay"}    }"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}   }"#,
                    r#"{"type":"content_block_stop","index":0    }"#,
                    r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01DBsB4vvYLnBDzZ5rBSxSLs","name":"test-tool","input":{}}      }"#,
                    r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}           }"#,
                    r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"value"}              }"#,
                    r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\":"}      }"#,
                    r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"Spark"}          }"#,
                    r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"le"}          }"#,
                    r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" Day\"}"}               }"#,
                    r#"{"type":"content_block_stop","index":1              }"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":65}           }"#,
                    r#"{"type":"message_stop"           }"#,
                ])]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "tools": [
                            {
                                "type": "function",
                                "name": "test-tool",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "value": {
                                            "type": "string",
                                        },
                                    },
                                    "required": [
                                        "value",
                                    ],
                                    "additionalProperties": false,
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                },
                            },
                        ],
                        "prompt": test_prompt(),
                    }),
                )
                .await;
                assert_equals(
                    &json!(parts),
                    &json!([
                        {
                            "type": "stream-start",
                            "warnings": [],
                        },
                        {
                            "id": "msg_01GouTqNCGXzrj5LQ5jEkw67",
                            "modelId": "claude-3-haiku-20240307",
                            "type": "response-metadata",
                        },
                        {
                            "id": "0",
                            "type": "text-start",
                        },
                        {
                            "delta": "Okay",
                            "id": "0",
                            "type": "text-delta",
                        },
                        {
                            "delta": "!",
                            "id": "0",
                            "type": "text-delta",
                        },
                        {
                            "id": "0",
                            "type": "text-end",
                        },
                        {
                            "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                            "toolName": "test-tool",
                            "type": "tool-input-start",
                        },
                        {
                            "delta": r#"{"value"#,
                            "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                            "type": "tool-input-delta",
                        },
                        {
                            "delta": r#"":"#,
                            "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                            "type": "tool-input-delta",
                        },
                        {
                            "delta": r#""Spark"#,
                            "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                            "type": "tool-input-delta",
                        },
                        {
                            "delta": "le",
                            "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                            "type": "tool-input-delta",
                        },
                        {
                            "delta": r#" Day"}"#,
                            "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                            "type": "tool-input-delta",
                        },
                        {
                            "id": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                            "type": "tool-input-end",
                        },
                        {
                            "input": r#"{"value":"Sparkle Day"}"#,
                            "toolCallId": "toolu_01DBsB4vvYLnBDzZ5rBSxSLs",
                            "toolName": "test-tool",
                            "type": "tool-call",
                        },
                        {
                            "finishReason": {
                                "raw": "tool_use",
                                "unified": "tool-calls",
                            },
                            "providerMetadata": {
                                CANONICAL: {
                                    "container": null,
                                    "contextManagement": null,
                                    "iterations": null,
                                    "stopSequence": null,
                                    "usage": {
                                        "input_tokens": 441,
                                        "output_tokens": 65,
                                    },
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "cacheWrite": 0,
                                    "noCache": 441,
                                    "total": 441,
                                },
                                "outputTokens": {
                                    "total": 65,
                                },
                                "raw": {
                                    "input_tokens": 441,
                                    "output_tokens": 65,
                                },
                            },
                        },
                    ]),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › function tool › should support tools with empty parameters in streaming
            #[tokio::test]
            async fn should_support_tools_with_empty_parameters_in_streaming() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-tool-no-args.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "tools": [
                            {
                                "type": "function",
                                "name": "test-tool",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {},
                                    "required": [],
                                    "additionalProperties": false,
                                    "$schema": "http://json-schema.org/draft-07/schema#",
                                },
                            },
                        ],
                        "prompt": test_prompt(),
                    }),
                )
                .await;
                assert_equals(
                    &json!(parts),
                    &json!([
                        {
                            "type": "stream-start",
                            "warnings": [],
                        },
                        {
                            "id": "msg_01GE2RKp1VYsPzdFs3sS9z5S",
                            "modelId": "claude-sonnet-4-5-20250929",
                            "type": "response-metadata",
                        },
                        {
                            "id": "0",
                            "type": "text-start",
                        },
                        {
                            "delta": "I'll update the issue list for",
                            "id": "0",
                            "type": "text-delta",
                        },
                        {
                            "delta": " you.",
                            "id": "0",
                            "type": "text-delta",
                        },
                        {
                            "id": "0",
                            "type": "text-end",
                        },
                        {
                            "id": "toolu_01QE1WLsSVp5hy5Q3GmGTmjP",
                            "toolName": "updateIssueList",
                            "type": "tool-input-start",
                        },
                        {
                            "id": "toolu_01QE1WLsSVp5hy5Q3GmGTmjP",
                            "type": "tool-input-end",
                        },
                        {
                            "input": "{}",
                            "toolCallId": "toolu_01QE1WLsSVp5hy5Q3GmGTmjP",
                            "toolName": "updateIssueList",
                            "type": "tool-call",
                        },
                        {
                            "finishReason": {
                                "raw": "tool_use",
                                "unified": "tool-calls",
                            },
                            "providerMetadata": {
                                CANONICAL: {
                                    "container": null,
                                    "contextManagement": null,
                                    "iterations": null,
                                    "stopSequence": null,
                                    "usage": {
                                        "cache_creation": {
                                            "ephemeral_1h_input_tokens": 0,
                                            "ephemeral_5m_input_tokens": 0,
                                        },
                                        "cache_creation_input_tokens": 0,
                                        "cache_read_input_tokens": 0,
                                        "input_tokens": 565,
                                        "output_tokens": 48,
                                        "service_tier": "standard",
                                    },
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "cacheWrite": 0,
                                    "noCache": 565,
                                    "total": 565,
                                },
                                "outputTokens": {
                                    "total": 48,
                                },
                                "raw": {
                                    "cache_creation": {
                                        "ephemeral_1h_input_tokens": 0,
                                        "ephemeral_5m_input_tokens": 0,
                                    },
                                    "cache_creation_input_tokens": 0,
                                    "cache_read_input_tokens": 0,
                                    "input_tokens": 565,
                                    "output_tokens": 48,
                                    "service_tier": "standard",
                                },
                            },
                        },
                    ]),
                );
            }
        }

        mod programmatic_tool_calling {
            use super::*;

            fn tool_call(parts: &[Value]) -> &Value {
                find_part(parts, "tool-call").expect("a tool-call part")
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › programmatic tool calling › should include caller info when tool_use has caller field from code_execution
            #[tokio::test]
            async fn should_include_caller_info_when_tool_use_has_caller_field_from_code_execution()
            {
                let mock = server(vec![sse_data(&[
                    r#"{"type":"message_start","message":{"id":"msg_01Test","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}"#,
                    r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01Test","name":"query_database","input":{},"caller":{"type":"code_execution_20250825","tool_id":"srvtoolu_01CodeExec"}}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\""}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":": \"SELECT * FROM users\"}"}}"#,
                    r#"{"type":"content_block_stop","index":0}"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":50}}"#,
                    "[DONE]",
                ])]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "tools": [
                            {
                                "type": "function",
                                "name": "query_database",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "sql": {
                                            "type": "string",
                                        },
                                    },
                                },
                            },
                        ],
                        "prompt": test_prompt(),
                    }),
                )
                .await;
                let tool_call = tool_call(&parts);
                assert_eq!(tool_call["input"], r#"{"sql": "SELECT * FROM users"}"#);
                assert_equals(
                    &tool_call["providerMetadata"],
                    &json!({
                        CANONICAL: {
                            "caller": {
                                "type": "code_execution_20250825",
                                "toolId": "srvtoolu_01CodeExec",
                            },
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › programmatic tool calling › should include caller info when tool_use has direct caller type
            #[tokio::test]
            async fn should_include_caller_info_when_tool_use_has_direct_caller_type() {
                let mock = server(vec![sse_data(&[
                    r#"{"type":"message_start","message":{"id":"msg_01Test","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}"#,
                    r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01Test","name":"get_weather","input":{},"caller":{"type":"direct"}}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\""}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":": \"Tokyo\"}"}}"#,
                    r#"{"type":"content_block_stop","index":0}"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":50}}"#,
                    "[DONE]",
                ])]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "tools": [
                            {
                                "type": "function",
                                "name": "get_weather",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "city": {
                                            "type": "string",
                                        },
                                    },
                                },
                            },
                        ],
                        "prompt": test_prompt(),
                    }),
                )
                .await;
                let tool_call = tool_call(&parts);
                assert_equals(
                    &tool_call["providerMetadata"],
                    &json!({
                        CANONICAL: {
                            "caller": {
                                "type": "direct",
                            },
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › programmatic tool calling › should use non-empty input from content_block_start for deferred tool calls
            #[tokio::test]
            async fn should_use_non_empty_input_from_content_block_start_for_deferred_tool_calls() {
                let mock = server(vec![sse_data(&[
                    r#"{"type":"message_start","message":{"id":"msg_01Test","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}"#,
                    r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01Deferred","name":"query_database","input":{"sql":"SELECT COUNT(*) FROM orders"},"caller":{"type":"code_execution_20250825","tool_id":"srvtoolu_01CodeExec"}}}"#,
                    r#"{"type":"content_block_stop","index":0}"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":30}}"#,
                    "[DONE]",
                ])]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "tools": [
                            {
                                "type": "function",
                                "name": "query_database",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "sql": {
                                            "type": "string",
                                        },
                                    },
                                },
                            },
                        ],
                        "prompt": test_prompt(),
                    }),
                )
                .await;
                let tool_call = tool_call(&parts);
                assert_eq!(tool_call["toolCallId"], "toolu_01Deferred");
                assert_eq!(
                    tool_call["input"],
                    r#"{"sql":"SELECT COUNT(*) FROM orders"}"#
                );
                assert_equals(
                    &tool_call["providerMetadata"],
                    &json!({
                        CANONICAL: {
                            "caller": {
                                "type": "code_execution_20250825",
                                "toolId": "srvtoolu_01CodeExec",
                            },
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › programmatic tool calling › should NOT prepend empty {} input when deltas follow content_block_start
            #[tokio::test]
            async fn should_not_prepend_empty_input_when_deltas_follow_content_block_start() {
                let mock = server(vec![sse_data(&[
                    r#"{"type":"message_start","message":{"id":"msg_01Test","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":1}}}"#,
                    r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01Test","name":"get_weather","input":{}}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\""}}"#,
                    r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":": \"London\"}"}}"#,
                    r#"{"type":"content_block_stop","index":0}"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":30}}"#,
                    "[DONE]",
                ])]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "tools": [
                            {
                                "type": "function",
                                "name": "get_weather",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "city": {
                                            "type": "string",
                                        },
                                    },
                                },
                            },
                        ],
                        "prompt": test_prompt(),
                    }),
                )
                .await;
                assert_eq!(tool_call(&parts)["input"], r#"{"city": "London"}"#);
            }

            mod with_fixture_multi_turn_dice_game {
                use super::*;

                async fn parts() -> Vec<Value> {
                    let mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-programmatic-tool-calling.1.chunks.txt"
                    ))]);
                    stream_parts(
                        &test_model(&mock),
                        json!({
                            "prompt": test_prompt(),
                            "tools": [
                                {
                                    "type": "provider",
                                    "id": "anthropic.code_execution_20250825",
                                    "name": "code_execution",
                                    "args": {},
                                },
                                {
                                    "type": "function",
                                    "name": "rollDie",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "player": {
                                                "type": "string",
                                            },
                                        },
                                    },
                                },
                            ],
                        }),
                    )
                    .await
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › programmatic tool calling › with fixture (multi-turn dice game) › should stream programmatic tool calling with multiple message_start/stop sequences
                #[tokio::test]
                async fn should_stream_programmatic_tool_calling_with_multiple_message_start_stop_sequences()
                 {
                    assert_equals(
                        &json!(parts().await),
                        &snapshot(include_str!(
                            "snapshots/stream_programmatic_tool_calling.json"
                        )),
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › programmatic tool calling › with fixture (multi-turn dice game) › should extract caller metadata from streamed tool calls
                #[tokio::test]
                async fn should_extract_caller_metadata_from_streamed_tool_calls() {
                    let parts = parts().await;
                    let tool_calls = filter_parts(&parts, "tool-call");

                    // Filter rollDie calls (not code_execution)
                    let roll_die_calls: Vec<_> = tool_calls
                        .iter()
                        .filter(|call| call["toolName"] == "rollDie")
                        .collect();
                    assert!(!roll_die_calls.is_empty());

                    // Each rollDie call should have caller metadata
                    for call in roll_die_calls {
                        assert_equals(
                            &call["providerMetadata"][CANONICAL]["caller"],
                            &json!({
                                "type": "code_execution_20250825",
                                "toolId": {
                                    "$stringMatching": "^srvtoolu_",
                                },
                            }),
                        );
                    }
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › programmatic tool calling › with fixture (multi-turn dice game) › should include code_execution_tool_result in stream
                #[tokio::test]
                async fn should_include_code_execution_tool_result_in_stream() {
                    let parts = parts().await;
                    let tool_results = filter_parts(&parts, "tool-result");

                    // Should have code_execution result
                    let code_exec_result = tool_results
                        .iter()
                        .find(|result| result["toolName"] == "code_execution")
                        .expect("code_execution result");
                    assert_match_object(
                        code_exec_result,
                        &json!({
                            "type": "tool-result",
                            "toolName": "code_execution",
                        }),
                    );
                }
            }
        }

        mod code_execution_20250825_tool {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › code execution 20250825 tool › should stream code execution tool results
            #[tokio::test]
            async fn should_stream_code_execution_tool_results() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-code-execution-20250825.1.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.code_execution_20250825",
                                "name": "code_execution",
                                "args": {},
                            },
                        ],
                    }),
                )
                .await;
                assert_equals(
                    &json!(parts),
                    &snapshot(include_str!(
                        "snapshots/stream_code_execution_20250825.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › code execution 20250825 tool › should include file id list in code execution tool call result.
            #[tokio::test]
            async fn should_include_file_id_list_in_code_execution_tool_call_result() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-code-execution-20250825.2.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.code_execution_20250825",
                                "name": "code_execution",
                                "args": {},
                            },
                        ],
                    }),
                )
                .await;
                assert_equals(
                    &without_parser_message(json!(parts)),
                    &without_parser_message(snapshot(include_str!(
                        "snapshots/stream_code_execution_20250825_file_id_list.json"
                    ))),
                );
            }
        }

        mod advisor_tool {
            use super::*;

            /// The `beforeEach`.
            async fn parts() -> Vec<Value> {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-advisor-20250301.1.chunks.txt"
                ))]);
                stream_parts(
                    &test_provider(&mock).messages("claude-sonnet-4-6"),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.advisor_20260301",
                                "name": "advisor",
                                "args": {
                                    "model": "claude-opus-4-7",
                                },
                            },
                        ],
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › advisor tool › should emit the advisor server_tool_use as a provider-executed tool call
            #[tokio::test]
            async fn should_emit_the_advisor_server_tool_use_as_a_provider_executed_tool_call() {
                let parts = parts().await;
                let advisor_tool_call = parts
                    .iter()
                    .find(|part| part["type"] == "tool-call" && part["toolName"] == "advisor")
                    .expect("advisor tool call");
                assert_equals(
                    advisor_tool_call,
                    &json!({
                        "input": "{}",
                        "providerExecuted": true,
                        "toolCallId": "srvtoolu_01R6zRtm9VnRaSJUVkGK9zvM",
                        "toolName": "advisor",
                        "type": "tool-call",
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › advisor tool › should emit the fully formed advisor result before executor text resumes
            #[tokio::test]
            async fn should_emit_the_fully_formed_advisor_result_before_executor_text_resumes() {
                let parts = parts().await;
                let advisor_tool_result_index = parts
                    .iter()
                    .position(|part| part["type"] == "tool-result" && part["toolName"] == "advisor")
                    .expect("advisor tool result");
                let next_text_start_index = parts.iter().enumerate().position(|(index, part)| {
                    index > advisor_tool_result_index && part["type"] == "text-start"
                });
                assert!(
                    next_text_start_index.is_some_and(|index| index > advisor_tool_result_index)
                );
                assert_match_object(
                    &parts[advisor_tool_result_index],
                    &json!({
                        "type": "tool-result",
                        "toolCallId": "srvtoolu_01R6zRtm9VnRaSJUVkGK9zvM",
                        "toolName": "advisor",
                        "result": {
                            "type": "advisor_result",
                            "text": {
                                "$stringContaining": "Concurrent Worker Pool in Go",
                            },
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › advisor tool › should expose advisor usage iterations on the finish part
            #[tokio::test]
            async fn should_expose_advisor_usage_iterations_on_the_finish_part() {
                let parts = parts().await;
                let finish_part = find_part(&parts, "finish").expect("finish part");
                assert_equals(
                    &finish_part["providerMetadata"][CANONICAL]["iterations"],
                    &json!([
                        {
                            "inputTokens": 1051,
                            "outputTokens": 35,
                            "type": "message",
                        },
                        {
                            "inputTokens": 2728,
                            "model": "claude-opus-4-7",
                            "outputTokens": 3880,
                            "type": "advisor_message",
                        },
                        {
                            "inputTokens": 3676,
                            "outputTokens": 3356,
                            "type": "message",
                        },
                    ]),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › advisor tool › should stream stopReason for plaintext and redacted advisor results
            #[tokio::test]
            async fn should_stream_stop_reason_for_plaintext_and_redacted_advisor_results() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-advisor-stop-reasons.chunks.txt"
                ))]);
                let result_parts = stream_parts(
                    &test_provider(&mock).messages("claude-sonnet-4-6"),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.advisor_20260301",
                                "name": "advisor",
                                "args": {
                                    "model": "claude-opus-4-7",
                                    "maxTokens": 2048,
                                },
                            },
                        ],
                    }),
                )
                .await;
                assert_equals(
                    &json!(filter_parts(&result_parts, "tool-result")),
                    &json!([
                        {
                            "result": {
                                "stopReason": "max_tokens",
                                "text": "Partial plaintext advice.",
                                "type": "advisor_result",
                            },
                            "toolCallId": "srvtoolu_advisor_plaintext",
                            "toolName": "advisor",
                            "type": "tool-result",
                        },
                        {
                            "result": {
                                "encryptedContent": "opaque-encrypted-advice",
                                "stopReason": "end_turn",
                                "type": "advisor_redacted_result",
                            },
                            "toolCallId": "srvtoolu_advisor_redacted",
                            "toolName": "advisor",
                            "type": "tool-result",
                        },
                    ]),
                );
            }
        }

        mod code_execution_20260120_tool {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › code execution 20260120 tool › should stream code execution tool results without beta header
            #[tokio::test]
            async fn should_stream_code_execution_tool_results_without_beta_header() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-code-execution-20250825.1.chunks.txt"
                ))]);
                let parts = stream_parts(
                    &test_model(&mock),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.code_execution_20260120",
                                "name": "code_execution",
                                "args": {},
                            },
                        ],
                    }),
                )
                .await;
                assert_equals(
                    &json!(parts),
                    &snapshot(include_str!(
                        "snapshots/stream_code_execution_20260120.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › code execution 20260120 tool › should preserve the complete code execution transcript when replaying streamed output
            #[tokio::test]
            async fn should_preserve_the_complete_code_execution_transcript_when_replaying_streamed_output()
             {
                let mock = server(vec![
                    sse_fixture(include_str!(
                        "fixtures/anthropic-code-execution-20260120-prompt-cache.1.chunks.txt"
                    )),
                    sse_fixture(include_str!("fixtures/anthropic-refusal.chunks.txt")),
                ]);
                let model = test_provider(&mock).messages("claude-sonnet-5");
                let tools = json!([
                    {
                        "type": "provider",
                        "id": "anthropic.code_execution_20260120",
                        "name": "code_execution",
                        "args": {},
                    },
                ]);
                let parts =
                    stream_parts(&model, json!({ "prompt": test_prompt(), "tools": tools })).await;
                let tool_calls: Vec<_> = parts
                    .iter()
                    .filter(|part| {
                        part["type"] == "tool-call" && part["toolName"] == "code_execution"
                    })
                    .collect();
                let tool_results: Vec<_> = parts
                    .iter()
                    .filter(|part| {
                        part["type"] == "tool-result" && part["toolName"] == "code_execution"
                    })
                    .collect();
                let text = join_deltas(&parts, "text-delta", None);

                assert_eq!(tool_calls.len(), 2);
                assert_eq!(tool_results.len(), 2);

                let call = |call: &Value| {
                    json!({
                        "type": "tool-call",
                        "toolCallId": call["toolCallId"],
                        "toolName": call["toolName"],
                        "input": serde_json::from_str::<Value>(call["input"].as_str().unwrap()).unwrap(),
                        "providerExecuted": true,
                    })
                };
                let result = |result: &Value| {
                    json!({
                        "type": "tool-result",
                        "toolCallId": result["toolCallId"],
                        "toolName": result["toolName"],
                        "output": { "type": "json", "value": result["result"] },
                    })
                };
                stream_parts(
                    &model,
                    json!({
                        "prompt": [
                            test_prompt()[0],
                            {
                                "role": "assistant",
                                "content": [
                                    call(tool_calls[0]),
                                    result(tool_results[0]),
                                    call(tool_calls[1]),
                                    result(tool_results[1]),
                                    { "type": "text", "text": text },
                                ],
                            },
                        ],
                        "tools": tools,
                    }),
                )
                .await;

                let replay_request = request_body(&mock, 1);
                let expected_transcript = json!([
                    {
                        "type": "server_tool_use",
                        "id": "srvtoolu_011fxGj786xCAh2kPk9GMxQw",
                        "name": "bash_code_execution",
                        "input": {
                            "command": r#"for n in $(seq 1 12); do echo "$n: $((n*n))"; done"#,
                        },
                    },
                    {
                        "type": "bash_code_execution_tool_result",
                        "tool_use_id": "srvtoolu_011fxGj786xCAh2kPk9GMxQw",
                        "content": {
                            "type": "bash_code_execution_result",
                            "stdout": "1: 1\n2: 4\n3: 9\n4: 16\n5: 25\n6: 36\n7: 49\n8: 64\n9: 81\n10: 100\n11: 121\n12: 144\n",
                            "stderr": "",
                            "return_code": 0,
                            "content": [],
                        },
                    },
                    {
                        "type": "server_tool_use",
                        "id": "srvtoolu_013eUksWZnfcjFk1iarJsYgM",
                        "name": "bash_code_execution",
                        "input": {
                            "command": r#"sum=0; for n in $(seq 1 12); do sum=$((sum + n*n)); done; echo "Sum: $sum""#,
                        },
                    },
                    {
                        "type": "bash_code_execution_tool_result",
                        "tool_use_id": "srvtoolu_013eUksWZnfcjFk1iarJsYgM",
                        "content": {
                            "type": "bash_code_execution_result",
                            "stdout": "Sum: 650\n",
                            "stderr": "",
                            "return_code": 0,
                            "content": [],
                        },
                    },
                    {
                        "type": "text",
                        "text": "The sum of the squares of the numbers 1 through 12 is **650**.",
                    },
                ]);
                let replayed_transcript = &replay_request["messages"][1]["content"];

                assert_equals(replayed_transcript, &expected_transcript);
                // Key order included (`JSON.stringify`).
                assert_eq!(
                    serde_json::to_string(replayed_transcript).unwrap(),
                    serde_json::to_string(&expected_transcript).unwrap()
                );
            }
        }

        mod web_fetch_tool {
            use super::*;

            mod txt_response {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › web fetch tool › txt response › should stream web search tool results
                #[tokio::test]
                async fn should_stream_web_search_tool_results() {
                    let mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-web-fetch-tool.1.chunks.txt"
                    ))]);
                    let parts = stream_parts(
                        &test_model(&mock),
                        json!({
                            "prompt": test_prompt(),
                            "tools": [
                                {
                                    "type": "provider",
                                    "id": "anthropic.web_fetch_20250910",
                                    "name": "web_fetch",
                                    "args": {
                                        "maxUses": 1,
                                    },
                                },
                            ],
                        }),
                    )
                    .await;
                    assert_equals(
                        &json!(parts),
                        &snapshot(include_str!("snapshots/stream_web_fetch.json")),
                    );
                }
            }
        }

        mod web_fetch_20260209_tool {
            use super::*;

            mod input_provided_in_content_block_start {
                use super::*;

                /// The `beforeEach`.
                async fn parts() -> Vec<Value> {
                    let mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-web-fetch-tool-20260209.1.chunks.txt"
                    ))]);
                    stream_parts(
                        &test_model(&mock),
                        json!({
                            "prompt": test_prompt(),
                            "tools": [
                                {
                                    "type": "provider",
                                    "id": "anthropic.web_fetch_20260209",
                                    "name": "web_fetch",
                                    "args": {},
                                },
                                {
                                    "type": "provider",
                                    "id": "anthropic.code_execution_20260120",
                                    "name": "code_execution",
                                    "args": {},
                                },
                            ],
                        }),
                    )
                    .await
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › web fetch 20260209 tool › input provided in content_block_start › should include input from content_block_start in web_fetch tool call
                #[tokio::test]
                async fn should_include_input_from_content_block_start_in_web_fetch_tool_call() {
                    let stream_array = parts().await;
                    let tool_call_end = stream_array
                        .iter()
                        .find(|part| part["type"] == "tool-call" && part["toolName"] == "web_fetch")
                        .expect("web_fetch tool call");
                    assert_match_object(
                        tool_call_end,
                        &json!({
                            "type": "tool-call",
                            "toolName": "web_fetch",
                            "input": r#"{"url":"https://example.com"}"#,
                        }),
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › web fetch 20260209 tool › input provided in content_block_start › should stream web fetch 20260209 tool results
                #[tokio::test]
                async fn should_stream_web_fetch_20260209_tool_results() {
                    assert_equals(
                        &json!(parts().await),
                        &snapshot(include_str!("snapshots/stream_web_fetch_20260209.json")),
                    );
                }

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › web fetch 20260209 tool › input provided in content_block_start › should preserve dynamic filtering callers
                #[tokio::test]
                async fn should_preserve_dynamic_filtering_callers() {
                    let stream_array = parts().await;
                    let find = |kind: &str, tool_name: &str| {
                        stream_array
                            .iter()
                            .find(|part| part["type"] == kind && part["toolName"] == tool_name)
                            .cloned()
                            .unwrap_or(Value::Null)
                    };
                    let code_execution_call = find("tool-call", "code_execution");
                    let web_fetch_call = find("tool-call", "web_fetch");
                    let web_fetch_result = find("tool-result", "web_fetch");

                    assert_equals(
                        &code_execution_call["providerMetadata"][CANONICAL]["caller"],
                        &json!({
                            "type": "direct",
                        }),
                    );
                    assert_equals(
                        &web_fetch_call["providerMetadata"][CANONICAL]["caller"],
                        &json!({
                            "type": "code_execution_20260120",
                            "toolId": "srvtoolu_01LKcA5qc1HwvLQSe3cLKmcK",
                        }),
                    );
                    assert_equals(
                        &web_fetch_result["providerMetadata"][CANONICAL]["caller"],
                        &json!({
                            "type": "code_execution_20260120",
                            "toolId": "srvtoolu_01LKcA5qc1HwvLQSe3cLKmcK",
                        }),
                    );
                }
            }
        }

        mod web_search_tool {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › web search tool › should stream web search tool results
            #[tokio::test]
            async fn should_stream_web_search_tool_results() {
                let mock = server(vec![sse_fixture(include_str!(
                    "fixtures/anthropic-web-search-tool.1.chunks.txt"
                ))]);
                // Upstream's shared `mockId` counter stands at 27 when this test
                // runs (the snapshot's first source id); the counter starts there.
                let parts = stream_parts(
                    &test_provider_with_ids(&mock, 27).messages("claude-3-haiku-20240307"),
                    json!({
                        "prompt": test_prompt(),
                        "tools": [
                            {
                                "type": "provider",
                                "id": "anthropic.web_search_20250305",
                                "name": "web_search",
                                "args": {
                                    "maxUses": 1,
                                    "userLocation": {
                                        "type": "approximate",
                                        "country": "US",
                                    },
                                },
                            },
                        ],
                    }),
                )
                .await;
                assert_equals(
                    &json!(parts),
                    &snapshot(include_str!("snapshots/stream_web_search.json")),
                );
            }
        }

        mod tool_search_tool {
            use super::*;

            mod regex_variant {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › tool search tool › regex variant › should stream tool search regex results
                #[tokio::test]
                async fn should_stream_tool_search_regex_results() {
                    let mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-tool-search-regex.1.chunks.txt"
                    ))]);
                    let parts = stream_parts(
                        &test_provider(&mock).messages("claude-sonnet-4-5"),
                        json!({
                            "prompt": [
                                {
                                    "role": "user",
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": "Find out weather data in SF",
                                        },
                                    ],
                                },
                            ],
                            "tools": [
                                {
                                    "type": "provider",
                                    "id": "anthropic.tool_search_regex_20251119",
                                    "name": "tool_search",
                                    "args": {},
                                },
                                {
                                    "type": "function",
                                    "name": "get_temp_data",
                                    "description": "For a location",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "location": {
                                                "type": "string",
                                            },
                                            "unit": {
                                                "type": "string",
                                                "enum": [
                                                    "celsius",
                                                    "fahrenheit",
                                                ],
                                            },
                                        },
                                    },
                                    "providerOptions": {
                                        CANONICAL: {
                                            "deferLoading": true,
                                        },
                                    },
                                },
                            ],
                        }),
                    )
                    .await;
                    assert_equals(
                        &json!(parts),
                        &snapshot(include_str!("snapshots/stream_tool_search_regex.json")),
                    );
                }
            }

            mod bm25_variant {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › tool search tool › bm25 variant › should stream tool search bm25 results
                #[tokio::test]
                async fn should_stream_tool_search_bm25_results() {
                    let mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-tool-search-bm25.1.chunks.txt"
                    ))]);
                    let parts = stream_parts(
                        &test_provider(&mock).messages("claude-sonnet-4-5"),
                        json!({
                            "prompt": [
                                {
                                    "role": "user",
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": "What is the weather in San Francisco?",
                                        },
                                    ],
                                },
                            ],
                            "tools": [
                                {
                                    "type": "provider",
                                    "id": "anthropic.tool_search_bm25_20251119",
                                    "name": "tool_search",
                                    "args": {},
                                },
                                {
                                    "type": "function",
                                    "name": "get_weather",
                                    "description": "Get the current weather at a specific location",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "location": {
                                                "type": "string",
                                            },
                                            "unit": {
                                                "type": "string",
                                                "enum": [
                                                    "celsius",
                                                    "fahrenheit",
                                                ],
                                            },
                                        },
                                    },
                                    "providerOptions": {
                                        CANONICAL: {
                                            "deferLoading": true,
                                        },
                                    },
                                },
                            ],
                        }),
                    )
                    .await;
                    assert_equals(
                        &json!(parts),
                        &snapshot(include_str!("snapshots/stream_tool_search_bm25.json")),
                    );
                }
            }

            mod deferred_result_bm25_variant {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › tool search tool › deferred result - bm25 variant › should correctly map tool_search_tool_result when result comes without server_tool_use in same response
                #[tokio::test]
                async fn should_correctly_map_tool_search_tool_result_when_result_comes_without_server_tool_use_in_same_response()
                 {
                    let mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-tool-search-deferred-bm25.chunks.txt"
                    ))]);
                    let chunks = stream_parts(
                        &test_provider(&mock).messages("claude-sonnet-4-5"),
                        json!({
                            "prompt": [
                                {
                                    "role": "user",
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": "What is the weather in San Francisco?",
                                        },
                                    ],
                                },
                            ],
                            "tools": [
                                {
                                    "type": "provider",
                                    "id": "anthropic.tool_search_bm25_20251119",
                                    "name": "tool_search",
                                    "args": {},
                                },
                                {
                                    "type": "function",
                                    "name": "get_weather",
                                    "description": "Get the current weather at a specific location",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "location": {
                                                "type": "string",
                                            },
                                        },
                                    },
                                    "providerOptions": {
                                        CANONICAL: {
                                            "deferLoading": true,
                                        },
                                    },
                                },
                            ],
                        }),
                    )
                    .await;
                    let tool_result_chunk = find_part(&chunks, "tool-result").expect("tool-result");
                    assert_eq!(tool_result_chunk["toolName"], "tool_search");
                }
            }

            mod deferred_result_regex_variant {
                use super::*;

                /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › tool search tool › deferred result - regex variant › should correctly map tool_search_tool_result when result comes without server_tool_use in same response
                #[tokio::test]
                async fn should_correctly_map_tool_search_tool_result_when_result_comes_without_server_tool_use_in_same_response()
                 {
                    let mock = server(vec![sse_fixture(include_str!(
                        "fixtures/anthropic-tool-search-deferred-regex.chunks.txt"
                    ))]);
                    let chunks = stream_parts(
                        &test_provider(&mock).messages("claude-sonnet-4-5"),
                        json!({
                            "prompt": [
                                {
                                    "role": "user",
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": "Find weather data in NYC",
                                        },
                                    ],
                                },
                            ],
                            "tools": [
                                {
                                    "type": "provider",
                                    "id": "anthropic.tool_search_regex_20251119",
                                    "name": "tool_search",
                                    "args": {},
                                },
                                {
                                    "type": "function",
                                    "name": "get_temp_data",
                                    "description": "For a location",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "location": {
                                                "type": "string",
                                            },
                                        },
                                    },
                                    "providerOptions": {
                                        CANONICAL: {
                                            "deferLoading": true,
                                        },
                                    },
                                },
                            ],
                        }),
                    )
                    .await;
                    let tool_result_chunk = find_part(&chunks, "tool-result").expect("tool-result");
                    assert_eq!(tool_result_chunk["toolName"], "tool_search");
                }
            }
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should throw an api error when the server is returning a 529 overloaded error
        #[tokio::test]
        async fn should_throw_an_api_error_when_the_server_is_returning_a_529_overloaded_error() {
            let body = r#"{"type":"error","error":{"details":null,"type":"overloaded_error","message":"Overloaded"}}"#;
            let mock = server(vec![Canned {
                status: 529,
                headers: vec![
                    ("content-type".into(), "text/plain".into()),
                    ("content-length".into(), body.len().to_string()),
                ],
                body: body.as_bytes().to_vec(),
            }]);
            let Err(error) = test_model(&mock)
                .do_stream(&call_options(json!({ "prompt": test_prompt() })))
                .await
            else {
                panic!("Expected an error to be thrown");
            };
            assert_equals(
                &api_call_error_json(&error),
                &json!({
                    "isRetryable": true,
                    "message": "Overloaded",
                    "requestBodyValues": {
                        "max_tokens": 4096,
                        "messages": [
                            {
                                "content": [
                                    {
                                        "text": "Hello",
                                        "type": "text",
                                    },
                                ],
                                "role": "user",
                            },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stream": true,
                    },
                    "responseBody": r#"{"type":"error","error":{"details":null,"type":"overloaded_error","message":"Overloaded"}}"#,
                    "responseHeaders": {
                        "content-length": "90",
                        "content-type": "text/plain",
                    },
                    "statusCode": 529,
                    "url": "https://api.anthropic.com/v1/messages",
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should throw an api error when the first stream chunk is an overloaded error
        #[tokio::test]
        async fn should_throw_an_api_error_when_the_first_stream_chunk_is_an_overloaded_error() {
            let mock = server(vec![sse_raw(&[
                "event: error\n",
                "data: {\"type\":\"error\",\"error\":{\"details\":null,\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n",
                "\n",
            ])]);
            let Err(error) = test_model(&mock)
                .do_stream(&call_options(json!({ "prompt": test_prompt() })))
                .await
            else {
                panic!("Expected an error to be thrown");
            };
            assert_equals(
                &api_call_error_json(&error),
                &json!({
                    "isRetryable": true,
                    "message": "Overloaded",
                    "requestBodyValues": {
                        "max_tokens": 4096,
                        "messages": [
                            {
                                "content": [
                                    {
                                        "text": "Hello",
                                        "type": "text",
                                    },
                                ],
                                "role": "user",
                            },
                        ],
                        "model": "claude-3-haiku-20240307",
                        "stream": true,
                    },
                    "responseBody": r#"{"type":"overloaded_error","message":"Overloaded"}"#,
                    "responseHeaders": {
                        "cache-control": "no-cache",
                        "connection": "keep-alive",
                        "content-type": "text/event-stream",
                    },
                    "statusCode": 529,
                    "url": "https://api.anthropic.com/v1/messages",
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should attach provider-owned metadata to a mid-stream $type error
        #[tokio::test]
        async fn should_attach_provider_owned_metadata_to_a_mid_stream_type_error() {
            for (error_type, message, status_code, is_retryable) in [
                ("overloaded_error", "Overloaded", 529, true),
                ("api_error", "Internal server error", 500, true),
                ("request_too_large", "Request too large", 413, false),
            ] {
                let error_frame = serde_json::to_string(&json!({
                    "type": "error",
                    "error": { "type": error_type, "message": message },
                }))
                .unwrap();
                let mock = server(vec![sse_raw(&[
                    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01KfpJoAEabmH2iHRRFjQMAG\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-3-haiku-20240307\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":17,\"output_tokens\":1}}}\n\n",
                    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
                    "event: error\n",
                    &format!("data: {error_frame}\n\n"),
                ])]);
                let chunks =
                    stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
                let error_part = find_part(&chunks, "error");

                assert_match_object(
                    &json!(chunks),
                    &json!([
                        {
                            "type": "stream-start",
                            "warnings": [],
                        },
                        {
                            "type": "response-metadata",
                            "id": "msg_01KfpJoAEabmH2iHRRFjQMAG",
                            "modelId": "claude-3-haiku-20240307",
                        },
                        {
                            "type": "text-start",
                            "id": "0",
                        },
                        {
                            "type": "text-delta",
                            "id": "0",
                            "delta": "Hello",
                        },
                        {
                            "type": "error",
                        },
                    ]),
                );
                let error_part = error_part.expect("Expected an error part");
                assert_match_object(
                    &error_part["error"],
                    &json!({
                        "message": message,
                        "type": error_type,
                        "statusCode": status_code,
                        "isRetryable": is_retryable,
                        "data": { "message": message, "type": error_type },
                    }),
                );
            }
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › doStream › should preserve explicit metadata from an Anthropic-compatible transport
        #[tokio::test]
        async fn should_preserve_explicit_metadata_from_an_anthropic_compatible_transport() {
            let data = json!({
                "message": "The model stream failed",
                "originalStatusCode": 503,
            });
            let error = json!({
                "type": "modelStreamErrorException",
                "code": "bedrock_stream_error",
                "message": data["message"],
                "statusCode": 424,
                "isRetryable": true,
                "data": data,
            });
            let frame = serde_json::to_string(&json!({ "type": "error", "error": error })).unwrap();
            let mock = server(vec![sse_raw(&[
                "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01KfpJoAEabmH2iHRRFjQMAG\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-3-haiku-20240307\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":17,\"output_tokens\":1}}}\n\n",
                "event: error\n",
                &format!("data: {frame}\n\n"),
            ])]);
            let chunks = stream_parts(&test_model(&mock), json!({ "prompt": test_prompt() })).await;
            let error_part = find_part(&chunks, "error").expect("Expected an error part");
            assert_match_object(&error_part["error"], &error);
        }
    }

    mod transform_request_body {
        use super::*;

        fn prepare_transform_json_response() -> Canned {
            json_response(json!({
                "id": "msg_test",
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "Hello!",
                    },
                ],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 4,
                    "output_tokens": 30,
                },
            }))
        }

        fn prepare_transform_stream_response() -> Canned {
            sse_data(&[
                r#"{"type":"message_start","message":{"id":"msg_test","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":4,"output_tokens":1}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":2}}"#,
                r#"{"type":"message_stop"}"#,
            ])
        }

        /// `new AnthropicLanguageModel('claude-3-haiku-20240307', { provider: 'test-provider',
        /// baseURL, headers: {}, transformRequestBody })`: the provider's model config
        /// with that identity and transform. The calls the transform received are
        /// recorded (aimux's transform takes the body only, not the beta set).
        fn transform_model(
            mock: &Arc<MockFetch>,
            transform: bool,
        ) -> (AnthropicMessagesModel, Arc<Mutex<Vec<Value>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let provider = plain_provider(mock);
            let mut config = provider.model_config("test-provider".into());
            if transform {
                let seen = calls.clone();
                config.transform_request_body = Some(Arc::new(move |mut body: Value| {
                    seen.lock().unwrap().push(body.clone());
                    body["custom_field"] = json!("added-by-transform");
                    body
                }));
            }
            (
                AnthropicMessagesModel::with_config("claude-3-haiku-20240307".into(), config),
                calls,
            )
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › transformRequestBody › should transform request body in doGenerate when transformRequestBody is provided
        #[tokio::test]
        async fn should_transform_request_body_in_do_generate_when_transform_request_body_is_provided()
         {
            let mock = server(vec![prepare_transform_json_response()]);
            let (model, calls) = transform_model(&mock, true);
            generate(&model, json!({ "prompt": test_prompt() })).await;

            // Verify transform was called
            let calls = calls.lock().unwrap().clone();
            assert_eq!(calls.len(), 1);
            assert!(
                equals(
                    &calls[0],
                    &json!({ "$objectContaining": {
                    "model": "claude-3-haiku-20240307",
                    "messages": [
                        { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                    ],
                } })
                ),
                "{:#}",
                calls[0]
            );

            // Verify transformed body was sent
            assert_match_object(
                &request_body(&mock, 0),
                &json!({
                    "custom_field": "added-by-transform",
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › transformRequestBody › should transform request body in doStream when transformRequestBody is provided
        #[tokio::test]
        async fn should_transform_request_body_in_do_stream_when_transform_request_body_is_provided()
         {
            let mock = server(vec![prepare_transform_stream_response()]);
            let (model, calls) = transform_model(&mock, true);
            stream_parts(&model, json!({ "prompt": test_prompt() })).await;

            // Verify transform was called
            let calls = calls.lock().unwrap().clone();
            assert_eq!(calls.len(), 1);
            assert!(
                equals(
                    &calls[0],
                    &json!({ "$objectContaining": {
                    "model": "claude-3-haiku-20240307",
                    "messages": [
                        { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
                    ],
                    "stream": true,
                } })
                ),
                "{:#}",
                calls[0]
            );

            // Verify transformed body was sent
            assert_match_object(
                &request_body(&mock, 0),
                &json!({
                    "custom_field": "added-by-transform",
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › transformRequestBody › should work without transformRequestBody
        #[tokio::test]
        async fn should_work_without_transform_request_body() {
            let mock = server(vec![prepare_transform_json_response()]);
            let (model, _) = transform_model(&mock, false);
            generate(&model, json!({ "prompt": test_prompt() })).await;

            let request_body = request_body(&mock, 0);
            assert_match_object(
                &request_body,
                &json!({
                    "model": "claude-3-haiku-20240307",
                }),
            );
            assert!(request_body.get("custom_field").is_none());
        }
    }

    mod custom_provider_name_support {
        use super::*;

        fn create_custom_model(
            mock: &Arc<MockFetch>,
            provider_name: &str,
        ) -> AnthropicMessagesModel {
            create_anthropic(AnthropicProviderSettings {
                api_key: Some("test-api-key".into()),
                name: Some(provider_name.into()),
                generate_id: Some(mock_id_from("id", 0)),
                fetch: Some(mock.transport()),
                ..Default::default()
            })
            .unwrap()
            .messages("claude-3-haiku-20240307")
        }

        fn prepare_custom_json_response() -> Canned {
            json_response(json!({
                "id": "msg_custom_test",
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "Hello, World!",
                    },
                ],
                "model": "claude-3-haiku-20240307",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 20,
                },
            }))
        }

        fn prepare_custom_stream_response() -> Canned {
            let chunks: [&str; 7] = [
                r#"{"type":"message_start","message":{"id":"msg_custom_stream","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":0}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" World!"}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":20}}"#,
                r#"{"type":"message_stop"}"#,
            ];
            let mut all: Vec<&str> = chunks.to_vec();
            all.push("[DONE]");
            sse_data(&all)
        }

        fn metadata_keys(metadata: &Value) -> Vec<String> {
            let mut keys: Vec<String> = metadata
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            keys.sort();
            keys
        }

        mod do_generate {
            use super::*;

            // Case 1: providerOptions with 'anthropic' key → only 'anthropic' in providerMetadata
            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doGenerate › should only include "anthropic" key in providerMetadata when providerOptions uses "anthropic" key
            #[tokio::test]
            async fn should_only_include_anthropic_key_in_provider_metadata_when_provider_options_uses_anthropic_key()
             {
                let mock = server(vec![prepare_custom_json_response()]);
                let custom_model = create_custom_model(&mock, "my-custom-anthropic");
                let result = generate(
                    &custom_model,
                    json!({
                        "prompt": test_prompt(),
                        "providerOptions": {
                            CANONICAL: {
                                "sendReasoning": true,
                            },
                        },
                    }),
                )
                .await;

                // Only 'anthropic' key should be in providerMetadata
                let metadata = serde_json::to_value(&result.provider_metadata).unwrap();
                assert!(metadata.get(CANONICAL).is_some());
                assert_eq!(metadata_keys(&metadata), [CANONICAL]);
            }

            // Case 2: providerOptions with custom key → both 'anthropic' and custom key in providerMetadata
            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doGenerate › should include both "anthropic" and custom key in providerMetadata when providerOptions uses custom key
            #[tokio::test]
            async fn should_include_both_anthropic_and_custom_key_in_provider_metadata_when_provider_options_uses_custom_key()
             {
                let mock = server(vec![prepare_custom_json_response()]);
                let custom_model = create_custom_model(&mock, "my-custom-anthropic");
                let result = generate(
                    &custom_model,
                    json!({
                        "prompt": test_prompt(),
                        "providerOptions": {
                            "my-custom-anthropic": {
                                "sendReasoning": true,
                            },
                        },
                    }),
                )
                .await;

                // Both 'anthropic' and custom key should be in providerMetadata
                let metadata = serde_json::to_value(&result.provider_metadata).unwrap();
                assert!(metadata.get(CANONICAL).is_some(), "{metadata:#}");
                assert!(
                    metadata.get("my-custom-anthropic").is_some(),
                    "{metadata:#}"
                );
            }

            // Fallback: no providerOptions → only 'anthropic' in providerMetadata
            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doGenerate › should only include "anthropic" key in providerMetadata when no providerOptions used
            #[tokio::test]
            async fn should_only_include_anthropic_key_in_provider_metadata_when_no_provider_options_used()
             {
                let mock = server(vec![prepare_custom_json_response()]);
                let custom_model = create_custom_model(&mock, "my-custom-anthropic");
                let result = generate(
                    &custom_model,
                    json!({
                        "prompt": test_prompt(),
                    }),
                )
                .await;

                // Only 'anthropic' key should be in providerMetadata (default)
                let metadata = serde_json::to_value(&result.provider_metadata).unwrap();
                assert!(metadata.get(CANONICAL).is_some());
                assert_eq!(metadata_keys(&metadata), [CANONICAL]);
            }

            // providerOptions parsing tests
            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doGenerate › should accept providerOptions with custom provider name key
            #[tokio::test]
            async fn should_accept_provider_options_with_custom_provider_name_key() {
                let mock = server(vec![prepare_custom_json_response()]);
                let custom_model = create_custom_model(&mock, "my-custom-anthropic");
                generate(
                    &custom_model,
                    json!({
                        "prompt": test_prompt(),
                        "providerOptions": {
                            "my-custom-anthropic": {
                                "disableParallelToolUse": true,
                            },
                        },
                        "tools": [
                            {
                                "type": "function",
                                "name": "testTool",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {},
                                },
                            },
                        ],
                    }),
                )
                .await;
                assert_match_object(
                    &request_body(&mock, 0),
                    &json!({
                        "tool_choice": {
                            "type": "auto",
                            "disable_parallel_tool_use": true,
                        },
                    }),
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doGenerate › should accept providerOptions with canonical "anthropic" key for backward compatibility
            #[tokio::test]
            async fn should_accept_provider_options_with_canonical_anthropic_key_for_backward_compatibility()
             {
                let mock = server(vec![prepare_custom_json_response()]);
                let custom_model = create_custom_model(&mock, "my-custom-anthropic");
                generate(
                    &custom_model,
                    json!({
                        "prompt": test_prompt(),
                        "providerOptions": {
                            CANONICAL: {
                                "disableParallelToolUse": true,
                            },
                        },
                        "tools": [
                            {
                                "type": "function",
                                "name": "testTool",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {},
                                },
                            },
                        ],
                    }),
                )
                .await;
                assert_match_object(
                    &request_body(&mock, 0),
                    &json!({
                        "tool_choice": {
                            "type": "auto",
                            "disable_parallel_tool_use": true,
                        },
                    }),
                );
            }
        }

        mod do_stream {
            use super::*;

            async fn finish_metadata(options: Value) -> Value {
                let mock = server(vec![prepare_custom_stream_response()]);
                let custom_model = create_custom_model(&mock, "my-custom-anthropic");
                let chunks = stream_parts(&custom_model, options).await;
                let finish_chunk = find_part(&chunks, "finish").expect("finish chunk");
                finish_chunk["providerMetadata"].clone()
            }

            // Case 1: providerOptions with 'anthropic' key → only 'anthropic' in providerMetadata
            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doStream › should only include "anthropic" key in providerMetadata when providerOptions uses "anthropic" key
            #[tokio::test]
            async fn should_only_include_anthropic_key_in_provider_metadata_when_provider_options_uses_anthropic_key()
             {
                let metadata = finish_metadata(json!({
                    "prompt": test_prompt(),
                    "providerOptions": {
                        CANONICAL: {
                            "sendReasoning": true,
                        },
                    },
                }))
                .await;

                // Only 'anthropic' key should be in providerMetadata
                assert!(metadata.get(CANONICAL).is_some());
                assert_eq!(metadata_keys(&metadata), [CANONICAL]);
            }

            // Case 2: providerOptions with custom key → both 'anthropic' and custom key in providerMetadata
            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doStream › should include both "anthropic" and custom key in providerMetadata when providerOptions uses custom key
            #[tokio::test]
            async fn should_include_both_anthropic_and_custom_key_in_provider_metadata_when_provider_options_uses_custom_key()
             {
                let metadata = finish_metadata(json!({
                    "prompt": test_prompt(),
                    "providerOptions": {
                        "my-custom-anthropic": {
                            "sendReasoning": true,
                        },
                    },
                }))
                .await;

                // Both 'anthropic' and custom key should be in providerMetadata
                assert!(metadata.get(CANONICAL).is_some(), "{metadata:#}");
                assert!(
                    metadata.get("my-custom-anthropic").is_some(),
                    "{metadata:#}"
                );
            }

            // Fallback: no providerOptions → only 'anthropic' in providerMetadata
            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doStream › should only include "anthropic" key in providerMetadata when no providerOptions used
            #[tokio::test]
            async fn should_only_include_anthropic_key_in_provider_metadata_when_no_provider_options_used()
             {
                let metadata = finish_metadata(json!({
                    "prompt": test_prompt(),
                }))
                .await;

                // Only 'anthropic' key should be in providerMetadata (default)
                assert!(metadata.get(CANONICAL).is_some());
                assert_eq!(metadata_keys(&metadata), [CANONICAL]);
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › AnthropicLanguageModel › custom provider name support › doStream › should accept providerOptions with custom provider name key
            #[tokio::test]
            async fn should_accept_provider_options_with_custom_provider_name_key() {
                let mock = server(vec![prepare_custom_stream_response()]);
                let custom_model = create_custom_model(&mock, "my-custom-anthropic");
                stream_parts(
                    &custom_model,
                    json!({
                        "prompt": test_prompt(),
                        "providerOptions": {
                            "my-custom-anthropic": {
                                "disableParallelToolUse": true,
                            },
                        },
                        "tools": [
                            {
                                "type": "function",
                                "name": "testTool",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {},
                                },
                            },
                        ],
                    }),
                )
                .await;
                assert_match_object(
                    &request_body(&mock, 0),
                    &json!({
                        "tool_choice": {
                            "type": "auto",
                            "disable_parallel_tool_use": true,
                        },
                    }),
                );
            }
        }
    }
}

/// `getModelCapabilities(modelId)` in upstream's JSON shape.
fn model_capabilities(model_id: &str) -> Value {
    let caps = crate::anthropic::convert::get_model_capabilities(model_id);
    json!({
        "isKnownModel": caps.is_known_model,
        "maxOutputTokens": caps.max_output_tokens,
        "rejectsForcedToolUse": caps.rejects_forced_tool_use,
        "rejectsSamplingParameters": caps.rejects_sampling_parameters,
        "rejectsThinkingDisabled": caps.rejects_thinking_disabled,
        "rejectsThinkingDisabledAboveHighEffort": caps.rejects_thinking_disabled_above_high_effort,
        "supportsAdaptiveThinking": caps.supports_adaptive_thinking,
        "supportsBetweenToolsThinking": caps.supports_between_tools_thinking,
        "supportsStructuredOutput": caps.supports_structured_output,
        "supportsXhighEffort": caps.supports_xhigh_effort,
    })
}

/// A `doGenerate` against an `anthropic-text.json` answer: the result and the transport.
async fn generate_text_fixture(
    model: impl Fn(&Arc<MockFetch>) -> AnthropicMessagesModel,
    options: Value,
) -> (GenerateResult, Arc<MockFetch>) {
    let mock = server(vec![json_fixture(include_str!(
        "fixtures/anthropic-text.json"
    ))]);
    let result = generate(&model(&mock), options).await;
    (result, mock)
}

mod get_model_capabilities {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return correct capabilities for claude-opus-4-8
    #[test]
    fn should_return_correct_capabilities_for_claude_opus_4_8() {
        assert_equals(
            &model_capabilities("claude-opus-4-8"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": false,
                "rejectsThinkingDisabledAboveHighEffort": false,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return correct capabilities for claude-fable-5
    #[test]
    fn should_return_correct_capabilities_for_claude_fable_5() {
        assert_equals(
            &model_capabilities("claude-fable-5"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": true,
                "rejectsThinkingDisabledAboveHighEffort": false,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return correct capabilities for claude-fable-5-1
    #[test]
    fn should_return_correct_capabilities_for_claude_fable_5_1() {
        assert_equals(
            &model_capabilities("claude-fable-5-1"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": true,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": true,
                "rejectsThinkingDisabledAboveHighEffort": false,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return correct capabilities for claude-opus-4-7
    #[test]
    fn should_return_correct_capabilities_for_claude_opus_4_7() {
        assert_equals(
            &model_capabilities("claude-opus-4-7"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": false,
                "rejectsThinkingDisabledAboveHighEffort": false,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return correct capabilities for claude-sonnet-5
    #[test]
    fn should_return_correct_capabilities_for_claude_sonnet_5() {
        assert_equals(
            &model_capabilities("claude-sonnet-5"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": false,
                "rejectsThinkingDisabledAboveHighEffort": false,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return correct capabilities for claude-opus-4-6
    #[test]
    fn should_return_correct_capabilities_for_claude_opus_4_6() {
        let caps = model_capabilities("claude-opus-4-6");
        assert_eq!(caps["rejectsSamplingParameters"], false);
        assert_eq!(caps["supportsXhighEffort"], false);
        assert_eq!(caps["supportsAdaptiveThinking"], true);
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return correct capabilities for claude-sonnet-4-6
    #[test]
    fn should_return_correct_capabilities_for_claude_sonnet_4_6() {
        let caps = model_capabilities("claude-sonnet-4-6");
        assert_eq!(caps["rejectsSamplingParameters"], false);
        assert_eq!(caps["supportsXhighEffort"], false);
        assert_eq!(caps["supportsAdaptiveThinking"], true);
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return correct capabilities for claude-opus-5
    #[test]
    fn should_return_correct_capabilities_for_claude_opus_5() {
        assert_equals(
            &model_capabilities("claude-opus-5"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": false,
                "rejectsThinkingDisabledAboveHighEffort": true,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return current-generation capabilities for an unknown Claude model
    #[test]
    fn should_return_current_generation_capabilities_for_an_unknown_claude_model() {
        assert_equals(
            &model_capabilities("claude-future-9"),
            &json!({
                "isKnownModel": false,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": false,
                "rejectsThinkingDisabledAboveHighEffort": true,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should recognize an unknown platform-prefixed Claude model
    #[test]
    fn should_recognize_an_unknown_platform_prefixed_claude_model() {
        assert_equals(
            &model_capabilities("us.anthropic.claude-future-9-20990101-v1:0"),
            &json!({
                "isKnownModel": false,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": false,
                "rejectsThinkingDisabledAboveHighEffort": true,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should retain conservative capabilities for legacy Claude model %s
    #[test]
    fn should_retain_conservative_capabilities_for_legacy_claude_model_s() {
        for model_id in [
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
            "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
            "anthropic.claude-v2:1",
            "anthropic.claude-instant-v1",
        ] {
            assert_eq!(
                model_capabilities(model_id),
                json!({
                    "isKnownModel": false,
                    "maxOutputTokens": 4096,
                    "rejectsForcedToolUse": false,
                    "rejectsSamplingParameters": false,
                    "rejectsThinkingDisabled": false,
                    "rejectsThinkingDisabledAboveHighEffort": false,
                    "supportsAdaptiveThinking": false,
                    "supportsBetweenToolsThinking": false,
                    "supportsStructuredOutput": false,
                    "supportsXhighEffort": false,
                }),
                "{model_id}"
            );
        }
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should match a known model before the forward-compatible fallback
    #[test]
    fn should_match_a_known_model_before_the_forward_compatible_fallback() {
        assert_equals(
            &model_capabilities("claude-opus-4-5"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 64000,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": false,
                "rejectsThinkingDisabled": false,
                "rejectsThinkingDisabledAboveHighEffort": false,
                "supportsAdaptiveThinking": false,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": false,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should recognize the Vertex model ID %s
    #[test]
    fn should_recognize_the_vertex_model_id_s() {
        for (model_id, max_output_tokens) in [
            ("claude-sonnet-4@20250514", 64000),
            ("claude-opus-4@20250514", 32000),
        ] {
            assert_match_object(
                &model_capabilities(model_id),
                &json!({ "isKnownModel": true, "maxOutputTokens": max_output_tokens }),
            );
        }
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › getModelCapabilities › should return conservative capabilities for an unknown non-Claude model
    #[test]
    fn should_return_conservative_capabilities_for_an_unknown_non_claude_model() {
        assert_equals(
            &model_capabilities("third-party-future-model"),
            &json!({
                "isKnownModel": false,
                "maxOutputTokens": 4096,
                "rejectsForcedToolUse": false,
                "rejectsSamplingParameters": false,
                "rejectsThinkingDisabled": false,
                "rejectsThinkingDisabledAboveHighEffort": false,
                "supportsAdaptiveThinking": false,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": false,
                "supportsXhighEffort": false,
            }),
        );
    }
}

mod effort_with_thinking_disabled {
    use super::*;

    fn provider(mock: &Arc<MockFetch>, model_id: &str) -> AnthropicMessagesModel {
        plain_provider(mock).messages(model_id)
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › effort with thinking disabled › should warn and lower effort to high when thinking is disabled with effort %s on an unknown Claude model
    #[tokio::test]
    async fn should_warn_and_lower_effort_to_high_when_thinking_is_disabled_with_effort_s_on_an_unknown_claude_model()
     {
        for effort in ["xhigh", "max"] {
            let (result, mock) = generate_text_fixture(
                |mock| provider(mock, "claude-future-9"),
                json!({
                    "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }],
                    "maxOutputTokens": 1024,
                    "providerOptions": {
                        CANONICAL: {
                            "thinking": { "type": "disabled" },
                            "effort": effort,
                        },
                    },
                }),
            )
            .await;

            let request_body = request_body(&mock, 0);
            assert_eq!(request_body["output_config"]["effort"], "high", "{effort}");
            assert_equals(
                &request_body["thinking"],
                &json!({
                    "type": "disabled",
                }),
            );
            let warnings = warnings_json(&result.warnings);
            assert_contains_equal(
                warnings.as_array().unwrap(),
                &json!({
                    "$objectContaining": {
                        "type": "unsupported",
                        "feature": "providerOptions.anthropic.effort",
                    },
                }),
            );
        }
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › effort with thinking disabled › should keep effort high when thinking is disabled with effort high
    #[tokio::test]
    async fn should_keep_effort_high_when_thinking_is_disabled_with_effort_high() {
        let (result, mock) = generate_text_fixture(
            |mock| provider(mock, "claude-future-9"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "maxOutputTokens": 1024,
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "disabled",
                        },
                        "effort": "high",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_eq!(request_body["output_config"]["effort"], "high");
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › effort with thinking disabled › should not lower effort for models without the constraint
    #[tokio::test]
    async fn should_not_lower_effort_for_models_without_the_constraint() {
        let (_, mock) = generate_text_fixture(
            |mock| provider(mock, "claude-opus-4-8"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "disabled",
                        },
                        "effort": "xhigh",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_eq!(request_body["output_config"]["effort"], "xhigh");
    }
}

mod claude_opus_4_7_specific_behavior {
    use super::*;

    fn provider(mock: &Arc<MockFetch>, model_id: &str) -> AnthropicMessagesModel {
        plain_provider(mock).messages(model_id)
    }

    fn opus_model(mock: &Arc<MockFetch>) -> AnthropicMessagesModel {
        provider(mock, "claude-opus-4-7")
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should warn and strip temperature when set
    #[tokio::test]
    async fn should_warn_and_strip_temperature_when_set() {
        let (result, mock) = generate_text_fixture(
            opus_model,
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "temperature": 0.7,
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert!(
            request_body.get("temperature").is_none(),
            "{request_body:#}"
        );
        assert_contains_equal(
            warnings_json(&result.warnings).as_array().unwrap(),
            &json!({
                "$objectContaining": {
                    "type": "unsupported",
                    "feature": "temperature",
                },
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should warn and strip topK when set
    #[tokio::test]
    async fn should_warn_and_strip_top_k_when_set() {
        let (result, mock) = generate_text_fixture(
            opus_model,
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "topK": 40,
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert!(request_body.get("top_k").is_none(), "{request_body:#}");
        assert_contains_equal(
            warnings_json(&result.warnings).as_array().unwrap(),
            &json!({
                "$objectContaining": {
                    "type": "unsupported",
                    "feature": "topK",
                },
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should warn and strip topP when set
    #[tokio::test]
    async fn should_warn_and_strip_top_p_when_set() {
        let (result, mock) = generate_text_fixture(
            opus_model,
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "topP": 0.9,
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert!(request_body.get("top_p").is_none(), "{request_body:#}");
        assert_contains_equal(
            warnings_json(&result.warnings).as_array().unwrap(),
            &json!({
                "$objectContaining": {
                    "type": "unsupported",
                    "feature": "topP",
                },
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should map xhigh reasoning effort to xhigh for claude-opus-4-7
    #[tokio::test]
    async fn should_map_xhigh_reasoning_effort_to_xhigh_for_claude_opus_4_7() {
        let (_, mock) = generate_text_fixture(
            opus_model,
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "reasoning": "xhigh",
            }),
        )
        .await;
        assert_eq!(
            request_body(&mock, 0).pointer("/output_config/effort"),
            Some(&json!("xhigh"))
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should map xhigh reasoning effort to max for claude-opus-4-6
    #[tokio::test]
    async fn should_map_xhigh_reasoning_effort_to_max_for_claude_opus_4_6() {
        let (_, mock) = generate_text_fixture(
            |mock| provider(mock, "claude-opus-4-6"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "reasoning": "xhigh",
            }),
        )
        .await;
        assert_eq!(
            request_body(&mock, 0).pointer("/output_config/effort"),
            Some(&json!("max"))
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should include task_budget in output_config and add beta header
    #[tokio::test]
    async fn should_include_task_budget_in_output_config_and_add_beta_header() {
        let (_, mock) = generate_text_fixture(
            opus_model,
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "providerOptions": {
                    CANONICAL: {
                        "taskBudget": {
                            "type": "tokens",
                            "total": 400000,
                        },
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["output_config"]["task_budget"],
            &json!({
                "type": "tokens",
                "total": 400000,
            }),
        );
        let beta_header = request_header(&mock, 0, "anthropic-beta").unwrap_or_default();
        assert!(
            beta_header.contains("task-budgets-2026-03-13"),
            "{beta_header}"
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should include remaining in task_budget when provided
    #[tokio::test]
    async fn should_include_remaining_in_task_budget_when_provided() {
        let (_, mock) = generate_text_fixture(
            opus_model,
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "providerOptions": {
                    CANONICAL: {
                        "taskBudget": {
                            "type": "tokens",
                            "total": 400000,
                            "remaining": 215000,
                        },
                    },
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["output_config"]["task_budget"],
            &json!({
                "type": "tokens",
                "total": 400000,
                "remaining": 215000,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should include display in thinking block when set
    #[tokio::test]
    async fn should_include_display_in_thinking_block_when_set() {
        let (_, mock) = generate_text_fixture(
            opus_model,
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "adaptive",
                            "display": "summarized",
                        },
                    },
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["thinking"],
            &json!({
                "type": "adaptive",
                "display": "summarized",
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should include the updates thinking display mode
    #[tokio::test]
    async fn should_include_the_updates_thinking_display_mode() {
        let (_, mock) = generate_text_fixture(
            opus_model,
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "adaptive",
                            "display": "updates",
                        },
                    },
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["thinking"],
            &json!({
                "type": "adaptive",
                "display": "updates",
            }),
        );
        let beta_header = request_header(&mock, 0, "anthropic-beta").unwrap_or_default();
        assert!(
            beta_header.contains("thinking-display-updates-2026-08-18"),
            "{beta_header}"
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should serialize thinking binding controls and add the beta header
    #[tokio::test]
    async fn should_serialize_thinking_binding_controls_and_add_the_beta_header() {
        let (_, mock) = generate_text_fixture(
            |mock| provider(mock, "claude-fable-5"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "blockBinding": {
                                "prefixMismatchBehavior": "drop_block",
                            },
                        },
                    },
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["thinking"],
            &json!({
                "block_binding": {
                    "prefix_mismatch_behavior": "drop_block",
                },
            }),
        );
        let beta_header = request_header(&mock, 0, "anthropic-beta").unwrap_or_default();
        assert!(
            beta_header.contains("thinking-binding-controls-2026-08-01"),
            "{beta_header}"
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-4-7 specific behavior › should serialize strict thinking binding controls with adaptive thinking
    #[tokio::test]
    async fn should_serialize_strict_thinking_binding_controls_with_adaptive_thinking() {
        let (_, mock) = generate_text_fixture(
            |mock| provider(mock, "claude-fable-5-1"),
            json!({
                "prompt": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "Hello",
                            },
                        ],
                    },
                ],
                "maxOutputTokens": 4096,
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "adaptive",
                            "blockBinding": {
                                "prefixMismatchBehavior": "error",
                            },
                        },
                    },
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0),
            &json!({
                "max_tokens": 4096,
                "messages": [
                    {
                        "content": [
                            {
                                "text": "Hello",
                                "type": "text",
                            },
                        ],
                        "role": "user",
                    },
                ],
                "model": "claude-fable-5-1",
                "thinking": {
                    "block_binding": {
                        "prefix_mismatch_behavior": "error",
                    },
                    "type": "adaptive",
                },
            }),
        );
        let beta_header = request_header(&mock, 0, "anthropic-beta").unwrap_or_default();
        assert!(
            beta_header.contains("thinking-binding-controls-2026-08-01"),
            "{beta_header}"
        );
    }
}

/// `TEST_TOOL` of the `claude-*-5-5 specific behavior` describes.
fn test_tool() -> Value {
    json!({
        "type": "function",
        "name": "testTool",
        "inputSchema": {
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"],
            "additionalProperties": false,
        },
    })
}

mod claude_opus_5_5_specific_behavior {
    use super::*;

    fn provider(mock: &Arc<MockFetch>, model_id: &str) -> AnthropicMessagesModel {
        test_provider(mock).messages(model_id)
    }

    fn opus_5_5(mock: &Arc<MockFetch>) -> AnthropicMessagesModel {
        provider(mock, "claude-opus-5-5")
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should return capabilities that always use adaptive thinking and reject forced tool use
    #[test]
    fn should_return_capabilities_that_always_use_adaptive_thinking_and_reject_forced_tool_use() {
        assert_equals(
            &model_capabilities("claude-opus-5-5"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": true,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": true,
                "rejectsThinkingDisabledAboveHighEffort": true,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": false,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should not send thinking by default
    #[tokio::test]
    async fn should_not_send_thinking_by_default() {
        let (result, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert!(request_body.get("thinking").is_none(), "{request_body:#}");
        assert_eq!(request_body["max_tokens"], 128000);
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should remove disabled thinking and warn
    #[tokio::test]
    async fn should_remove_disabled_thinking_and_warn() {
        let (result, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "disabled",
                        },
                        "effort": "low",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert!(request_body.get("thinking").is_none(), "{request_body:#}");
        assert_equals(
            &request_body["output_config"],
            &json!({
                "effort": "low",
            }),
        );
        assert_equals(
            &warnings_json(&result.warnings),
            &json!([
                {
                    "details": "thinking cannot be disabled for claude-opus-5-5; it always uses adaptive thinking. The thinking setting has been removed. Lower 'effort' to reduce thinking.",
                    "feature": "providerOptions.anthropic.thinking",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should not lower the effort when disabled thinking is removed at xhigh effort
    #[tokio::test]
    async fn should_not_lower_the_effort_when_disabled_thinking_is_removed_at_xhigh_effort() {
        let (result, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "disabled",
                        },
                        "effort": "xhigh",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert!(request_body.get("thinking").is_none(), "{request_body:#}");
        assert_equals(
            &request_body["output_config"],
            &json!({
                "effort": "xhigh",
            }),
        );
        assert_eq!(result.warnings.len(), 1);
        assert_match_object(
            &warnings_json(&result.warnings)[0],
            &json!({
                "feature": "providerOptions.anthropic.thinking",
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should convert budget-based thinking to adaptive thinking and warn
    #[tokio::test]
    async fn should_convert_budget_based_thinking_to_adaptive_thinking_and_warn() {
        let (result, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "enabled",
                            "budgetTokens": 5000,
                        },
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["thinking"],
            &json!({
                "type": "adaptive",
            }),
        );
        assert_eq!(request_body["max_tokens"], 128000);
        assert_equals(
            &warnings_json(&result.warnings),
            &json!([
                {
                    "details": "budget-based thinking is not supported by claude-opus-5-5; it always uses adaptive thinking. Using adaptive thinking instead. Use 'effort' to control how much the model thinks.",
                    "feature": "providerOptions.anthropic.thinking",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should keep adaptive thinking with display and block binding
    #[tokio::test]
    async fn should_keep_adaptive_thinking_with_display_and_block_binding() {
        let (_, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "adaptive",
                            "display": "updates",
                            "blockBinding": {
                                "prefixMismatchBehavior": "drop_block",
                            },
                        },
                    },
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["thinking"],
            &json!({
                "type": "adaptive",
                "display": "updates",
                "block_binding": {
                    "prefix_mismatch_behavior": "drop_block",
                },
            }),
        );
        let beta_header = request_header(&mock, 0, "anthropic-beta").unwrap_or_default();
        assert!(
            beta_header.contains("thinking-display-updates-2026-08-18"),
            "{beta_header}"
        );
        assert!(
            beta_header.contains("thinking-binding-controls-2026-08-01"),
            "{beta_header}"
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should map reasoning "none" to low effort instead of disabling thinking
    #[tokio::test]
    async fn should_map_reasoning_none_to_low_effort_instead_of_disabling_thinking() {
        let (result, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
                "reasoning": "none",
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert!(request_body.get("thinking").is_none(), "{request_body:#}");
        assert_equals(
            &request_body["output_config"],
            &json!({
                "effort": "low",
            }),
        );
        assert_equals(
            &warnings_json(&result.warnings),
            &json!([
                {
                    "details": "reasoning 'none' is not supported by claude-opus-5-5; it always uses adaptive thinking. Using effort 'low' to minimize thinking instead.",
                    "feature": "reasoning",
                    "type": "compatibility",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should map reasoning "xhigh" to adaptive thinking with effort "xhigh"
    #[tokio::test]
    async fn should_map_reasoning_xhigh_to_adaptive_thinking_with_effort_xhigh() {
        let (result, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
                "reasoning": "xhigh",
            }),
        )
        .await;
        assert_match_object(
            &request_body(&mock, 0),
            &json!({
                "thinking": {
                    "type": "adaptive",
                    "display": "summarized",
                },
                "output_config": {
                    "effort": "xhigh",
                },
            }),
        );
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should fall back to auto tool choice when tool choice is "required"
    #[tokio::test]
    async fn should_fall_back_to_auto_tool_choice_when_tool_choice_is_required() {
        let (result, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
                "tools": [
                    test_tool(),
                ],
                "toolChoice": {
                    "type": "required",
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["tool_choice"],
            &json!({
                "type": "auto",
            }),
        );
        assert_eq!(request_body["tools"].as_array().map(Vec::len), Some(1));
        assert_equals(
            &warnings_json(&result.warnings),
            &json!([
                {
                    "details": "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.",
                    "feature": "toolChoice",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should only send the selected tool with auto tool choice when tool choice is "tool"
    #[tokio::test]
    async fn should_only_send_the_selected_tool_with_auto_tool_choice_when_tool_choice_is_tool() {
        let mut other_tool = test_tool();
        other_tool["name"] = json!("otherTool");
        let (result, mock) = generate_text_fixture(
            opus_5_5,
            json!({
                "prompt": test_prompt(),
                "tools": [test_tool(), other_tool],
                "toolChoice": { "type": "tool", "toolName": "otherTool" },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["tool_choice"],
            &json!({
                "type": "auto",
            }),
        );
        let names: Vec<_> = request_body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].clone())
            .collect();
        assert_eq!(json!(names), json!(["otherTool",]));
        assert_equals(
            &warnings_json(&result.warnings),
            &json!([
                {
                    "details": "toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the 'otherTool' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made.",
                    "feature": "toolChoice",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should use native structured outputs when jsonTool mode is requested
    #[tokio::test]
    async fn should_use_native_structured_outputs_when_json_tool_mode_is_requested() {
        let mock = server(vec![json_fixture(include_str!(
            "fixtures/anthropic-json-output-format.1.json"
        ))]);
        let result = generate(
            &opus_5_5(&mock),
            json!({
                "prompt": test_prompt(),
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                            },
                        },
                        "required": [
                            "name",
                        ],
                        "additionalProperties": false,
                    },
                },
                "providerOptions": {
                    CANONICAL: {
                        "structuredOutputMode": "jsonTool",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["output_config"]["format"],
            &json!({
                "type": "json_schema",
                "schema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                        },
                    },
                    "required": [
                        "name",
                    ],
                    "additionalProperties": false,
                },
            }),
        );
        assert!(request_body.get("tools").is_none(), "{request_body:#}");
        assert!(
            request_body.get("tool_choice").is_none(),
            "{request_body:#}"
        );
        assert_equals(
            &warnings_json(&result.warnings),
            &json!([
                {
                    "details": "structuredOutputMode 'jsonTool' is not supported by claude-opus-5-5 because it rejects forced tool use. Using 'outputFormat' instead.",
                    "feature": "providerOptions.anthropic.structuredOutputMode",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should use auto tool choice for claude-fable-5-1 when tool choice is "required"
    #[tokio::test]
    async fn should_use_auto_tool_choice_for_claude_fable_5_1_when_tool_choice_is_required() {
        let (result, mock) = generate_text_fixture(
            |mock| provider(mock, "claude-fable-5-1"),
            json!({
                "prompt": test_prompt(),
                "tools": [
                    test_tool(),
                ],
                "toolChoice": {
                    "type": "required",
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["tool_choice"],
            &json!({
                "type": "auto",
            }),
        );
        assert_eq!(result.warnings.len(), 1);
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-opus-5-5 specific behavior › should still send forced tool choice for claude-opus-5
    #[tokio::test]
    async fn should_still_send_forced_tool_choice_for_claude_opus_5() {
        let (result, mock) = generate_text_fixture(
            |mock| provider(mock, "claude-opus-5"),
            json!({
                "prompt": test_prompt(),
                "tools": [
                    test_tool(),
                ],
                "toolChoice": {
                    "type": "required",
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["tool_choice"],
            &json!({
                "type": "any",
            }),
        );
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }
}

mod claude_sonnet_5_5_specific_behavior {
    use super::*;

    fn provider(mock: &Arc<MockFetch>, model_id: &str) -> AnthropicMessagesModel {
        test_provider(mock).messages(model_id)
    }

    fn sonnet_5_5(mock: &Arc<MockFetch>) -> AnthropicMessagesModel {
        provider(mock, "claude-sonnet-5-5")
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should return capabilities that reject disabled thinking and forced tool use
    #[test]
    fn should_return_capabilities_that_reject_disabled_thinking_and_forced_tool_use() {
        assert_equals(
            &model_capabilities("claude-sonnet-5-5"),
            &json!({
                "isKnownModel": true,
                "maxOutputTokens": 128000,
                "rejectsForcedToolUse": true,
                "rejectsSamplingParameters": true,
                "rejectsThinkingDisabled": true,
                "rejectsThinkingDisabledAboveHighEffort": true,
                "supportsAdaptiveThinking": true,
                "supportsBetweenToolsThinking": true,
                "supportsStructuredOutput": true,
                "supportsXhighEffort": true,
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should not send thinking by default
    #[tokio::test]
    async fn should_not_send_thinking_by_default() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert!(request_body.get("thinking").is_none(), "{request_body:#}");
        assert_eq!(request_body["max_tokens"], 128000);
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should send between_tools thinking
    #[tokio::test]
    async fn should_send_between_tools_thinking() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "between_tools",
                        },
                        "effort": "medium",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["thinking"],
            &json!({
                "type": "between_tools",
            }),
        );
        assert_equals(
            &request_body["output_config"],
            &json!({
                "effort": "medium",
            }),
        );
        assert_eq!(request_body["max_tokens"], 128000);
        assert_eq!(request_header(&mock, 0, "anthropic-beta"), None);
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should replace disabled thinking with between_tools thinking and warn
    #[tokio::test]
    async fn should_replace_disabled_thinking_with_between_tools_thinking_and_warn() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "disabled",
                        },
                        "effort": "low",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["thinking"],
            &json!({
                "type": "between_tools",
            }),
        );
        assert_equals(
            &request_body["output_config"],
            &json!({
                "effort": "low",
            }),
        );
        assert_equals(
            &warnings_json(&result.warnings),
            &json!([
                {
                    "details": "thinking cannot be disabled for claude-sonnet-5-5. Using 'between_tools' thinking, the lowest thinking setting, instead.",
                    "feature": "providerOptions.anthropic.thinking",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should lower xhigh effort to high with between_tools thinking and warn
    #[tokio::test]
    async fn should_lower_xhigh_effort_to_high_with_between_tools_thinking_and_warn() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "between_tools",
                        },
                        "effort": "xhigh",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["thinking"],
            &json!({
                "type": "between_tools",
            }),
        );
        assert_equals(
            &request_body["output_config"],
            &json!({
                "effort": "high",
            }),
        );
        assert_equals(
            &warnings_json(&result.warnings),
            &json!([
                {
                    "details": "effort 'xhigh' is not supported with 'between_tools' thinking. The effort has been lowered to 'high'.",
                    "feature": "providerOptions.anthropic.effort",
                    "type": "unsupported",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should lower max effort to high when disabled thinking is replaced
    #[tokio::test]
    async fn should_lower_max_effort_to_high_when_disabled_thinking_is_replaced() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "disabled",
                        },
                        "effort": "max",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["thinking"],
            &json!({
                "type": "between_tools",
            }),
        );
        assert_equals(
            &request_body["output_config"],
            &json!({
                "effort": "high",
            }),
        );
        let features: Vec<Value> = warnings_json(&result.warnings)
            .as_array()
            .unwrap()
            .iter()
            .map(|warning| warning["feature"].clone())
            .collect();
        assert_equals(
            &json!(features),
            &json!([
                "providerOptions.anthropic.thinking",
                "providerOptions.anthropic.effort",
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should convert budget-based thinking to adaptive thinking and warn
    #[tokio::test]
    async fn should_convert_budget_based_thinking_to_adaptive_thinking_and_warn() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "enabled",
                            "budgetTokens": 5000,
                        },
                    },
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["thinking"],
            &json!({
                "type": "adaptive",
            }),
        );
        assert_eq!(result.warnings.len(), 1);
        assert_match_object(
            &warnings_json(&result.warnings)[0],
            &json!({
                "feature": "providerOptions.anthropic.thinking",
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should map reasoning "none" to between_tools thinking
    #[tokio::test]
    async fn should_map_reasoning_none_to_between_tools_thinking() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
                "reasoning": "none",
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_equals(
            &request_body["thinking"],
            &json!({
                "type": "between_tools",
            }),
        );
        assert!(
            request_body.get("output_config").is_none(),
            "{request_body:#}"
        );
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should map reasoning "xhigh" to adaptive thinking with effort "xhigh"
    #[tokio::test]
    async fn should_map_reasoning_xhigh_to_adaptive_thinking_with_effort_xhigh() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
                "reasoning": "xhigh",
            }),
        )
        .await;
        assert_match_object(
            &request_body(&mock, 0),
            &json!({
                "thinking": {
                    "type": "adaptive",
                    "display": "summarized",
                },
                "output_config": {
                    "effort": "xhigh",
                },
            }),
        );
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should fall back to auto tool choice when tool choice is "required"
    #[tokio::test]
    async fn should_fall_back_to_auto_tool_choice_when_tool_choice_is_required() {
        let (result, mock) = generate_text_fixture(
            sonnet_5_5,
            json!({
                "prompt": test_prompt(),
                "tools": [
                    test_tool(),
                ],
                "toolChoice": {
                    "type": "required",
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["tool_choice"],
            &json!({
                "type": "auto",
            }),
        );
        assert_eq!(result.warnings.len(), 1);
        assert_match_object(
            &warnings_json(&result.warnings)[0],
            &json!({
                "feature": "toolChoice",
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should use native structured outputs when jsonTool mode is requested
    #[tokio::test]
    async fn should_use_native_structured_outputs_when_json_tool_mode_is_requested() {
        let mock = server(vec![json_fixture(include_str!(
            "fixtures/anthropic-json-output-format.1.json"
        ))]);
        let result = generate(
            &sonnet_5_5(&mock),
            json!({
                "prompt": test_prompt(),
                "responseFormat": {
                    "type": "json",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                            },
                        },
                        "required": [
                            "name",
                        ],
                        "additionalProperties": false,
                    },
                },
                "providerOptions": {
                    CANONICAL: {
                        "structuredOutputMode": "jsonTool",
                    },
                },
            }),
        )
        .await;
        let request_body = request_body(&mock, 0);
        assert_match_object(
            &request_body["output_config"]["format"],
            &json!({
                "type": "json_schema",
            }),
        );
        assert!(request_body.get("tools").is_none(), "{request_body:#}");
        assert_eq!(result.warnings.len(), 1);
        assert_match_object(
            &warnings_json(&result.warnings)[0],
            &json!({
                "feature": "providerOptions.anthropic.structuredOutputMode",
            }),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › claude-sonnet-5-5 specific behavior › should keep disabled thinking for claude-sonnet-5
    #[tokio::test]
    async fn should_keep_disabled_thinking_for_claude_sonnet_5() {
        let (result, mock) = generate_text_fixture(
            |mock| provider(mock, "claude-sonnet-5"),
            json!({
                "prompt": test_prompt(),
                "providerOptions": {
                    CANONICAL: {
                        "thinking": {
                            "type": "disabled",
                        },
                    },
                },
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["thinking"],
            &json!({
                "type": "disabled",
            }),
        );
        assert_eq!(warnings_json(&result.warnings), json!([]));
    }
}

/// `computerToolset` of the `computer toolset` describe.
fn computer_toolset() -> Value {
    json!({
        "type": "provider",
        "id": "anthropic.computer_toolset_20260801",
        "name": "computer",
        "args": {},
    })
}

mod computer_toolset {
    use super::*;

    fn opus_5_5(mock: &Arc<MockFetch>) -> AnthropicMessagesModel {
        test_provider(mock).messages("claude-opus-5-5")
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › computer toolset › should send the toolset definition without a name and map member calls to the computer tool
    #[tokio::test]
    async fn should_send_the_toolset_definition_without_a_name_and_map_member_calls_to_the_computer_tool()
     {
        let mock = server(vec![json_response(json!({
            "id": "msg_toolset",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "Clicking the search box.",
                },
                {
                    "type": "tool_use",
                    "id": "toolu_click",
                    "name": "left_click",
                    "toolset_name": "computer",
                    "input": {
                        "coordinate": [
                            640,
                            60,
                        ],
                    },
                },
                {
                    "type": "tool_use",
                    "id": "toolu_type",
                    "name": "type",
                    "toolset_name": "computer",
                    "input": {
                        "text": "pictures of cats",
                    },
                },
                {
                    "type": "tool_use",
                    "id": "toolu_screenshot",
                    "name": "screenshot",
                    "toolset_name": "computer",
                    "input": {},
                },
            ],
            "model": "claude-opus-5-5",
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 4,
                "output_tokens": 30,
            },
        }))]);
        let result = generate(
            &opus_5_5(&mock),
            json!({
                "prompt": test_prompt(),
                "tools": [
                    computer_toolset(),
                ],
            }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["tools"],
            &json!([
                {
                    "type": "computer_toolset_20260801",
                },
            ]),
        );
        assert_eq!(request_header(&mock, 0, "anthropic-beta"), None);
        assert_equals(
            &content_json(&result),
            &json!([
                {
                    "text": "Clicking the search box.",
                    "type": "text",
                },
                {
                    "input": r#"{"action":"left_click","coordinate":[640,60]}"#,
                    "providerMetadata": {
                        CANONICAL: {
                            "toolsetName": "computer",
                        },
                    },
                    "toolCallId": "toolu_click",
                    "toolName": "computer",
                    "type": "tool-call",
                },
                {
                    "input": r#"{"action":"type","text":"pictures of cats"}"#,
                    "providerMetadata": {
                        CANONICAL: {
                            "toolsetName": "computer",
                        },
                    },
                    "toolCallId": "toolu_type",
                    "toolName": "computer",
                    "type": "tool-call",
                },
                {
                    "input": r#"{"action":"screenshot"}"#,
                    "providerMetadata": {
                        CANONICAL: {
                            "toolsetName": "computer",
                        },
                    },
                    "toolCallId": "toolu_screenshot",
                    "toolName": "computer",
                    "type": "tool-call",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › computer toolset › should map member calls to a custom tool name
    #[tokio::test]
    async fn should_map_member_calls_to_a_custom_tool_name() {
        let mock = server(vec![json_response(json!({
            "id": "msg_toolset",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_screenshot",
                    "name": "screenshot",
                    "toolset_name": "computer",
                    "input": {},
                },
            ],
            "model": "claude-opus-5-5",
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 4,
                "output_tokens": 30,
            },
        }))]);
        let mut tool = computer_toolset();
        tool["name"] = json!("desktop");
        let result = generate(
            &opus_5_5(&mock),
            json!({ "prompt": test_prompt(), "tools": [tool] }),
        )
        .await;
        assert_match_object(
            &content_json(&result),
            &json!([
                {
                    "type": "tool-call",
                    "toolCallId": "toolu_screenshot",
                    "toolName": "desktop",
                    "input": r#"{"action":"screenshot"}"#,
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › computer toolset › should send member configs
    #[tokio::test]
    async fn should_send_member_configs() {
        let mock = server(vec![json_response(json!({
            "id": "msg_toolset",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "ok",
                },
            ],
            "model": "claude-opus-5-5",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 4,
                "output_tokens": 30,
            },
        }))]);
        let mut tool = computer_toolset();
        tool["args"] = json!({
            "configs": {
                "zoom": {
                    "enabled": false,
                },
                "hold_key": {
                    "deferLoading": true,
                },
            },
        });
        generate(
            &opus_5_5(&mock),
            json!({ "prompt": test_prompt(), "tools": [tool] }),
        )
        .await;
        assert_equals(
            &request_body(&mock, 0)["tools"],
            &json!([
                {
                    "type": "computer_toolset_20260801",
                    "configs": {
                        "zoom": {
                            "enabled": false,
                        },
                        "hold_key": {
                            "defer_loading": true,
                        },
                    },
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › computer toolset › should round-trip member calls and results with toolset_name
    #[tokio::test]
    async fn should_round_trip_member_calls_and_results_with_toolset_name() {
        let mock = server(vec![json_response(json!({
            "id": "msg_toolset",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "Done.",
                },
            ],
            "model": "claude-opus-5-5",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 4,
                "output_tokens": 30,
            },
        }))]);
        generate(&opus_5_5(&mock), json!({
            "tools": [
                computer_toolset(),
            ],
            "prompt": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Open the search.",
                        },
                    ],
                },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "toolu_click",
                            "toolName": "computer",
                            "input": {
                                "action": "left_click",
                                "coordinate": [
                                    640,
                                    60,
                                ],
                            },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "toolu_screenshot",
                            "toolName": "computer",
                            "input": {
                                "action": "screenshot",
                            },
                        },
                    ],
                },
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "toolu_click",
                            "toolName": "computer",
                            "output": {
                                "type": "text",
                                "value": "OK",
                            },
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "toolu_screenshot",
                            "toolName": "computer",
                            "output": {
                                "type": "error-text",
                                "value": "Not executed: an earlier computer action in this turn failed.",
                            },
                        },
                    ],
                },
            ],
        })).await;
        assert_equals(
            &request_body(&mock, 0)["messages"],
            &json!([
                {
                    "content": [
                        {
                            "text": "Open the search.",
                            "type": "text",
                        },
                    ],
                    "role": "user",
                },
                {
                    "content": [
                        {
                            "id": "toolu_click",
                            "input": {
                                "action": "left_click",
                                "coordinate": [
                                    640,
                                    60,
                                ],
                            },
                            "name": "left_click",
                            "toolset_name": "computer",
                            "type": "tool_use",
                        },
                        {
                            "id": "toolu_screenshot",
                            "input": {
                                "action": "screenshot",
                            },
                            "name": "screenshot",
                            "toolset_name": "computer",
                            "type": "tool_use",
                        },
                    ],
                    "role": "assistant",
                },
                {
                    "content": [
                        {
                            "content": "OK",
                            "tool_use_id": "toolu_click",
                            "toolset_name": "computer",
                            "type": "tool_result",
                        },
                        {
                            "content": "Not executed: an earlier computer action in this turn failed.",
                            "is_error": true,
                            "tool_use_id": "toolu_screenshot",
                            "toolset_name": "computer",
                            "type": "tool_result",
                        },
                    ],
                    "role": "user",
                },
            ]),
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › computer toolset › should stream member calls as a single input delta with the action injected
    #[tokio::test]
    async fn should_stream_member_calls_as_a_single_input_delta_with_the_action_injected() {
        let mock = server(vec![sse_data(&[
            r#"{"type":"message_start","message":{"id":"msg_toolset","type":"message","role":"assistant","model":"claude-opus-5-5","stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":1},"content":[],"stop_reason":null}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_click","name":"left_click","toolset_name":"computer","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"coordinate\""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":":[640,60]}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_screenshot","name":"screenshot","toolset_name":"computer","input":{}}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":20}}"#,
            r#"{"type":"message_stop"}"#,
        ])]);
        let parts = stream_parts(
            &opus_5_5(&mock),
            json!({
                "prompt": test_prompt(),
                "tools": [
                    computer_toolset(),
                ],
            }),
        )
        .await;
        let filtered: Vec<_> = parts
            .into_iter()
            .filter(|part| {
                [
                    "tool-input-start",
                    "tool-input-delta",
                    "tool-input-end",
                    "tool-call",
                ]
                .contains(&part["type"].as_str().unwrap_or_default())
            })
            .collect();
        assert_equals(
            &json!(filtered),
            &json!([
                {
                    "id": "toolu_click",
                    "toolName": "computer",
                    "type": "tool-input-start",
                },
                {
                    "delta": r#"{"action":"left_click","coordinate":[640,60]}"#,
                    "id": "toolu_click",
                    "type": "tool-input-delta",
                },
                {
                    "id": "toolu_click",
                    "type": "tool-input-end",
                },
                {
                    "input": r#"{"action":"left_click","coordinate":[640,60]}"#,
                    "providerMetadata": {
                        CANONICAL: {
                            "toolsetName": "computer",
                        },
                    },
                    "toolCallId": "toolu_click",
                    "toolName": "computer",
                    "type": "tool-call",
                },
                {
                    "id": "toolu_screenshot",
                    "toolName": "computer",
                    "type": "tool-input-start",
                },
                {
                    "delta": r#"{"action":"screenshot"}"#,
                    "id": "toolu_screenshot",
                    "type": "tool-input-delta",
                },
                {
                    "id": "toolu_screenshot",
                    "type": "tool-input-end",
                },
                {
                    "input": r#"{"action":"screenshot"}"#,
                    "providerMetadata": {
                        CANONICAL: {
                            "toolsetName": "computer",
                        },
                    },
                    "toolCallId": "toolu_screenshot",
                    "toolName": "computer",
                    "type": "tool-call",
                },
            ]),
        );
    }
}

mod mid_conversation_tool_changes {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › mid-conversation tool changes › should send separate effort updates and turn-scoped reminders with their beta headers
    #[tokio::test]
    async fn should_send_separate_effort_updates_and_turn_scoped_reminders_with_beta_headers() {
        let server = MockFetch::new(vec![Canned::json(
            &serde_json::from_str::<Value>(ANTHROPIC_TEXT).unwrap(),
        )]);
        let provider = create_anthropic(AnthropicProviderSettings {
            api_key: Some("test-api-key".into()),
            fetch: Some(server.transport()),
            ..Default::default()
        })
        .unwrap();

        let prompt: LanguageModelPrompt = serde_json::from_value(json!([
            { "role": "user", "content": [{ "type": "text", "text": "Draft an answer." }] },
            { "role": "assistant", "content": [{ "type": "text", "text": "Draft." }] },
            {
                "role": "system",
                "content": "",
                "providerOptions": { CANONICAL: { "effort": "xhigh" } },
            },
            { "role": "user", "content": [{ "type": "text", "text": "Now finalize it." }] },
            {
                "role": "system",
                "content": "Verify every claim in this answer.",
                "providerOptions": { CANONICAL: { "clearAt": "next_user_message" } },
            },
        ]))
        .unwrap();
        provider
            .call("claude-fable-5-1")
            .do_generate(&CallOptions::new(prompt))
            .await
            .unwrap();

        let calls = server.seen();
        let request_body = calls[0].json_body();
        assert_eq!(
            request_body["messages"],
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "Draft an answer." }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "Draft." }] },
                { "role": "system", "content": [], "output_config": { "effort": "xhigh" } },
                { "role": "user", "content": [{ "type": "text", "text": "Now finalize it." }] },
                {
                    "role": "system",
                    "content": [{ "type": "text", "text": "Verify every claim in this answer." }],
                    "clear_at": "next_user_message",
                },
            ])
        );
        let beta = &calls[0].headers["anthropic-beta"];
        assert!(
            beta.contains("mid-conversation-system-clear-at-2026-08-21"),
            "{beta}"
        );
        assert!(
            beta.contains("mid-conversation-output-config-2026-07-01"),
            "{beta}"
        );
    }
}
