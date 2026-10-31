// Shared helpers of the non-streaming (`doGenerate`, mixed citation types,
// programmatic tool calling) cases. Streaming helpers are prefixed
// `stream_` / `sse_` and live with the `do_stream` mod.

use std::sync::Arc;

use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateResult;

use crate::openai::config::OpenAIModelConfig;
use crate::openai::responses::{OpenAIResponsesModel, ResponsesNamespace};
use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::shared::Credential;
use crate::shared::ProviderHeaders;
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_core::result::StreamResult;
use aimux_provider_utils::Resolvable;

const GENERATE_BASE_URL: &str = "https://api.openai.com/v1";
const GENERATE_API_KEY: &str = "APIKEY";
const AZURE_RESPONSES: ResponsesNamespace = ResponsesNamespace::new(&["azure", "openai"], "azure");

/// An upstream `__fixtures__/<name>.json` response body.
macro_rules! json_fixture {
    ($name:literal) => {
        serde_json::from_str::<serde_json::Value>(include_str!(concat!(
            "fixtures/",
            $name,
            ".json"
        )))
        .expect("fixture is JSON")
    };
}

/// An upstream `__fixtures__/<name>.chunks.txt` event stream, one event per
/// line, as upstream's `prepareChunksFixtureResponse` sends it.
macro_rules! json_chunks_fixture {
    ($name:literal) => {
        json_chunks_canned(include_str!(concat!("fixtures/", $name, ".chunks.txt")))
    };
}

fn json_chunks_canned(text: &str) -> Canned {
    let payloads: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    json_event_stream(&payloads)
}

/// `data: <payload>\n\n` for every payload, then `data: [DONE]\n\n`.
fn json_event_stream(payloads: &[&str]) -> Canned {
    let mut body = String::new();
    for payload in payloads {
        body.push_str("data: ");
        body.push_str(payload);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }
}

/// Upstream's `createModel` provider: the OpenAI API URL and an explicit key.
fn generate_provider(mock: &Arc<MockFetch>) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some(GENERATE_API_KEY.into()),
        base_url: Some(GENERATE_BASE_URL.into()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("provider")
}

/// Upstream's call options object (AI SDK JSON shape).
fn generate_call_options(options: Value) -> CallOptions {
    serde_json::from_value(options).expect("call options deserialize")
}

/// Upstream's `mockId()` numbers the source ids `id-0`, `id-1`, …; aimux
/// numbers them with a process-wide counter (`source-<n>`) that a test cannot
/// reset, so the generated ids are renumbered in order before comparing.
fn json_normalize_source_ids(parts: &mut Value) {
    let mut next = 0;
    if let Some(items) = parts.as_array_mut() {
        for item in items {
            if item["type"] == "source"
                && item["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("source-"))
            {
                item["id"] = json!(format!("id-{next}"));
                next += 1;
            }
        }
    }
}

/// Upstream's `mockId()` gives a provider-generated tool call id `id-0`;
/// aimux generates a random 16-character id (`generate_id`, no injection
/// seam). The id is checked to be one such id, shared by every part that
/// names it, and then replaced with `id-0` before comparing.
fn json_normalize_generated_tool_call_id(parts: &mut Value) {
    let items = parts.as_array_mut().expect("parts");
    let generated = items
        .iter()
        .find(|part| part["type"] == "tool-call")
        .and_then(|part| part["toolCallId"].as_str())
        .expect("tool call")
        .to_owned();
    assert!(
        generated.len() == 16 && generated.chars().all(|c| c.is_ascii_alphanumeric()),
        "not a generated id: {generated}"
    );
    for item in items {
        if item["toolCallId"] == generated.as_str() {
            item["toolCallId"] = json!("id-0");
        }
    }
}

/// Jest's `toMatchObject`: objects match on the expected keys, arrays match
/// element by element with the same length, everything else is equal.
fn json_match_object(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected.iter().all(|(key, value)| {
            actual
                .get(key)
                .is_some_and(|actual| json_match_object(actual, value))
        }),
        (Value::Array(actual), Value::Array(expected)) => {
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| json_match_object(actual, expected))
        }
        _ => actual == expected,
    }
}

#[track_caller]
fn assert_json_match_object(actual: &Value, expected: &Value) {
    assert!(
        json_match_object(actual, expected),
        "toMatchObject failed\nactual: {actual:#}\nexpected: {expected:#}"
    );
}

/// Jest's `expect.objectContaining`: the expected keys are present and equal.
fn json_object_containing(actual: &Value, expected: &Value) -> bool {
    expected
        .as_object()
        .expect("expected object")
        .iter()
        .all(|(key, value)| actual.get(key) == Some(value))
}

#[track_caller]
fn assert_json_eq(actual: &Value, expected: &Value) {
    assert!(
        actual == expected,
        "values differ\nactual: {actual:#}\nexpected: {expected:#}"
    );
}

/// One `doGenerate` call and what it sent.
struct Generated {
    json: Value,
    body: Value,
}

impl Generated {
    /// `result.content`, source ids renumbered.
    fn content(&self) -> Value {
        let mut content = self.json["content"].clone();
        json_normalize_source_ids(&mut content);
        content
    }

    fn warnings(&self) -> Value {
        self.json["warnings"].clone()
    }

    fn finish_reason(&self) -> Value {
        self.json["finishReason"].clone()
    }

    fn usage(&self) -> Value {
        self.json["usage"].clone()
    }

    fn provider_metadata(&self) -> Value {
        self.json["providerMetadata"].clone()
    }
}

async fn generate_canned(
    model_id: &str,
    response: Canned,
    options: Value,
) -> (Result<GenerateResult, AiMuxError>, Arc<MockFetch>) {
    let mock = MockFetch::new(vec![response]);
    let model = generate_provider(&mock).responses(model_id);
    let result = model.do_generate(&generate_call_options(options)).await;
    (result, mock)
}

/// `createModel(modelId).doGenerate(options)` against a JSON response body.
async fn generate(model_id: &str, response: Value, options: Value) -> Generated {
    let (result, mock) = generate_canned(model_id, Canned::json(&response), options).await;
    let result = result.expect("doGenerate succeeds");
    let json = serde_json::to_value(&result).expect("result serializes");
    let body = mock.seen()[0].json_body();
    Generated { json, body }
}

/// A `doGenerate` call that is expected to fail.
async fn generate_error(model_id: &str, response: Value, options: Value) -> AiMuxError {
    let (result, _mock) = generate_canned(model_id, Canned::json(&response), options).await;
    match result {
        Ok(result) => panic!("doGenerate succeeded: {result:?}"),
        Err(error) => error,
    }
}

/// One `doStream` call, its parts (as JSON, source ids renumbered) and what it
/// sent.
struct Collected {
    parts: Value,
    body: Value,
}

async fn collect_parts(model_id: &str, response: Canned, options: Value) -> Collected {
    let mock = MockFetch::new(vec![response]);
    let model = generate_provider(&mock).responses(model_id);
    let result = model
        .do_stream(&generate_call_options(options))
        .await
        .expect("doStream succeeds");
    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(part) = stream.next().await {
        parts.push(match part {
            Ok(part) => serde_json::to_value(&part).expect("part serializes"),
            Err(error) => json!({ "type": "error", "error": error.to_string() }),
        });
    }
    let mut parts = Value::Array(parts);
    json_normalize_source_ids(&mut parts);
    Collected {
        parts,
        body: mock.seen()[0].json_body(),
    }
}

fn test_prompt() -> Value {
    json!([
      {
        "role": "user",
        "content": [
          {
            "type": "text",
            "text": "Hello"
          }
        ]
      }
    ])
}

fn test_tools() -> Value {
    json!([
      {
        "type": "function",
        "name": "weather",
        "inputSchema": {
          "type": "object",
          "properties": {
            "location": {
              "type": "string"
            }
          },
          "required": [
            "location"
          ],
          "additionalProperties": false
        }
      },
      {
        "type": "function",
        "name": "cityAttractions",
        "inputSchema": {
          "type": "object",
          "properties": {
            "city": {
              "type": "string"
            }
          },
          "required": [
            "city"
          ],
          "additionalProperties": false
        }
      }
    ])
}

fn hosted_tool_search_tools() -> Value {
    json!([
      {
        "type": "provider",
        "id": "openai.tool_search",
        "name": "toolSearch",
        "args": {}
      },
      {
        "type": "function",
        "name": "get_weather",
        "description": "Get the current weather at a specific location",
        "inputSchema": {
          "type": "object",
          "properties": {
            "location": {
              "type": "string"
            },
            "unit": {
              "type": "string",
              "enum": [
                "celsius",
                "fahrenheit"
              ]
            }
          },
          "required": [
            "location",
            "unit"
          ],
          "additionalProperties": false
        },
        "strict": true,
        "providerOptions": {
          "openai": {
            "deferLoading": true
          }
        }
      },
      {
        "type": "function",
        "name": "search_files",
        "description": "Search through files in the workspace",
        "inputSchema": {
          "type": "object",
          "properties": {
            "query": {
              "type": "string"
            },
            "file_types": {
              "type": "array",
              "items": {
                "type": "string"
              }
            }
          },
          "required": [
            "query",
            "file_types"
          ],
          "additionalProperties": false
        },
        "strict": true,
        "providerOptions": {
          "openai": {
            "deferLoading": true
          }
        }
      },
      {
        "type": "function",
        "name": "send_email",
        "description": "Send an email to a recipient",
        "inputSchema": {
          "type": "object",
          "properties": {
            "to": {
              "type": "string"
            },
            "subject": {
              "type": "string"
            },
            "body": {
              "type": "string"
            }
          },
          "required": [
            "to",
            "subject",
            "body"
          ],
          "additionalProperties": false
        },
        "strict": true,
        "providerOptions": {
          "openai": {
            "deferLoading": true
          }
        }
      }
    ])
}

fn client_tool_search_tools() -> Value {
    json!([
      {
        "type": "provider",
        "id": "openai.tool_search",
        "name": "toolSearch",
        "args": {
          "execution": "client",
          "description": "Search for available tools based on what the user needs.",
          "parameters": {
            "type": "object",
            "properties": {
              "goal": {
                "type": "string",
                "description": "What the user is trying to accomplish"
              }
            },
            "required": [
              "goal"
            ],
            "additionalProperties": false
          }
        }
      },
      {
        "type": "function",
        "name": "get_weather",
        "description": "Get the current weather at a specific location",
        "inputSchema": {
          "type": "object",
          "properties": {
            "location": {
              "type": "string"
            },
            "unit": {
              "type": "string",
              "enum": [
                "celsius",
                "fahrenheit"
              ]
            }
          },
          "required": [
            "location",
            "unit"
          ],
          "additionalProperties": false
        },
        "strict": true,
        "providerOptions": {
          "openai": {
            "deferLoading": true
          }
        }
      },
      {
        "type": "function",
        "name": "search_files",
        "description": "Search through files in the workspace",
        "inputSchema": {
          "type": "object",
          "properties": {
            "query": {
              "type": "string"
            },
            "file_types": {
              "type": "array",
              "items": {
                "type": "string"
              }
            }
          },
          "required": [
            "query",
            "file_types"
          ],
          "additionalProperties": false
        },
        "strict": true,
        "providerOptions": {
          "openai": {
            "deferLoading": true
          }
        }
      }
    ])
}

/// `openaiResponsesReasoningModelIds` (4.0.83).
const REASONING_MODEL_IDS: &[&str] = &[
    "o1",
    "o1-2024-12-17",
    "o3",
    "o3-2025-04-16",
    "o3-mini",
    "o3-mini-2025-01-31",
    "o4-mini",
    "o4-mini-2025-04-16",
    "gpt-5",
    "gpt-5-2025-08-07",
    "gpt-5-codex",
    "gpt-5-mini",
    "gpt-5-mini-2025-08-07",
    "gpt-5-nano",
    "gpt-5-nano-2025-08-07",
    "gpt-5-pro",
    "gpt-5-pro-2025-10-06",
    "gpt-5.1",
    "gpt-5.1-chat-latest",
    "gpt-5.1-codex-mini",
    "gpt-5.1-codex",
    "gpt-5.1-codex-max",
    "gpt-5.2",
    "gpt-5.2-chat-latest",
    "gpt-5.2-pro",
    "gpt-5.2-codex",
    "gpt-5.3-chat-latest",
    "gpt-5.3-codex",
    "gpt-5.4",
    "gpt-5.4-2026-03-05",
    "gpt-5.4-mini",
    "gpt-5.4-mini-2026-03-17",
    "gpt-5.4-nano",
    "gpt-5.4-nano-2026-03-17",
    "gpt-5.4-pro",
    "gpt-5.4-pro-2026-03-05",
    "gpt-5.5",
    "gpt-5.5-2026-04-23",
    "gpt-5.6",
    "gpt-5.6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-6-astra",
    "gpt-6.1-sol",
    "gpt-6-luna",
    "gpt-6-sol",
];

/// `openaiResponsesModelIds` minus the reasoning model ids (4.0.83).
const NON_REASONING_MODEL_IDS: &[&str] = &[
    "gpt-4.1",
    "gpt-4.1-2025-04-14",
    "gpt-4.1-mini",
    "gpt-4.1-mini-2025-04-14",
    "gpt-4.1-nano",
    "gpt-4.1-nano-2025-04-14",
    "gpt-4o",
    "gpt-4o-2024-05-13",
    "gpt-4o-2024-08-06",
    "gpt-4o-2024-11-20",
    "gpt-4o-audio-preview",
    "gpt-4o-audio-preview-2024-12-17",
    "gpt-4o-search-preview",
    "gpt-4o-search-preview-2025-03-11",
    "gpt-4o-mini-search-preview",
    "gpt-4o-mini-search-preview-2025-03-11",
    "gpt-4o-mini",
    "gpt-4o-mini-2024-07-18",
    "gpt-3.5-turbo-0125",
    "gpt-3.5-turbo",
    "gpt-3.5-turbo-1106",
    "gpt-5-chat-latest",
];

/// Upstream's `TEST_PROMPT`.
fn stream_test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

/// Upstream's `TEST_TOOLS`.
fn stream_test_tools() -> Value {
    json!([
        {"type": "function", "name": "weather", "inputSchema": {"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"], "additionalProperties": false}},
        {"type": "function", "name": "cityAttractions", "inputSchema": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"], "additionalProperties": false}},
    ])
}

/// Upstream's `HOSTED_TOOL_SEARCH_TOOLS`.
fn stream_hosted_tool_search_tools() -> Value {
    json!([
        {"type": "provider", "id": "openai.tool_search", "name": "toolSearch", "args": {}},
        {"type": "function", "name": "get_weather", "description": "Get the current weather at a specific location", "inputSchema": {"type": "object", "properties": {"location": {"type": "string"}, "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]}}, "required": ["location", "unit"], "additionalProperties": false}, "strict": true, "providerOptions": {"openai": {"deferLoading": true}}},
        {"type": "function", "name": "search_files", "description": "Search through files in the workspace", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "file_types": {"type": "array", "items": {"type": "string"}}}, "required": ["query", "file_types"], "additionalProperties": false}, "strict": true, "providerOptions": {"openai": {"deferLoading": true}}},
        {"type": "function", "name": "send_email", "description": "Send an email to a recipient", "inputSchema": {"type": "object", "properties": {"to": {"type": "string"}, "subject": {"type": "string"}, "body": {"type": "string"}}, "required": ["to", "subject", "body"], "additionalProperties": false}, "strict": true, "providerOptions": {"openai": {"deferLoading": true}}},
    ])
}

/// Upstream's `CLIENT_TOOL_SEARCH_TOOLS`.
fn stream_client_tool_search_tools() -> Value {
    json!([
        {"type": "provider", "id": "openai.tool_search", "name": "toolSearch", "args": {"execution": "client", "description": "Search for available tools based on what the user needs.", "parameters": {"type": "object", "properties": {"goal": {"type": "string", "description": "What the user is trying to accomplish"}}, "required": ["goal"], "additionalProperties": false}}},
        {"type": "function", "name": "get_weather", "description": "Get the current weather at a specific location", "inputSchema": {"type": "object", "properties": {"location": {"type": "string"}, "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]}}, "required": ["location", "unit"], "additionalProperties": false}, "strict": true, "providerOptions": {"openai": {"deferLoading": true}}},
        {"type": "function", "name": "search_files", "description": "Search through files in the workspace", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "file_types": {"type": "array", "items": {"type": "string"}}}, "required": ["query", "file_types"], "additionalProperties": false}, "strict": true, "providerOptions": {"openai": {"deferLoading": true}}},
    ])
}

/// The test server's `stream-chunks` response: the chunks are the body.
fn sse_mock(chunks: Vec<String>) -> Arc<MockFetch> {
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: chunks.concat().into_bytes(),
    }])
}

/// Upstream's `prepareChunksFixtureResponse`: every non-empty fixture line
/// becomes a `data:` event, then `data: [DONE]`.
fn sse_fixture_mock(fixture: &str) -> Arc<MockFetch> {
    let mut chunks: Vec<String> = fixture
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("data: {line}\n\n"))
        .collect();
    chunks.push("data: [DONE]\n\n".into());
    sse_mock(chunks)
}

/// The model config of upstream's `createModel(modelId)`: provider
/// `openai`, `https://api.openai.com/v1`, `Authorization: Bearer APIKEY`.
fn stream_config(provider: &str, mock: &Arc<MockFetch>) -> OpenAIModelConfig {
    OpenAIModelConfig::fixed(
        provider.into(),
        "https://api.openai.com/v1".into(),
        ProviderHeaders::bearer(
            Credential::Explicit(Resolvable::Value("APIKEY".into())),
            Vec::new(),
            None,
        ),
        Some(mock.transport()),
        None,
    )
}

/// `createModel(modelId).doStream(options)`.
async fn stream_do(
    mock: &Arc<MockFetch>,
    model_id: &str,
    options: Value,
) -> Result<StreamResult, AiMuxError> {
    let model = OpenAIResponsesModel::from_config(model_id.into(), stream_config("openai", mock));
    stream_do_with(&model, options).await
}

async fn stream_do_with(
    model: &OpenAIResponsesModel,
    options: Value,
) -> Result<StreamResult, AiMuxError> {
    let options: CallOptions = serde_json::from_value(options).expect("call options");
    model.do_stream(&options).await
}

/// `convertReadableStreamToArray(stream)`, each part as its JSON. A stream
/// item that is an `Err` is an `error` part. Generated ids go through
/// [`stream_mock_ids`].
async fn stream_collect(result: StreamResult) -> Vec<Value> {
    let parts: Vec<Value> = result
        .stream
        .map(|part| match part {
            Ok(part) => serde_json::to_value(part).expect("stream part JSON"),
            Err(error) => json!({ "type": "error", "error": error }),
        })
        .collect()
        .await;
    stream_mock_ids(parts)
}

/// Upstream's `createModel` passes `generateId: mockId()` (`id-0`, `id-1`,
/// ...). The aimux Responses model has no id hook: it numbers streamed
/// sources `source-N` with a process-wide counter and gives an MCP approval
/// request's tool call a random id. Those generated ids are renamed `id-N`
/// in order of first appearance, which is the order upstream generates them.
fn stream_mock_ids(mut parts: Vec<Value>) -> Vec<Value> {
    let generated: Vec<String> = parts
        .iter()
        .filter_map(|part| match part["type"].as_str() {
            Some("source") => part["id"].as_str().filter(|id| id.starts_with("source-")),
            Some("tool-approval-request") => part["toolCallId"].as_str(),
            _ => None,
        })
        .map(str::to_owned)
        .collect();
    let mut renamed: Vec<(String, String)> = Vec::new();
    for part in &mut parts {
        for key in ["id", "toolCallId"] {
            let Some(id) = part.get(key).and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            if !generated.contains(&id) {
                continue;
            }
            let mock = match renamed.iter().find(|(from, _)| *from == id) {
                Some((_, mock)) => mock.clone(),
                None => {
                    let mock = format!("id-{}", renamed.len());
                    renamed.push((id, mock.clone()));
                    mock
                }
            };
            part[key] = Value::String(mock);
        }
    }
    parts
}

/// A `toMatchSnapshot()` value from `snapshots/`.
fn stream_snapshot(snapshot: &str) -> Vec<Value> {
    serde_json::from_str(snapshot).expect("snapshot JSON")
}

/// `expect(actual).toMatchObject(expected)`: every expected object key
/// matches, recursively; arrays match element by element.
fn stream_assert_match(actual: &Value, expected: &Value) {
    fn matches(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => expected
                .iter()
                .all(|(key, value)| actual.get(key).is_some_and(|actual| matches(actual, value))),
            (Value::Array(actual), Value::Array(expected)) => {
                actual.len() == expected.len()
                    && actual.iter().zip(expected).all(|(a, e)| matches(a, e))
            }
            _ => actual == expected,
        }
    }
    assert!(
        matches(actual, expected),
        "toMatchObject failed\n  actual: {actual}\nexpected: {expected}"
    );
}

/// `expect(parts).toEqual(expected)`, reporting the first differing part.
fn stream_assert_parts(actual: &[Value], expected: &[Value]) {
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            actual == expected,
            "part {index} differs\n  actual: {actual}\nexpected: {expected}"
        );
    }
    let extra = |parts: &[Value], from: usize| -> Vec<String> {
        parts[from..].iter().map(Value::to_string).collect()
    };
    assert!(
        actual.len() == expected.len(),
        "part count differs: {} vs {}\n  actual extra: {:?}\nexpected extra: {:?}",
        actual.len(),
        expected.len(),
        extra(actual, expected.len().min(actual.len())),
        extra(expected, expected.len().min(actual.len())),
    );
}

/// The first part of a type.
fn stream_find<'a>(parts: &'a [Value], kind: &str) -> Option<&'a Value> {
    parts.iter().find(|part| part["type"] == kind)
}

/// `{ ...base, ...extra }`.
fn stream_merge(base: &Value, extra: Value) -> Value {
    let mut merged = base.clone();
    for (key, value) in extra.as_object().expect("object").clone() {
        merged[key] = value;
    }
    merged
}

/// The parts whose type starts with `tool-`.
fn stream_tool_parts(parts: &[Value]) -> Vec<Value> {
    parts
        .iter()
        .filter(|part| {
            part["type"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("tool-"))
        })
        .cloned()
        .collect()
}

/// A `tool-call` part with its `input` parsed (`{ ...part, input: JSON.parse(part.input) }`).
fn stream_parse_tool_call_input(mut part: Value) -> Value {
    if part["type"] == "tool-call" {
        part["input"] =
            serde_json::from_str(part["input"].as_str().expect("input text")).expect("input JSON");
    }
    part
}

mod openai_responses_language_model {
    use super::*;

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › should throw a descriptive error when the response has no output
        #[tokio::test]
        async fn should_throw_a_descriptive_error_when_the_response_has_no_output() {
            let error = generate_error(
                "gpt-4o",
                json!({
                  "id": "resp_no_output",
                  "object": "response",
                  "created_at": 1741257730,
                  "status": "incomplete",
                  "error": null,
                  "incomplete_details": {
                    "reason": "content_filter"
                  },
                  "model": "gpt-4o-2024-07-18",
                  "usage": {
                    "input_tokens": 10,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 0,
                    "output_tokens_details": {
                      "reasoning_tokens": 0
                    },
                    "total_tokens": 10
                  },
                  "metadata": {}
                }),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            assert!(
                error
                    .to_string()
                    .contains("Responses API returned no output (content_filter)"),
                "{error}"
            );
        }

        mod basic_text_response {
            use super::*;

            fn response_body() -> Value {
                json!({
                  "id": "resp_67c97c0203188190a025beb4a75242bc",
                  "object": "response",
                  "created_at": 1741257730,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-4o-2024-07-18",
                  "output": [
                    {
                      "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "answer text",
                          "annotations": []
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": null,
                    "summary": null,
                    "context": "current_turn"
                  },
                  "store": true,
                  "temperature": 1,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 345,
                    "input_tokens_details": {
                      "cached_tokens": 234,
                      "cache_write_tokens": 45,
                      "future_input_detail": {
                        "tokens": 7
                      }
                    },
                    "output_tokens": 538,
                    "output_tokens_details": {
                      "reasoning_tokens": 123,
                      "future_output_detail": [
                        "preserved"
                      ]
                    },
                    "total_tokens": 572,
                    "future_usage_field": {
                      "value": true
                    }
                  },
                  "user": null,
                  "metadata": {}
                })
            }

            async fn run(model_id: &str, options: Value) -> Generated {
                generate(model_id, response_body(), options).await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should generate text
            #[tokio::test]
            async fn should_generate_text() {
                let r = run("gpt-4o", json!({ "prompt": test_prompt() })).await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1"
                          }
                        },
                        "text": "answer text",
                        "type": "text"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should extract usage
            #[tokio::test]
            async fn should_extract_usage() {
                let r = run("gpt-4o", json!({ "prompt": test_prompt() })).await;
                assert_json_eq(
                    &r.usage(),
                    &json!({
                      "inputTokens": {
                        "cacheRead": 234,
                        "cacheWrite": 45,
                        "noCache": 66,
                        "total": 345
                      },
                      "outputTokens": {
                        "reasoning": 123,
                        "text": 415,
                        "total": 538
                      },
                      "raw": {
                        "future_usage_field": {
                          "value": true
                        },
                        "input_tokens": 345,
                        "input_tokens_details": {
                          "cache_write_tokens": 45,
                          "cached_tokens": 234,
                          "future_input_detail": {
                            "tokens": 7
                          }
                        },
                        "output_tokens": 538,
                        "output_tokens_details": {
                          "future_output_detail": [
                            "preserved"
                          ],
                          "reasoning_tokens": 123
                        },
                        "total_tokens": 572
                      }
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should preserve orchestration usage fields in raw
            #[tokio::test]
            async fn should_preserve_orchestration_usage_fields_in_raw() {
                let r = generate(
                    "gpt-4o",
                    json!({
                      "id": "resp_67c97c0203188190a025beb4a75242bc",
                      "object": "response",
                      "created_at": 1741257730,
                      "status": "completed",
                      "error": null,
                      "incomplete_details": null,
                      "input": [],
                      "instructions": null,
                      "max_output_tokens": null,
                      "model": "gpt-4o-2024-07-18",
                      "output": [
                        {
                          "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                          "type": "message",
                          "status": "completed",
                          "role": "assistant",
                          "content": [
                            {
                              "type": "output_text",
                              "text": "answer text",
                              "annotations": []
                            }
                          ]
                        }
                      ],
                      "parallel_tool_calls": true,
                      "previous_response_id": null,
                      "reasoning": {
                        "effort": null,
                        "summary": null
                      },
                      "store": true,
                      "temperature": 1,
                      "text": {
                        "format": {
                          "type": "text"
                        }
                      },
                      "tool_choice": "auto",
                      "tools": [],
                      "top_p": 1,
                      "truncation": "disabled",
                      "usage": {
                        "input_tokens": 120,
                        "input_tokens_details": {
                          "cached_tokens": 0,
                          "orchestration_input_tokens": 40,
                          "orchestration_input_cached_tokens": 10
                        },
                        "output_tokens": 80,
                        "output_tokens_details": {
                          "reasoning_tokens": 0,
                          "orchestration_output_tokens": 25
                        },
                        "total_tokens": 265
                      },
                      "user": null,
                      "metadata": {}
                    }),
                    json!({ "prompt": test_prompt() }),
                )
                .await;
                assert_json_match_object(
                    &r.usage()["raw"],
                    &json!({
                      "input_tokens_details": {
                        "orchestration_input_tokens": 40,
                        "orchestration_input_cached_tokens": 10
                      },
                      "output_tokens_details": {
                        "orchestration_output_tokens": 25
                      }
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should extract response id metadata
            #[tokio::test]
            async fn should_extract_response_id_metadata() {
                let r = run("gpt-4o", json!({ "prompt": test_prompt() })).await;
                assert_json_eq(
                    &r.provider_metadata(),
                    &json!({
                      "openai": {
                        "reasoningContext": "current_turn",
                        "responseId": "resp_67c97c0203188190a025beb4a75242bc"
                      }
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send model id, settings, and input
            #[tokio::test]
            async fn should_send_model_id_settings_and_input() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": [
                        {
                          "role": "system",
                          "content": "You are a helpful assistant."
                        },
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "temperature": 0.5,
                      "topP": 0.3,
                      "providerOptions": {
                        "openai": {
                          "maxToolCalls": 10
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": "You are a helpful assistant.",
                          "role": "system"
                        },
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "max_tool_calls": 10,
                      "model": "gpt-4o",
                      "temperature": 0.5,
                      "top_p": 0.3
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should keep temperature and topP for gpt-5.1 models when reasoning effort is none
            #[tokio::test]
            async fn should_keep_temperature_and_top_p_for_gpt_5_1_models_when_reasoning_effort_is()
            {
                let r = run(
                    "gpt-5.1",
                    json!({
                      "prompt": test_prompt(),
                      "temperature": 0.5,
                      "topP": 0.3,
                      "providerOptions": {
                        "openai": {
                          "reasoningEffort": "none"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5.1",
                      "reasoning": {
                        "effort": "none"
                      },
                      "temperature": 0.5,
                      "top_p": 0.3
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should remove unsupported settings for o1
            #[tokio::test]
            async fn should_remove_unsupported_settings_for_o1() {
                let r = run(
                    "o1",
                    json!({
                      "prompt": [
                        {
                          "role": "system",
                          "content": "You are a helpful assistant."
                        },
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "temperature": 0.5,
                      "topP": 0.3
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "o1",
                      "input": [
                        {
                          "role": "developer",
                          "content": "You are a helpful assistant."
                        },
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ]
                    }),
                );
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "details": "temperature is not supported for reasoning models",
                        "feature": "temperature",
                        "type": "unsupported"
                      },
                      {
                        "details": "topP is not supported for reasoning models",
                        "feature": "topP",
                        "type": "unsupported"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should remove and warn about unsupported settings for reasoning model %s
            #[tokio::test]
            async fn should_remove_and_warn_about_unsupported_settings_for_reasoning_model() {
                for &model_id in REASONING_MODEL_IDS {
                    let r = run(
                        model_id,
                        json!({
                          "prompt": [
                            {
                              "role": "system",
                              "content": "You are a helpful assistant."
                            },
                            {
                              "role": "user",
                              "content": [
                                {
                                  "type": "text",
                                  "text": "Hello"
                                }
                              ]
                            }
                          ],
                          "temperature": 0.5,
                          "topP": 0.3
                        }),
                    )
                    .await;
                    let expected_messages = json!([
                      {
                        "role": "developer",
                        "content": "You are a helpful assistant."
                      },
                      {
                        "role": "user",
                        "content": [
                          {
                            "type": "input_text",
                            "text": "Hello"
                          }
                        ]
                      }
                    ]);
                    assert_json_eq(
                        &r.body,
                        &json!({ "model": model_id, "input": expected_messages }),
                    );
                    assert_json_eq(
                        &r.warnings(),
                        &json!([
                          {
                            "details": "temperature is not supported for reasoning models",
                            "feature": "temperature",
                            "type": "unsupported"
                          },
                          {
                            "details": "topP is not supported for reasoning models",
                            "feature": "topP",
                            "type": "unsupported"
                          }
                        ]),
                    );
                }
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send response format json schema
            #[tokio::test]
            async fn should_send_response_format_json_schema() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "responseFormat": {
                        "type": "json",
                        "name": "response",
                        "description": "A response",
                        "schema": {
                          "type": "object",
                          "properties": {
                            "value": {
                              "type": "string"
                            }
                          },
                          "required": [
                            "value"
                          ],
                          "additionalProperties": false,
                          "$schema": "http://json-schema.org/draft-07/schema#"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-4o",
                      "text": {
                        "format": {
                          "description": "A response",
                          "name": "response",
                          "schema": {
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
                          },
                          "strict": true,
                          "type": "json_schema"
                        }
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send response format json object
            #[tokio::test]
            async fn should_send_response_format_json_object() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "responseFormat": {
                        "type": "json"
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-4o",
                      "text": {
                        "format": {
                          "type": "json_object"
                        }
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send parallelToolCalls provider option
            #[tokio::test]
            async fn should_send_parallel_tool_calls_provider_option() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "parallelToolCalls": false
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-4o",
                      "parallel_tool_calls": false
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send store = false provider option and opt into reasoning.encrypted_content for reasoning models
            #[tokio::test]
            async fn should_send_store_false_provider_option_and_opt_into_reasoning_encrypted() {
                let r = run(
                    "gpt-5-mini",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "store": false
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "include": [
                        "reasoning.encrypted_content"
                      ],
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5-mini",
                      "store": false
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send store = false provider option and not opt into reasoning.encrypted_content for non-reasoning models
            #[tokio::test]
            async fn should_send_store_false_provider_option_and_not_opt_into_reasoning_encrypted()
            {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "store": false
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-4o",
                      "store": false
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send store = true provider option without reasoning.encrypted_content
            #[tokio::test]
            async fn should_send_store_true_provider_option_without_reasoning_encrypted_content() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "store": true
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-4o",
                      "store": true
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send user provider option
            #[tokio::test]
            async fn should_send_user_provider_option() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "user": "user_123"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-4o",
                      "user": "user_123"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send conversation provider option
            #[tokio::test]
            async fn should_send_conversation_provider_option() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "conversation": "conv_123"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "conversation": "conv_123"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should warn when both conversation and previousResponseId are provided
            #[tokio::test]
            async fn should_warn_when_both_conversation_and_previous_response_id_are_provided() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "conversation": "conv_123",
                          "previousResponseId": "resp_123"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "conversation": "conv_123",
                      "previous_response_id": "resp_123"
                    }),
                );
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "type": "unsupported",
                        "feature": "conversation",
                        "details": "conversation and previousResponseId cannot be used together"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send previous response id provider option
            #[tokio::test]
            async fn should_send_previous_response_id_provider_option() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "previousResponseId": "resp_123"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "previous_response_id": "resp_123"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should not send item references for reasoning items when previousResponseId is set
            #[tokio::test]
            async fn should_not_send_item_references_for_reasoning_items_when_previous_response_id()
            {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "Hello"
                            }
                          ]
                        },
                        {
                          "role": "assistant",
                          "content": [
                            {
                              "type": "reasoning",
                              "text": "Let me think...",
                              "providerOptions": {
                                "openai": {
                                  "itemId": "rs_existing_123"
                                }
                              }
                            }
                          ]
                        }
                      ],
                      "providerOptions": {
                        "openai": {
                          "previousResponseId": "resp_123",
                          "store": true
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "previous_response_id": "resp_123",
                      "store": true
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send client-executed function calls in full when previousResponseId is set
            #[tokio::test]
            async fn should_send_client_executed_function_calls_in_full_when_previous_response_id_is()
             {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "What is the weather?"
                            }
                          ]
                        },
                        {
                          "role": "assistant",
                          "content": [
                            {
                              "type": "tool-call",
                              "toolCallId": "call_123",
                              "toolName": "weather",
                              "input": {
                                "location": "San Francisco"
                              },
                              "providerOptions": {
                                "openai": {
                                  "itemId": "fc_existing_123"
                                }
                              }
                            }
                          ]
                        },
                        {
                          "role": "tool",
                          "content": [
                            {
                              "type": "tool-result",
                              "toolCallId": "call_123",
                              "toolName": "weather",
                              "output": {
                                "type": "json",
                                "value": {
                                  "temp": 72
                                }
                              }
                            }
                          ]
                        }
                      ],
                      "providerOptions": {
                        "openai": {
                          "previousResponseId": "resp_123",
                          "store": true
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "What is the weather?"
                            }
                          ]
                        },
                        {
                          "type": "function_call",
                          "call_id": "call_123",
                          "name": "weather",
                          "arguments": "{\"location\":\"San Francisco\"}"
                        },
                        {
                          "type": "function_call_output",
                          "call_id": "call_123",
                          "output": "{\"temp\":72}"
                        }
                      ],
                      "previous_response_id": "resp_123",
                      "store": true
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should replay a regular function named tool_search as a function call
            #[tokio::test]
            async fn should_replay_a_regular_function_named_tool_search_as_a_function_call() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "Search the synthetic records."
                            }
                          ]
                        },
                        {
                          "role": "assistant",
                          "content": [
                            {
                              "type": "tool-call",
                              "toolCallId": "call_123",
                              "toolName": "tool_search",
                              "input": {
                                "query": "synthetic query",
                                "limit": 10
                              }
                            }
                          ]
                        },
                        {
                          "role": "tool",
                          "content": [
                            {
                              "type": "tool-result",
                              "toolCallId": "call_123",
                              "toolName": "tool_search",
                              "output": {
                                "type": "json",
                                "value": {
                                  "tools": []
                                }
                              }
                            }
                          ]
                        }
                      ],
                      "tools": [
                        {
                          "type": "function",
                          "name": "tool_search",
                          "description": "Search synthetic records",
                          "inputSchema": {
                            "type": "object",
                            "properties": {
                              "query": {
                                "type": "string"
                              },
                              "limit": {
                                "type": "number"
                              }
                            },
                            "required": [
                              "query",
                              "limit"
                            ],
                            "additionalProperties": false
                          }
                        }
                      ]
                    }),
                )
                .await;
                assert_json_match_object(
                    &r.body["tools"],
                    &json!([
                      {
                        "type": "function",
                        "name": "tool_search"
                      }
                    ]),
                );
                let input = r.body["input"].as_array().expect("input array");
                assert_json_eq(
                    &json!(input[1..]),
                    &json!([
                      {
                        "type": "function_call",
                        "call_id": "call_123",
                        "name": "tool_search",
                        "arguments": "{\"query\":\"synthetic query\",\"limit\":10}"
                      },
                      {
                        "type": "function_call_output",
                        "call_id": "call_123",
                        "output": "{\"tools\":[]}"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send metadata provider option
            #[tokio::test]
            async fn should_send_metadata_provider_option() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "user": "user_123"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "user": "user_123"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send reasoningEffort and reasoningSummary provider options for %s
            #[tokio::test]
            async fn should_send_reasoning_effort_and_reasoning_summary_provider_options_for() {
                for &model_id in REASONING_MODEL_IDS {
                    let r = run(
                        model_id,
                        json!({
                          "prompt": test_prompt(),
                          "providerOptions": {
                            "openai": {
                              "reasoningEffort": "low",
                              "reasoningSummary": "auto"
                            }
                          }
                        }),
                    )
                    .await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "model": model_id,
                          "input": [
                            {
                              "role": "user",
                              "content": [
                                {
                                  "type": "input_text",
                                  "text": "Hello"
                                }
                              ]
                            }
                          ],
                          "reasoning": {
                            "effort": "low",
                            "summary": "auto"
                          }
                        }),
                    );
                    assert_json_eq(&r.warnings(), &json!([]));
                }
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should allow forcing reasoning mode for unrecognized model IDs via providerOptions
            #[tokio::test]
            async fn should_allow_forcing_reasoning_mode_for_unrecognized_model_ids_via_provider() {
                let r = run(
                    "stealth-reasoning-model",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "forceReasoning": true,
                          "reasoningEffort": "low",
                          "reasoningSummary": "auto"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "stealth-reasoning-model",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "reasoning": {
                        "effort": "low",
                        "summary": "auto"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send xhigh reasoningEffort for codex-max model
            #[tokio::test]
            async fn should_send_xhigh_reasoning_effort_for_codex_max_model() {
                let r = run(
                    "gpt-5.1-codex-max",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "reasoningEffort": "xhigh"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5.1-codex-max",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "reasoning": {
                        "effort": "xhigh",
                        "summary": "detailed"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send GPT-5.6 reasoning effort, mode, and context
            #[tokio::test]
            async fn should_send_gpt_5_6_reasoning_effort_mode_and_context() {
                let r = run(
                    "gpt-5.6",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "reasoningEffort": "max",
                          "reasoningMode": "pro",
                          "reasoningContext": "all_turns"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5.6",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "reasoning": {
                        "effort": "max",
                        "summary": "detailed",
                        "mode": "pro",
                        "context": "all_turns"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should preserve disabled reasoning for %s
            #[tokio::test]
            async fn should_preserve_disabled_reasoning_for() {
                for model_id in ["gpt-6-sol", "gpt-6-luna"] {
                    let r = run(
                        model_id,
                        json!({
                          "prompt": test_prompt(),
                          "providerOptions": {
                            "openai": {
                              "reasoningEffort": "none"
                            }
                          }
                        }),
                    )
                    .await;
                    assert_json_match_object(
                        &r.body,
                        &json!({
                          "model": model_id,
                          "reasoning": {
                            "effort": "none"
                          }
                        }),
                    );
                    assert_json_eq(&r.warnings(), &json!([]));
                }
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should omit unsupported GPT-6 reasoning effort %s
            #[tokio::test]
            async fn should_omit_unsupported_gpt_6_reasoning_effort() {
                for reasoning_effort in ["none", "minimal"] {
                    let r = run(
                        "gpt-6-astra",
                        json!({
                          "prompt": test_prompt(),
                          "providerOptions": {
                            "openai": {
                              "reasoningEffort": reasoning_effort
                            }
                          }
                        }),
                    )
                    .await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "model": "gpt-6-astra",
                          "input": [
                            {
                              "role": "user",
                              "content": [
                                {
                                  "type": "input_text",
                                  "text": "Hello"
                                }
                              ]
                            }
                          ]
                        }),
                    );
                    assert_json_eq(
                        &r.warnings(),
                        &json!([
                          {
                            "type": "unsupported",
                            "feature": "reasoningEffort",
                            "details": "gpt-6-astra only supports the following reasoning efforts: low, medium, high, xhigh, max"
                          }
                        ]),
                    );
                }
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should strip sampling and logprob settings for GPT-6 models
            #[tokio::test]
            async fn should_strip_sampling_and_logprob_settings_for_gpt_6_models() {
                let r = run(
                    "gpt-6-astra",
                    json!({
                      "prompt": test_prompt(),
                      "temperature": 0.5,
                      "topP": 0.7,
                      "providerOptions": {
                        "openai": {
                          "reasoningEffort": "low",
                          "logprobs": 5,
                          "include": [
                            "message.output_text.logprobs"
                          ]
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-6-astra",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "reasoning": {
                        "effort": "low",
                        "summary": "detailed"
                      }
                    }),
                );
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "type": "unsupported",
                        "feature": "temperature",
                        "details": "temperature is not supported for reasoning models"
                      },
                      {
                        "type": "unsupported",
                        "feature": "topP",
                        "details": "topP is not supported for reasoning models"
                      },
                      {
                        "type": "unsupported",
                        "feature": "logprobs",
                        "details": "logprobs is not supported for reasoning models"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should let GPT-5.6 use its default effort with pro mode
            #[tokio::test]
            async fn should_let_gpt_5_6_use_its_default_effort_with_pro_mode() {
                let r = run(
                    "gpt-5.6",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "reasoningMode": "pro",
                          "reasoningContext": "auto"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5.6",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "reasoning": {
                        "mode": "pro",
                        "context": "auto"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should warn about GPT-5.6 reasoning controls on non-reasoning models
            #[tokio::test]
            async fn should_warn_about_gpt_5_6_reasoning_controls_on_non_reasoning_models() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "reasoningMode": "pro",
                          "reasoningContext": "all_turns"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ]
                    }),
                );
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "type": "unsupported",
                        "feature": "reasoningMode",
                        "details": "reasoningMode is not supported for non-reasoning models"
                      },
                      {
                        "type": "unsupported",
                        "feature": "reasoningContext",
                        "details": "reasoningContext is not supported for non-reasoning models"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should not send and warn about unsupported reasoningEffort and reasoningSummary provider options for %s
            #[tokio::test]
            async fn should_not_send_and_warn_about_unsupported_reasoning_effort_and_reasoning() {
                for &model_id in NON_REASONING_MODEL_IDS {
                    let r = run(
                        model_id,
                        json!({
                          "prompt": test_prompt(),
                          "providerOptions": {
                            "openai": {
                              "reasoningEffort": "low"
                            }
                          }
                        }),
                    )
                    .await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "model": model_id,
                          "input": [
                            {
                              "role": "user",
                              "content": [
                                {
                                  "type": "input_text",
                                  "text": "Hello"
                                }
                              ]
                            }
                          ]
                        }),
                    );
                    assert_json_eq(
                        &r.warnings(),
                        &json!([
                          {
                            "type": "unsupported",
                            "feature": "reasoningEffort",
                            "details": "reasoningEffort is not supported for non-reasoning models"
                          }
                        ]),
                    );
                }
            }

            mod top_level_reasoning_option {
                use super::*;

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › top-level reasoning option › should not set reasoning config when reasoning is "provider-default"
                #[tokio::test]
                async fn should_not_set_reasoning_config_when_reasoning_is_provider_default() {
                    let r = run(
                        "o3-mini",
                        json!({
                          "prompt": test_prompt(),
                          "reasoning": "provider-default"
                        }),
                    )
                    .await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "model": "o3-mini",
                          "input": [
                            {
                              "role": "user",
                              "content": [
                                {
                                  "type": "input_text",
                                  "text": "Hello"
                                }
                              ]
                            }
                          ]
                        }),
                    );
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › top-level reasoning option › should map top-level reasoning=%s to reasoning effort
                #[tokio::test]
                async fn should_map_top_level_reasoning_to_reasoning_effort() {
                    for reasoning_value in ["none", "minimal", "low", "medium", "high", "xhigh"] {
                        let r = run(
                            "o3-mini",
                            json!({
                              "prompt": test_prompt(),
                              "reasoning": reasoning_value
                            }),
                        )
                        .await;
                        let mut reasoning = json!({ "effort": reasoning_value });
                        if reasoning_value != "none" {
                            reasoning["summary"] = json!("detailed");
                        }
                        assert_json_eq(
                            &r.body,
                            &json!({
                                "model": "o3-mini",
                                "input": [
                                    {
                                        "role": "user",
                                        "content": [{ "type": "input_text", "text": "Hello" }],
                                    },
                                ],
                                "reasoning": reasoning,
                            }),
                        );
                        if reasoning_value == "none" {
                            assert_json_eq(&r.warnings(), &json!([]));
                        }
                    }
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › top-level reasoning option › should let providerOptions.openai.reasoningEffort take precedence over top-level reasoning
                #[tokio::test]
                async fn should_let_provider_options_openai_reasoning_effort_take_precedence_over_top()
                 {
                    let r = run(
                        "o3-mini",
                        json!({
                          "prompt": test_prompt(),
                          "reasoning": "low",
                          "providerOptions": {
                            "openai": {
                              "reasoningEffort": "high"
                            }
                          }
                        }),
                    )
                    .await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "model": "o3-mini",
                          "input": [
                            {
                              "role": "user",
                              "content": [
                                {
                                  "type": "input_text",
                                  "text": "Hello"
                                }
                              ]
                            }
                          ],
                          "reasoning": {
                            "effort": "high",
                            "summary": "detailed"
                          }
                        }),
                    );
                    assert_json_eq(&r.warnings(), &json!([]));
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › top-level reasoning option › should strip temperature/topP when top-level reasoning is set to non-none value on reasoning model
                #[tokio::test]
                async fn should_strip_temperature_top_p_when_top_level_reasoning_is_set_to_non_none()
                 {
                    let r = run(
                        "o3-mini",
                        json!({
                          "prompt": test_prompt(),
                          "reasoning": "medium",
                          "temperature": 0.5,
                          "topP": 0.7
                        }),
                    )
                    .await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "model": "o3-mini",
                          "input": [
                            {
                              "role": "user",
                              "content": [
                                {
                                  "type": "input_text",
                                  "text": "Hello"
                                }
                              ]
                            }
                          ],
                          "reasoning": {
                            "effort": "medium",
                            "summary": "detailed"
                          }
                        }),
                    );
                    assert_json_eq(
                        &r.warnings(),
                        &json!([
                          {
                            "type": "unsupported",
                            "feature": "temperature",
                            "details": "temperature is not supported for reasoning models"
                          },
                          {
                            "type": "unsupported",
                            "feature": "topP",
                            "details": "topP is not supported for reasoning models"
                          }
                        ]),
                    );
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › top-level reasoning option › should not strip parameters when top-level reasoning is none
                #[tokio::test]
                async fn should_not_strip_parameters_when_top_level_reasoning_is_none() {
                    let r = run(
                        "gpt-4o",
                        json!({
                          "prompt": test_prompt(),
                          "reasoning": "none",
                          "temperature": 0.5,
                          "topP": 0.7
                        }),
                    )
                    .await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "model": "gpt-4o",
                          "input": [
                            {
                              "role": "user",
                              "content": [
                                {
                                  "type": "input_text",
                                  "text": "Hello"
                                }
                              ]
                            }
                          ],
                          "temperature": 0.5,
                          "top_p": 0.7
                        }),
                    );
                    assert_json_eq(&r.warnings(), &json!([]));
                }
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send instructions provider option
            #[tokio::test]
            async fn should_send_instructions_provider_option() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "instructions": "You are a friendly assistant."
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "instructions": "You are a friendly assistant."
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send include provider option
            #[tokio::test]
            async fn should_send_include_provider_option() {
                let r = run(
                    "o3-mini",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "include": [
                            "reasoning.encrypted_content"
                          ]
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "o3-mini",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "include": [
                        "reasoning.encrypted_content"
                      ]
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send include provider option with multiple values
            #[tokio::test]
            async fn should_send_include_provider_option_with_multiple_values() {
                let r = run(
                    "o3-mini",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "include": [
                            "reasoning.encrypted_content",
                            "file_search_call.results",
                            "web_search_call.results"
                          ]
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "o3-mini",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "include": [
                        "reasoning.encrypted_content",
                        "file_search_call.results",
                        "web_search_call.results"
                      ]
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send textVerbosity provider option
            #[tokio::test]
            async fn should_send_text_verbosity_provider_option_low() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "textVerbosity": "low"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5",
                      "text": {
                        "verbosity": "low"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send textVerbosity provider option
            #[tokio::test]
            async fn should_send_text_verbosity_provider_option_medium() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "textVerbosity": "medium"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5",
                      "text": {
                        "verbosity": "medium"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send textVerbosity provider option
            #[tokio::test]
            async fn should_send_text_verbosity_provider_option_high() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "textVerbosity": "high"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5",
                      "text": {
                        "verbosity": "high"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send promptCacheKey provider option
            #[tokio::test]
            async fn should_send_prompt_cache_key_provider_option() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "promptCacheKey": "test-cache-key-123"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "prompt_cache_key": "test-cache-key-123"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send promptCacheRetention provider option
            #[tokio::test]
            async fn should_send_prompt_cache_retention_provider_option() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "promptCacheRetention": "24h"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "prompt_cache_retention": "24h"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send promptCacheOptions provider option
            #[tokio::test]
            async fn should_send_prompt_cache_options_provider_option() {
                let r = run(
                    "gpt-5.6",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "promptCacheOptions": {
                            "mode": "explicit",
                            "ttl": "30m"
                          }
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5.6",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "prompt_cache_options": {
                        "mode": "explicit",
                        "ttl": "30m"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should insert a reasoning effort configuration update before the prompt
            #[tokio::test]
            async fn should_insert_a_reasoning_effort_configuration_update_before_the_prompt() {
                let r = run(
                    "gpt-6-astra",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "previousResponseId": "resp_123",
                          "reasoningEffort": "low",
                          "reasoningEffortUpdate": "high"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-6-astra",
                      "input": [
                        {
                          "type": "configuration_update",
                          "reasoning": {
                            "effort": "high"
                          }
                        },
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "previous_response_id": "resp_123",
                      "reasoning": {
                        "effort": "low",
                        "summary": "detailed"
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should omit reasoning effort updates for models before GPT-6
            #[tokio::test]
            async fn should_omit_reasoning_effort_updates_for_models_before_gpt_6() {
                let r = run(
                    "gpt-5.6",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "reasoningEffortUpdate": "high"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body["input"],
                    &json!([
                      {
                        "role": "user",
                        "content": [
                          {
                            "type": "input_text",
                            "text": "Hello"
                          }
                        ]
                      }
                    ]),
                );
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "type": "unsupported",
                        "feature": "reasoningEffortUpdate",
                        "details": "reasoningEffortUpdate is only supported by GPT-6 and later models"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should omit reasoning effort updates with incompatible automatic context management
            #[tokio::test]
            async fn should_omit_reasoning_effort_updates_with_incompatible_automatic_context() {
                let r = run(
                    "gpt-6-astra",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "reasoningEffortUpdate": "high",
                          "contextManagement": [
                            {
                              "type": "compaction",
                              "compactThreshold": 1000
                            }
                          ]
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body["input"],
                    &json!([
                      {
                        "role": "user",
                        "content": [
                          {
                            "type": "input_text",
                            "text": "Hello"
                          }
                        ]
                      }
                    ]),
                );
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "type": "unsupported",
                        "feature": "reasoningEffortUpdate",
                        "details": "reasoningEffortUpdate requires standard reasoning mode without automatic compaction or automatic truncation"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should omit legacy prompt cache retention for GPT-6 models
            #[tokio::test]
            async fn should_omit_legacy_prompt_cache_retention_for_gpt_6_models() {
                let r = run(
                    "gpt-6-astra",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "promptCacheRetention": "24h"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-6-astra",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ]
                    }),
                );
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "type": "unsupported",
                        "feature": "promptCacheRetention",
                        "details": "promptCacheRetention is not supported by GPT-6 and later models; use promptCacheOptions instead"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send safetyIdentifier provider option
            #[tokio::test]
            async fn should_send_safety_identifier_provider_option() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "safetyIdentifier": "test-safety-identifier-123"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "safety_identifier": "test-safety-identifier-123"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send serviceTier fast provider option
            #[tokio::test]
            async fn should_send_service_tier_fast_provider_option() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "serviceTier": "fast"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "service_tier": "fast"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should warn and drop serviceTier fast for a model without priority processing
            #[tokio::test]
            async fn should_warn_and_drop_service_tier_fast_for_a_model_without_priority_processing()
             {
                let r = run(
                    "gpt-5-nano",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "serviceTier": "fast"
                        }
                      }
                    }),
                )
                .await;
                assert!(r.body.get("service_tier").is_none(), "{:#}", r.body);
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "type": "unsupported",
                        "feature": "serviceTier",
                        "details": "priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send serviceTier ultrafast provider option
            #[tokio::test]
            async fn should_send_service_tier_ultrafast_provider_option() {
                let r = run(
                    "gpt-5.6-sol",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "serviceTier": "ultrafast"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5.6-sol",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "service_tier": "ultrafast"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send truncation auto provider option
            #[tokio::test]
            async fn should_send_truncation_auto_provider_option() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "truncation": "auto"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "truncation": "auto"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send truncation disabled provider option
            #[tokio::test]
            async fn should_send_truncation_disabled_provider_option() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "truncation": "disabled"
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "truncation": "disabled"
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should not send truncation when not specified
            #[tokio::test]
            async fn should_not_send_truncation_when_not_specified() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt()
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ]
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send logprobs provider option
            #[tokio::test]
            async fn should_send_logprobs_provider_option() {
                let r = run(
                    "gpt-5",
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "logprobs": 5
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-5",
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ],
                      "top_logprobs": 5,
                      "include": [
                        "message.output_text.logprobs"
                      ]
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send responseFormat json format
            #[tokio::test]
            async fn should_send_response_format_json_format() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "responseFormat": {
                        "type": "json"
                      },
                      "prompt": test_prompt()
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "text": {
                        "format": {
                          "type": "json_object"
                        }
                      },
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ]
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send responseFormat json_schema format
            #[tokio::test]
            async fn should_send_response_format_json_schema_format() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "responseFormat": {
                        "type": "json",
                        "name": "response",
                        "description": "A response",
                        "schema": {
                          "type": "object",
                          "properties": {
                            "value": {
                              "type": "string"
                            }
                          },
                          "required": [
                            "value"
                          ],
                          "additionalProperties": false,
                          "$schema": "http://json-schema.org/draft-07/schema#"
                        }
                      },
                      "prompt": test_prompt()
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-4o",
                      "text": {
                        "format": {
                          "description": "A response",
                          "name": "response",
                          "schema": {
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
                          },
                          "strict": true,
                          "type": "json_schema"
                        }
                      }
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should remove string propertyNames from response schemas and warn
            #[tokio::test]
            async fn should_remove_string_property_names_from_response_schemas_and_warn() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "responseFormat": {
                        "type": "json",
                        "schema": {
                          "type": "object",
                          "properties": {
                            "variables": {
                              "type": "object",
                              "propertyNames": {
                                "type": "string",
                                "pattern": "^[A-Z_]+$"
                              },
                              "additionalProperties": {
                                "type": "string"
                              }
                            }
                          },
                          "required": [
                            "variables"
                          ],
                          "additionalProperties": false
                        }
                      },
                      "prompt": test_prompt()
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "text": {
                        "format": {
                          "type": "json_schema",
                          "strict": true,
                          "name": "response",
                          "schema": {
                            "type": "object",
                            "properties": {
                              "variables": {
                                "type": "object",
                                "additionalProperties": {
                                  "type": "string"
                                }
                              }
                            },
                            "required": [
                              "variables"
                            ],
                            "additionalProperties": false
                          }
                        }
                      },
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ]
                    }),
                );
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "type": "compatibility",
                        "feature": "JSON Schema propertyNames",
                        "details": "OpenAI does not support JSON Schema propertyNames. It was removed before sending the schema, so OpenAI will not enforce property-name constraints."
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should send responseFormat json_schema format with strictJsonSchema false
            #[tokio::test]
            async fn should_send_response_format_json_schema_format_with_strict_json_schema_false()
            {
                let r = run(
                    "gpt-4o",
                    json!({
                      "responseFormat": {
                        "type": "json",
                        "name": "response",
                        "description": "A response",
                        "schema": {
                          "type": "object",
                          "properties": {
                            "value": {
                              "type": "string"
                            }
                          },
                          "required": [
                            "value"
                          ],
                          "additionalProperties": false,
                          "$schema": "http://json-schema.org/draft-07/schema#"
                        }
                      },
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "strictJsonSchema": false
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "model": "gpt-4o",
                      "text": {
                        "format": {
                          "type": "json_schema",
                          "strict": false,
                          "name": "response",
                          "description": "A response",
                          "schema": {
                            "type": "object",
                            "properties": {
                              "value": {
                                "type": "string"
                              }
                            },
                            "required": [
                              "value"
                            ],
                            "additionalProperties": false,
                            "$schema": "http://json-schema.org/draft-07/schema#"
                          }
                        }
                      },
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ]
                    }),
                );
                assert_json_eq(&r.warnings(), &json!([]));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should warn about unsupported settings
            #[tokio::test]
            async fn should_warn_about_unsupported_settings() {
                let r = run(
                    "gpt-4o",
                    json!({
                      "prompt": test_prompt(),
                      "stopSequences": [
                        "\n\n"
                      ],
                      "topK": 0.1,
                      "presencePenalty": 0,
                      "frequencyPenalty": 0,
                      "seed": 42
                    }),
                )
                .await;
                assert_json_eq(
                    &r.warnings(),
                    &json!([
                      {
                        "feature": "topK",
                        "type": "unsupported"
                      },
                      {
                        "feature": "seed",
                        "type": "unsupported"
                      },
                      {
                        "feature": "presencePenalty",
                        "type": "unsupported"
                      },
                      {
                        "feature": "frequencyPenalty",
                        "type": "unsupported"
                      },
                      {
                        "feature": "stopSequences",
                        "type": "unsupported"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › basic text response › should extract logprobs in providerMetadata
            #[tokio::test]
            async fn should_extract_logprobs_in_provider_metadata() {
                let r = generate(
                    "gpt-4o",
                    json!({
                      "id": "resp_67c97c0203188190a025beb4a75242bc",
                      "object": "response",
                      "created_at": 1741257730,
                      "status": "completed",
                      "error": null,
                      "incomplete_details": null,
                      "input": [],
                      "instructions": null,
                      "max_output_tokens": null,
                      "model": "gpt-4o-2024-07-18",
                      "output": [
                        {
                          "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                          "type": "message",
                          "status": "completed",
                          "role": "assistant",
                          "content": [
                            {
                              "type": "output_text",
                              "text": "answer text",
                              "annotations": [],
                              "logprobs": [
                                {
                                  "token": "Hello",
                                  "logprob": -0.0009994634,
                                  "top_logprobs": [
                                    {
                                      "token": "Hello",
                                      "logprob": -0.0009994634
                                    },
                                    {
                                      "token": "Hi",
                                      "logprob": -0.2
                                    }
                                  ]
                                },
                                {
                                  "token": "!",
                                  "logprob": -0.13410144,
                                  "top_logprobs": [
                                    {
                                      "token": "!",
                                      "logprob": -0.13410144
                                    }
                                  ]
                                }
                              ]
                            }
                          ]
                        }
                      ],
                      "parallel_tool_calls": true,
                      "previous_response_id": null,
                      "reasoning": {
                        "effort": null,
                        "summary": null
                      },
                      "store": true,
                      "temperature": 1,
                      "text": {
                        "format": {
                          "type": "text"
                        }
                      },
                      "tool_choice": "auto",
                      "tools": [],
                      "top_p": 1,
                      "truncation": "disabled",
                      "usage": {
                        "input_tokens": 345,
                        "input_tokens_details": {
                          "cached_tokens": 234
                        },
                        "output_tokens": 538,
                        "output_tokens_details": {
                          "reasoning_tokens": 123
                        },
                        "total_tokens": 572
                      },
                      "user": null,
                      "metadata": {}
                    }),
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "logprobs": 2
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.provider_metadata()["openai"]["logprobs"],
                    &json!([
                      [
                        {
                          "logprob": -0.0009994634,
                          "token": "Hello",
                          "top_logprobs": [
                            {
                              "logprob": -0.0009994634,
                              "token": "Hello"
                            },
                            {
                              "logprob": -0.2,
                              "token": "Hi"
                            }
                          ]
                        },
                        {
                          "logprob": -0.13410144,
                          "token": "!",
                          "top_logprobs": [
                            {
                              "logprob": -0.13410144,
                              "token": "!"
                            }
                          ]
                        }
                      ]
                    ]),
                );
            }
        }

        mod errors {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › errors › should throw an error
            #[tokio::test]
            async fn should_throw_an_error() {
                let error = generate_error(
                    "gpt-4o",
                    json_fixture!("openai-error.1"),
                    json!({ "prompt": test_prompt() }),
                )
                .await;
                assert!(
                    error.to_string().contains("You exceeded your current quota, please check your plan and billing details. For more information on this error, read the docs: https://platform.openai.com/docs/guides/error-codes/api-errors."),
                    "{error}"
                );
            }
        }

        mod reasoning {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › reasoning › should handle reasoning with summary
            #[tokio::test]
            async fn should_handle_reasoning_with_summary() {
                let r = generate("o3-mini", json!({
                  "id": "resp_67c97c0203188190a025beb4a75242bc",
                  "object": "response",
                  "created_at": 1741257730,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "o3-mini-2025-01-31",
                  "output": [
                    {
                      "id": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                      "type": "reasoning",
                      "summary": [
                        {
                          "type": "summary_text",
                          "text": "**Exploring burrito origins**\n\nThe user is curious about the debate regarding Taqueria La Cumbre and El Farolito."
                        },
                        {
                          "type": "summary_text",
                          "text": "**Investigating burrito origins**\n\nThere's a fascinating debate about who created the Mission burrito."
                        }
                      ]
                    },
                    {
                      "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "answer text",
                          "annotations": []
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": "low",
                    "summary": "auto"
                  },
                  "store": true,
                  "temperature": 1,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 34,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 538,
                    "output_tokens_details": {
                      "reasoning_tokens": 320
                    },
                    "total_tokens": 572
                  },
                  "user": null,
                  "metadata": {}
                }), json!({
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "reasoningEffort": "low",
                      "reasoningSummary": "auto"
                    }
                  }
                })).await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                            "reasoningEncryptedContent": null
                          }
                        },
                        "text": "**Exploring burrito origins**\n\nThe user is curious about the debate regarding Taqueria La Cumbre and El Farolito.",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                            "reasoningEncryptedContent": null
                          }
                        },
                        "text": "**Investigating burrito origins**\n\nThere's a fascinating debate about who created the Mission burrito.",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1"
                          }
                        },
                        "text": "answer text",
                        "type": "text"
                      }
                    ]),
                );
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "model": "o3-mini",
                      "reasoning": {
                        "effort": "low",
                        "summary": "auto"
                      }
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › reasoning › should generate reasoning encrypted content using real fixture
            #[tokio::test]
            async fn should_generate_reasoning_encrypted_content_using_real_fixture() {
                let r = generate(
                    "gpt-5-mini",
                    json_fixture!("openai-reasoning-encrypted-content.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.code_interpreter",
                          "name": "codeExecution",
                          "args": {}
                        }
                      ]
                    }),
                )
                .await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_reasoning_encrypted_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › reasoning › should handle reasoning with empty summary
            #[tokio::test]
            async fn should_handle_reasoning_with_empty_summary() {
                let r = generate(
                    "o3-mini",
                    json!({
                      "id": "resp_67c97c0203188190a025beb4a75242bc",
                      "object": "response",
                      "created_at": 1741257730,
                      "status": "completed",
                      "error": null,
                      "incomplete_details": null,
                      "input": [],
                      "instructions": null,
                      "max_output_tokens": null,
                      "model": "o3-mini-2025-01-31",
                      "output": [
                        {
                          "id": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                          "type": "reasoning",
                          "summary": []
                        },
                        {
                          "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                          "type": "message",
                          "status": "completed",
                          "role": "assistant",
                          "content": [
                            {
                              "type": "output_text",
                              "text": "answer text",
                              "annotations": []
                            }
                          ]
                        }
                      ],
                      "parallel_tool_calls": true,
                      "previous_response_id": null,
                      "reasoning": {
                        "effort": "low",
                        "summary": "auto"
                      },
                      "store": true,
                      "temperature": 1,
                      "text": {
                        "format": {
                          "type": "text"
                        }
                      },
                      "tool_choice": "auto",
                      "tools": [],
                      "top_p": 1,
                      "truncation": "disabled",
                      "usage": {
                        "input_tokens": 34,
                        "input_tokens_details": {
                          "cached_tokens": 0
                        },
                        "output_tokens": 538,
                        "output_tokens_details": {
                          "reasoning_tokens": 320
                        },
                        "total_tokens": 572
                      },
                      "user": null,
                      "metadata": {}
                    }),
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "reasoningEffort": "low",
                          "reasoningSummary": null
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                            "reasoningEncryptedContent": null
                          }
                        },
                        "text": "",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1"
                          }
                        },
                        "text": "answer text",
                        "type": "text"
                      }
                    ]),
                );
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "model": "o3-mini",
                      "reasoning": {
                        "effort": "low"
                      }
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › reasoning › should handle encrypted content with summary
            #[tokio::test]
            async fn should_handle_encrypted_content_with_summary() {
                let r = generate("o3-mini", json!({
                  "id": "resp_67c97c0203188190a025beb4a75242bc",
                  "object": "response",
                  "created_at": 1741257730,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "o3-mini-2025-01-31",
                  "output": [
                    {
                      "id": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                      "type": "reasoning",
                      "encrypted_content": "encrypted_reasoning_data_abc123",
                      "summary": [
                        {
                          "type": "summary_text",
                          "text": "**Exploring burrito origins**\n\nThe user is curious about the debate regarding Taqueria La Cumbre and El Farolito."
                        },
                        {
                          "type": "summary_text",
                          "text": "**Investigating burrito origins**\n\nThere's a fascinating debate about who created the Mission burrito."
                        }
                      ]
                    },
                    {
                      "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "answer text",
                          "annotations": []
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": "low",
                    "summary": "auto"
                  },
                  "store": true,
                  "temperature": 1,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 34,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 538,
                    "output_tokens_details": {
                      "reasoning_tokens": 320
                    },
                    "total_tokens": 572
                  },
                  "user": null,
                  "metadata": {}
                }), json!({
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "reasoningEffort": "low",
                      "reasoningSummary": "auto",
                      "include": [
                        "reasoning.encrypted_content"
                      ]
                    }
                  }
                })).await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                            "reasoningEncryptedContent": "encrypted_reasoning_data_abc123"
                          }
                        },
                        "text": "**Exploring burrito origins**\n\nThe user is curious about the debate regarding Taqueria La Cumbre and El Farolito.",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                            "reasoningEncryptedContent": "encrypted_reasoning_data_abc123"
                          }
                        },
                        "text": "**Investigating burrito origins**\n\nThere's a fascinating debate about who created the Mission burrito.",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1"
                          }
                        },
                        "text": "answer text",
                        "type": "text"
                      }
                    ]),
                );
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "model": "o3-mini",
                      "reasoning": {
                        "effort": "low",
                        "summary": "auto"
                      },
                      "include": [
                        "reasoning.encrypted_content"
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › reasoning › should handle encrypted content with empty summary
            #[tokio::test]
            async fn should_handle_encrypted_content_with_empty_summary() {
                let r = generate(
                    "o3-mini",
                    json!({
                      "id": "resp_67c97c0203188190a025beb4a75242bc",
                      "object": "response",
                      "created_at": 1741257730,
                      "status": "completed",
                      "error": null,
                      "incomplete_details": null,
                      "input": [],
                      "instructions": null,
                      "max_output_tokens": null,
                      "model": "o3-mini-2025-01-31",
                      "output": [
                        {
                          "id": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                          "type": "reasoning",
                          "encrypted_content": "encrypted_reasoning_data_abc123",
                          "summary": []
                        },
                        {
                          "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                          "type": "message",
                          "status": "completed",
                          "role": "assistant",
                          "content": [
                            {
                              "type": "output_text",
                              "text": "answer text",
                              "annotations": []
                            }
                          ]
                        }
                      ],
                      "parallel_tool_calls": true,
                      "previous_response_id": null,
                      "reasoning": {
                        "effort": "low",
                        "summary": "auto"
                      },
                      "store": true,
                      "temperature": 1,
                      "text": {
                        "format": {
                          "type": "text"
                        }
                      },
                      "tool_choice": "auto",
                      "tools": [],
                      "top_p": 1,
                      "truncation": "disabled",
                      "usage": {
                        "input_tokens": 34,
                        "input_tokens_details": {
                          "cached_tokens": 0
                        },
                        "output_tokens": 538,
                        "output_tokens_details": {
                          "reasoning_tokens": 320
                        },
                        "total_tokens": 572
                      },
                      "user": null,
                      "metadata": {}
                    }),
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "reasoningEffort": "low",
                          "reasoningSummary": null,
                          "include": [
                            "reasoning.encrypted_content"
                          ]
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                            "reasoningEncryptedContent": "encrypted_reasoning_data_abc123"
                          }
                        },
                        "text": "",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1"
                          }
                        },
                        "text": "answer text",
                        "type": "text"
                      }
                    ]),
                );
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "model": "o3-mini",
                      "reasoning": {
                        "effort": "low"
                      },
                      "include": [
                        "reasoning.encrypted_content"
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › reasoning › should handle multiple reasoning blocks
            #[tokio::test]
            async fn should_handle_multiple_reasoning_blocks() {
                let r = generate("o3-mini", json!({
                  "id": "resp_67c97c0203188190a025beb4a75242bc",
                  "object": "response",
                  "created_at": 1741257730,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "o3-mini-2025-01-31",
                  "output": [
                    {
                      "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                      "type": "reasoning",
                      "summary": [
                        {
                          "type": "summary_text",
                          "text": "**Initial analysis**\n\nFirst reasoning block: analyzing the problem structure."
                        },
                        {
                          "type": "summary_text",
                          "text": "**Deeper consideration**\n\nLet me think about the various approaches available."
                        }
                      ]
                    },
                    {
                      "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "Let me think about this step by step.",
                          "annotations": []
                        }
                      ]
                    },
                    {
                      "id": "rs_second_7908809g7gcc9291be3e3fee895028c4",
                      "type": "reasoning",
                      "summary": [
                        {
                          "type": "summary_text",
                          "text": "Second reasoning block: considering alternative approaches."
                        }
                      ]
                    },
                    {
                      "id": "msg_final_78d08d03767d92908f25523f5ge51e77",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "Based on my analysis, here is the solution.",
                          "annotations": []
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": "medium",
                    "summary": "auto"
                  },
                  "store": true,
                  "temperature": null,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": null,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 45,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 628,
                    "output_tokens_details": {
                      "reasoning_tokens": 420
                    },
                    "total_tokens": 673
                  },
                  "user": null,
                  "metadata": {}
                }), json!({
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "reasoningEffort": "medium",
                      "reasoningSummary": "auto"
                    }
                  }
                })).await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                            "reasoningEncryptedContent": null
                          }
                        },
                        "text": "**Initial analysis**\n\nFirst reasoning block: analyzing the problem structure.",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                            "reasoningEncryptedContent": null
                          }
                        },
                        "text": "**Deeper consideration**\n\nLet me think about the various approaches available.",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1"
                          }
                        },
                        "text": "Let me think about this step by step.",
                        "type": "text"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "rs_second_7908809g7gcc9291be3e3fee895028c4",
                            "reasoningEncryptedContent": null
                          }
                        },
                        "text": "Second reasoning block: considering alternative approaches.",
                        "type": "reasoning"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_final_78d08d03767d92908f25523f5ge51e77"
                          }
                        },
                        "text": "Based on my analysis, here is the solution.",
                        "type": "text"
                      }
                    ]),
                );
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "model": "o3-mini",
                      "reasoning": {
                        "effort": "medium",
                        "summary": "auto"
                      }
                    }),
                );
            }
        }

        mod tool_calls {
            use super::*;

            fn response_body() -> Value {
                json!({
                  "id": "resp_67c97c0203188190a025beb4a75242bc",
                  "object": "response",
                  "created_at": 1741257730,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-4o-2024-07-18",
                  "output": [
                    {
                      "type": "function_call",
                      "id": "fc_67caf7f4c1ec8190b27edfb5580cfd31",
                      "call_id": "call_0NdsJqOS8N3J9l2p0p4WpYU9",
                      "name": "weather",
                      "arguments": "{\"location\":\"San Francisco\"}",
                      "status": "completed"
                    },
                    {
                      "type": "function_call",
                      "id": "fc_67caf7f5071c81908209c2909c77af05",
                      "call_id": "call_gexo0HtjUfmAIW4gjNOgyrcr",
                      "name": "cityAttractions",
                      "arguments": "{\"city\":\"San Francisco\"}",
                      "status": "completed"
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": null,
                    "summary": null
                  },
                  "store": true,
                  "temperature": 1,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [
                    {
                      "type": "function",
                      "description": "Get the weather in a location",
                      "name": "weather",
                      "parameters": {
                        "type": "object",
                        "properties": {
                          "location": {
                            "type": "string",
                            "description": "The location to get the weather for"
                          }
                        },
                        "required": [
                          "location"
                        ],
                        "additionalProperties": false
                      },
                      "strict": true
                    },
                    {
                      "type": "function",
                      "description": null,
                      "name": "cityAttractions",
                      "parameters": {
                        "type": "object",
                        "properties": {
                          "city": {
                            "type": "string"
                          }
                        },
                        "required": [
                          "city"
                        ],
                        "additionalProperties": false
                      },
                      "strict": true
                    }
                  ],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 34,
                    "output_tokens": 538,
                    "output_tokens_details": {
                      "reasoning_tokens": 0
                    },
                    "total_tokens": 572
                  },
                  "user": null,
                  "metadata": {}
                })
            }

            fn tool_options() -> Value {
                json!({ "prompt": test_prompt(), "tools": test_tools() })
            }

            /// `{ ...TEST_TOOLS[0], providerOptions: { openai: <options> } }`.
            fn weather_tool_with(openai: Value) -> Value {
                let mut tool = test_tools()[0].clone();
                tool["providerOptions"] = json!({ "openai": openai });
                tool
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should generate tool calls
            #[tokio::test]
            async fn should_generate_tool_calls() {
                let r = generate("gpt-4o", response_body(), tool_options()).await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "input": "{\"location\":\"San Francisco\"}",
                        "providerMetadata": {
                          "openai": {
                            "itemId": "fc_67caf7f4c1ec8190b27edfb5580cfd31"
                          }
                        },
                        "toolCallId": "call_0NdsJqOS8N3J9l2p0p4WpYU9",
                        "toolName": "weather",
                        "type": "tool-call"
                      },
                      {
                        "input": "{\"city\":\"San Francisco\"}",
                        "providerMetadata": {
                          "openai": {
                            "itemId": "fc_67caf7f5071c81908209c2909c77af05"
                          }
                        },
                        "toolCallId": "call_gexo0HtjUfmAIW4gjNOgyrcr",
                        "toolName": "cityAttractions",
                        "type": "tool-call"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should gate async tools to GPT-6 and later models
            #[tokio::test]
            async fn should_gate_async_tools_to_gpt_6_and_later_models() {
                let async_tools = json!([weather_tool_with(json!({ "async": true }))]);

                let unsupported = generate(
                    "gpt-5.6",
                    response_body(),
                    json!({ "prompt": test_prompt(), "tools": async_tools.clone() }),
                )
                .await;
                assert!(
                    unsupported.body["tools"][0].get("async").is_none(),
                    "{:#}",
                    unsupported.body
                );
                let expected_warning = json!({
                  "type": "unsupported",
                  "feature": "async tool calling for \"weather\"",
                  "details": "Async tool calling is only supported by GPT-6 and later models."
                });
                assert!(
                    unsupported
                        .warnings()
                        .as_array()
                        .unwrap()
                        .contains(&expected_warning),
                    "{:#}",
                    unsupported.warnings()
                );

                let supported = generate(
                    "gpt-99",
                    response_body(),
                    json!({ "prompt": test_prompt(), "tools": async_tools.clone() }),
                )
                .await;
                assert_eq!(supported.body["tools"][0]["async"], json!(true));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should JSON-encode error outputs for tools with an output schema
            #[tokio::test]
            async fn should_json_encode_error_outputs_for_tools_with_an_output_schema() {
                let output_schema = json!({
                  "type": "object",
                  "properties": {
                    "temperature": {
                      "type": "number"
                    }
                  },
                  "required": [
                    "temperature"
                  ],
                  "additionalProperties": false
                });
                let options = json!({
                    "prompt": [
                        {
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "tool-call",
                                    "toolCallId": "call_123",
                                    "toolName": "weather",
                                    "input": { "location": "San Francisco" },
                                },
                            ],
                        },
                        {
                            "role": "tool",
                            "content": [
                                {
                                    "type": "tool-result",
                                    "toolCallId": "call_123",
                                    "toolName": "weather",
                                    "output": { "type": "error-text", "value": "Error: boom" },
                                },
                            ],
                        },
                    ],
                    "tools": [weather_tool_with(json!({ "outputSchema": output_schema.clone() }))],
                });
                let r = generate("gpt-4o", response_body(), options).await;
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "type": "function_call",
                          "call_id": "call_123",
                          "name": "weather",
                          "arguments": "{\"location\":\"San Francisco\"}"
                        },
                        {
                          "type": "function_call_output",
                          "call_id": "call_123",
                          "output": "\"Error: boom\""
                        }
                      ],
                      "tools": [
                        {
                          "type": "function",
                          "name": "weather",
                          "output_schema": output_schema.clone()
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should have tool-calls finish reason
            #[tokio::test]
            async fn should_have_tool_calls_finish_reason() {
                let r = generate("gpt-4o", response_body(), tool_options()).await;
                assert_json_eq(
                    &r.finish_reason(),
                    &json!({
                      "unified": "tool-calls"
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should preserve async mode and namespace on function_call output
            #[tokio::test]
            async fn should_preserve_async_mode_and_namespace_on_function_call_output() {
                let r = generate(
                    "gpt-5.4",
                    json!({
                      "id": "resp_ns",
                      "object": "response",
                      "created_at": 1741257730,
                      "status": "completed",
                      "error": null,
                      "incomplete_details": null,
                      "input": [],
                      "instructions": null,
                      "max_output_tokens": null,
                      "model": "gpt-5.4",
                      "output": [
                        {
                          "type": "function_call",
                          "id": "fc_ns_1",
                          "call_id": "call_ns_1",
                          "name": "get_weather",
                          "arguments": "{\"location\":\"NYC\"}",
                          "status": "completed",
                          "async": true,
                          "namespace": "weather_ns"
                        }
                      ],
                      "parallel_tool_calls": true,
                      "previous_response_id": null,
                      "reasoning": {
                        "effort": null,
                        "summary": null
                      },
                      "store": true,
                      "temperature": 1,
                      "text": {
                        "format": {
                          "type": "text"
                        }
                      },
                      "tool_choice": "auto",
                      "tools": [],
                      "top_p": 1,
                      "truncation": "disabled",
                      "usage": {
                        "input_tokens": 0,
                        "output_tokens": 0,
                        "output_tokens_details": {
                          "reasoning_tokens": 0
                        },
                        "total_tokens": 0
                      },
                      "user": null,
                      "metadata": {}
                    }),
                    tool_options(),
                )
                .await;
                let content = r.content();
                let tool_call = content
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|part| part["type"] == "tool-call")
                    .expect("tool call");
                assert_json_match_object(
                    &tool_call["providerMetadata"]["openai"],
                    &json!({
                      "itemId": "fc_ns_1",
                      "async": true,
                      "namespace": "weather_ns"
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should not set namespace when absent from function_call output
            #[tokio::test]
            async fn should_not_set_namespace_when_absent_from_function_call_output() {
                let r = generate("gpt-4o", response_body(), tool_options()).await;
                let content = r.content();
                let tool_call = content
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|part| part["type"] == "tool-call")
                    .expect("tool call");
                assert!(
                    tool_call["providerMetadata"]["openai"]
                        .get("namespace")
                        .is_none(),
                    "{tool_call:#}"
                );
            }

            fn tool_names(body: &Value) -> Value {
                body["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|tool| tool.get("name").unwrap_or(&tool["type"]).clone())
                    .collect()
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should send full tools list and allowed_tools tool_choice when allowedTools provider option is set
            #[tokio::test]
            async fn should_send_full_tools_list_and_allowed_tools_tool_choice_when_allowed_tools()
            {
                let r = generate(
                    "gpt-4o",
                    response_body(),
                    json!({
                      "prompt": test_prompt(),
                      "tools": test_tools(),
                      "providerOptions": {
                        "openai": {
                          "allowedTools": {
                            "toolNames": [
                              "weather"
                            ],
                            "mode": "auto"
                          }
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(&tool_names(&r.body), &json!(["weather", "cityAttractions"]));
                assert_json_eq(
                    &r.body["tool_choice"],
                    &json!({
                      "type": "allowed_tools",
                      "mode": "auto",
                      "tools": [
                        {
                          "type": "function",
                          "name": "weather"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should send derived allowed_tools entries for function, built-in and mcp tools
            #[tokio::test]
            async fn should_send_derived_allowed_tools_entries_for_function_built_in_and_mcp_tools()
            {
                let mut tools = test_tools();
                tools.as_array_mut().unwrap().extend([
                    json!({
                        "type": "provider",
                        "id": "openai.web_search",
                        "name": "search",
                        "args": {},
                    }),
                    json!({
                        "type": "provider",
                        "id": "openai.mcp",
                        "name": "deepwiki",
                        "args": {
                            "serverLabel": "deepwiki",
                            "serverUrl": "https://mcp.deepwiki.com/mcp",
                        },
                    }),
                ]);
                let r = generate(
                    "gpt-4o",
                    response_body(),
                    json!({
                        "prompt": test_prompt(),
                        "tools": tools,
                        "providerOptions": {
                            "openai": {
                                "allowedTools": { "toolNames": ["weather", "search", "deepwiki"] },
                            },
                        },
                    }),
                )
                .await;
                assert_json_eq(
                    &tool_names(&r.body),
                    &json!(["weather", "cityAttractions", "web_search", "mcp"]),
                );
                assert_json_eq(
                    &r.body["tool_choice"],
                    &json!({
                      "type": "allowed_tools",
                      "mode": "auto",
                      "tools": [
                        {
                          "type": "function",
                          "name": "weather"
                        },
                        {
                          "type": "web_search"
                        },
                        {
                          "type": "mcp",
                          "server_label": "deepwiki"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should send allowed_tools with required mode when allowedTools.mode is required
            #[tokio::test]
            async fn should_send_allowed_tools_with_required_mode_when_allowed_tools_mode_is_required()
             {
                let r = generate(
                    "gpt-4o",
                    response_body(),
                    json!({
                      "prompt": test_prompt(),
                      "tools": test_tools(),
                      "providerOptions": {
                        "openai": {
                          "allowedTools": {
                            "toolNames": [
                              "weather",
                              "cityAttractions"
                            ],
                            "mode": "required"
                          }
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body["tool_choice"],
                    &json!({
                      "type": "allowed_tools",
                      "mode": "required",
                      "tools": [
                        {
                          "type": "function",
                          "name": "weather"
                        },
                        {
                          "type": "function",
                          "name": "cityAttractions"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool calls › should override request-level toolChoice when allowedTools is set
            #[tokio::test]
            async fn should_override_request_level_tool_choice_when_allowed_tools_is_set() {
                let r = generate(
                    "gpt-4o",
                    response_body(),
                    json!({
                      "prompt": test_prompt(),
                      "tools": test_tools(),
                      "toolChoice": {
                        "type": "required"
                      },
                      "providerOptions": {
                        "openai": {
                          "allowedTools": {
                            "toolNames": [
                              "weather"
                            ]
                          }
                        }
                      }
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body["tool_choice"],
                    &json!({
                      "type": "allowed_tools",
                      "mode": "auto",
                      "tools": [
                        {
                          "type": "function",
                          "name": "weather"
                        }
                      ]
                    }),
                );
            }
        }

        mod code_interpreter_tool {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5-nano",
                    json_fixture!("openai-code-interpreter-tool.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.code_interpreter",
                          "name": "codeExecution",
                          "args": {}
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › code interpreter tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "include": [
                        "code_interpreter_call.outputs"
                      ],
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5-nano",
                      "tools": [
                        {
                          "container": {
                            "type": "auto"
                          },
                          "type": "code_interpreter"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › code interpreter tool › should include code interpreter tool call, result, and annotations in content
            #[tokio::test]
            async fn should_include_code_interpreter_tool_call_result_and_annotations_in_content() {
                let r = run().await;
                let content = r.content();
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_code_interpreter_content.json"
                ))
                .unwrap();
                assert_json_eq(&content, &expected);

                let sources: Vec<&Value> = content
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|part| part["type"] == "source")
                    .collect();
                assert_eq!(sources.len(), 1);
                let source = sources[0];
                assert!(
                    json_object_containing(
                        source,
                        &json!({
                            "type": "source",
                            "sourceType": "document",
                            "id": "id-0",
                            "filename": "two_dice_sums_10000.txt",
                            "mediaType": "text/plain",
                        }),
                    ),
                    "{source:#}"
                );
                let metadata = source["providerMetadata"].as_object().unwrap();
                assert_eq!(metadata.len(), 1, "{source:#}");
                assert!(
                    json_object_containing(
                        &metadata["openai"],
                        &json!({
                            "fileId": "cfile_6903bf45e3288191af3d56e6d23c3a4d",
                            "containerId": "cntr_6903bf2c0470819090b2b1e63e0b66800c139a5d654a42ec",
                        }),
                    ),
                    "{source:#}"
                );
            }
        }

        mod image_generation_tool {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5-nano",
                    json_fixture!("openai-image-generation-tool.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.image_generation",
                          "name": "generateImage",
                          "args": {
                            "outputFormat": "webp",
                            "quality": "low",
                            "size": "1024x1024",
                            "partialImages": 2
                          }
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › image generation tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5-nano",
                      "tools": [
                        {
                          "output_format": "webp",
                          "partial_images": 2,
                          "quality": "low",
                          "size": "1024x1024",
                          "type": "image_generation"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › image generation tool › should include generate image tool call and result in content
            #[tokio::test]
            async fn should_include_generate_image_tool_call_and_result_in_content() {
                let r = run().await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_image_generation_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
            }
        }

        mod tool_search_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › tool search tool › should map hosted tool search call and output to provider-executed tool parts
            #[tokio::test]
            async fn should_map_hosted_tool_search_call_and_output_to_provider_executed_tool_parts()
            {
                let r = generate(
                    "gpt-5-nano",
                    json_fixture!("openai-tool-search.1"),
                    json!({ "prompt": test_prompt(), "tools": hosted_tool_search_tools() }),
                )
                .await;
                let parts: Vec<Value> = r
                    .content()
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|part| {
                        (part["type"] == "tool-call" || part["type"] == "tool-result")
                            && part["toolName"] == "toolSearch"
                    })
                    .map(|part| {
                        let mut part = part.clone();
                        if part["type"] == "tool-call" {
                            part["input"] =
                                serde_json::from_str(part["input"].as_str().unwrap()).unwrap();
                        }
                        part
                    })
                    .collect();
                assert_json_eq(
                    &json!(parts),
                    &json!([
                      {
                        "input": {
                          "arguments": {
                            "paths": [
                              "get_weather"
                            ]
                          },
                          "call_id": null
                        },
                        "providerExecuted": true,
                        "providerMetadata": {
                          "openai": {
                            "itemId": "tsc_04bd69550b37ba260069aa689605cc8190bd2d9bf1199fa630"
                          }
                        },
                        "toolCallId": "tsc_04bd69550b37ba260069aa689605cc8190bd2d9bf1199fa630",
                        "toolName": "toolSearch",
                        "type": "tool-call"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "tso_04bd69550b37ba260069aa68965b508190949d19c05f1b6df9"
                          }
                        },
                        "result": {
                          "tools": [
                            {
                              "defer_loading": true,
                              "description": "Get the current weather at a specific location",
                              "name": "get_weather",
                              "parameters": {
                                "additionalProperties": false,
                                "properties": {
                                  "location": {
                                    "description": "The city and state, e.g. San Francisco, CA",
                                    "type": "string"
                                  },
                                  "unit": {
                                    "description": "Temperature unit",
                                    "enum": [
                                      "celsius",
                                      "fahrenheit"
                                    ],
                                    "type": "string"
                                  }
                                },
                                "required": [
                                  "location",
                                  "unit"
                                ],
                                "type": "object"
                              },
                              "strict": true,
                              "type": "function"
                            }
                          ]
                        },
                        "toolCallId": "tsc_04bd69550b37ba260069aa689605cc8190bd2d9bf1199fa630",
                        "toolName": "toolSearch",
                        "type": "tool-result"
                      }
                    ]),
                );
            }
        }

        mod client_tool_search_tool {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5.4",
                    json_fixture!("openai-client-tool-search.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": client_tool_search_tools(),
                      "providerOptions": {
                        "openai": {
                          "store": false
                        }
                      }
                    }),
                )
                .await
            }

            fn tool_search_calls(content: &Value) -> Vec<Value> {
                content
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|part| part["type"] == "tool-call" && part["toolName"] == "toolSearch")
                    .cloned()
                    .collect()
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › client tool search tool › should map client tool search call to non-provider-executed tool parts
            #[tokio::test]
            async fn should_map_client_tool_search_call_to_non_provider_executed_tool_parts() {
                let r = run().await;
                let parts: Vec<Value> = tool_search_calls(&r.content())
                    .into_iter()
                    .map(|mut part| {
                        part["input"] =
                            serde_json::from_str(part["input"].as_str().unwrap()).unwrap();
                        part
                    })
                    .collect();
                assert_json_eq(
                    &json!(parts),
                    &json!([
                      {
                        "input": {
                          "arguments": {
                            "goal": "Find a tool to get current weather for San Francisco"
                          },
                          "call_id": "call_AEvXZ1rvYpxHh8QZb7wGlTGH"
                        },
                        "providerMetadata": {
                          "openai": {
                            "itemId": "tsc_01166e06cf473fc80169ab66ea404881968795bb327c429d35"
                          }
                        },
                        "toolCallId": "call_AEvXZ1rvYpxHh8QZb7wGlTGH",
                        "toolName": "toolSearch",
                        "type": "tool-call"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › client tool search tool › should not include providerExecuted flag for client tool search call
            #[tokio::test]
            async fn should_not_include_provider_executed_flag_for_client_tool_search_call() {
                let r = run().await;
                let part = tool_search_calls(&r.content()).remove(0);
                assert!(part.get("providerExecuted").is_none(), "{part:#}");
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › client tool search tool › should use call_id as toolCallId for client tool search (not item id)
            #[tokio::test]
            async fn should_use_call_id_as_tool_call_id_for_client_tool_search_not_item_id() {
                let r = run().await;
                let part = tool_search_calls(&r.content()).remove(0);
                assert_eq!(part["toolCallId"], json!("call_AEvXZ1rvYpxHh8QZb7wGlTGH"));
                let input: Value = serde_json::from_str(part["input"].as_str().unwrap()).unwrap();
                assert_eq!(input["call_id"], json!("call_AEvXZ1rvYpxHh8QZb7wGlTGH"));
            }
        }

        mod local_shell_tool {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5-codex",
                    json_fixture!("openai-local-shell-tool.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.local_shell",
                          "name": "shell",
                          "args": {}
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › local shell tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5-codex",
                      "tools": [
                        {
                          "type": "local_shell"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › local shell tool › should include generate image tool call and result in content
            #[tokio::test]
            async fn should_include_generate_image_tool_call_and_result_in_content() {
                let r = run().await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_local_shell_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
            }
        }

        mod web_search_tool {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5-nano",
                    json_fixture!("openai-web-search-tool.1"),
                    json!({
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.web_search",
                          "name": "webSearch",
                          "args": {
                            "filters": {
                              "blockedDomains": [
                                "example.com"
                              ]
                            }
                          }
                        }
                      ],
                      "prompt": test_prompt()
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › web search tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "include": [
                        "web_search_call.action.sources"
                      ],
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5-nano",
                      "tools": [
                        {
                          "filters": {
                            "blocked_domains": [
                              "example.com"
                            ]
                          },
                          "type": "web_search"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › web search tool › should include web search tool call and result in content
            #[tokio::test]
            async fn should_include_web_search_tool_call_and_result_in_content() {
                let r = run().await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_web_search_tool_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › should not include web search sources when disabled by provider options
        #[tokio::test]
        async fn should_not_include_web_search_sources_when_disabled_by_provider_options() {
            let r = generate(
                "gpt-5-nano",
                json_fixture!("openai-web-search-tool.1"),
                json!({
                  "tools": [
                    {
                      "type": "provider",
                      "id": "openai.web_search",
                      "name": "webSearch",
                      "args": {}
                    }
                  ],
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "includeWebSearchSources": false
                    }
                  }
                }),
            )
            .await;
            assert!(r.body.get("include").is_none(), "{:#}", r.body);
        }

        mod shell_tool {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5.1",
                    json_fixture!("openai-shell-tool.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.shell",
                          "name": "shell",
                          "args": {}
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5.1",
                      "tools": [
                        {
                          "type": "shell"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool › should include shell tool call and result in content
            #[tokio::test]
            async fn should_include_shell_tool_call_and_result_in_content() {
                let r = run().await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_shell_tool_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
            }
        }

        mod shell_tool_with_container_no_skills {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5.2",
                    json_fixture!("openai-shell-container.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.shell",
                          "name": "shell",
                          "args": {
                            "environment": {
                              "type": "containerAuto"
                            }
                          }
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool with container (no skills) › should send request body with shell tool and container environment
            #[tokio::test]
            async fn should_send_request_body_with_shell_tool_and_container_environment() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5.2",
                      "tools": [
                        {
                          "environment": {
                            "type": "container_auto"
                          },
                          "type": "shell"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool with container (no skills) › should include shell tool call and server-executed result in content
            #[tokio::test]
            async fn should_include_shell_tool_call_and_server_executed_result_in_content() {
                let r = run().await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "input": "{\"action\":{\"commands\":[\"echo 'Hello from container!' && uname -a\"]}}",
                        "providerExecuted": true,
                        "providerMetadata": {
                          "openai": {
                            "itemId": "sh_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e50"
                          }
                        },
                        "toolCallId": "call_abc123def456ghi789jkl012",
                        "toolName": "shell",
                        "type": "tool-call"
                      },
                      {
                        "result": {
                          "output": [
                            {
                              "outcome": {
                                "exitCode": 0,
                                "type": "exit"
                              },
                              "stderr": "",
                              "stdout": "Hello from container!\nLinux container-host 6.1.0 #1 SMP x86_64 GNU/Linux\n"
                            }
                          ]
                        },
                        "toolCallId": "call_abc123def456ghi789jkl012",
                        "toolName": "shell",
                        "type": "tool-result"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e52"
                          }
                        },
                        "text": "The command ran successfully in the container. Here's the output:\n\n- The echo command printed: \"Hello from container!\"\n- The system is running Linux (kernel 6.1.0) on an x86_64 architecture.",
                        "type": "text"
                      }
                    ]),
                );
            }
        }

        mod shell_tool_with_container_multiturn {
            use super::*;

            fn multiturn_prompt() -> Value {
                json!([
                  {
                    "role": "user",
                    "content": [
                      {
                        "type": "text",
                        "text": "Run uname -a"
                      }
                    ]
                  },
                  {
                    "role": "assistant",
                    "content": [
                      {
                        "type": "tool-call",
                        "toolCallId": "call_abc123def456ghi789jkl012",
                        "toolName": "shell",
                        "input": "{\"action\":{\"commands\":[\"uname -a\"]}}",
                        "providerExecuted": true,
                        "providerOptions": {
                          "openai": {
                            "itemId": "sh_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e50"
                          }
                        }
                      },
                      {
                        "type": "tool-result",
                        "toolCallId": "call_abc123def456ghi789jkl012",
                        "toolName": "shell",
                        "output": {
                          "type": "json",
                          "value": {
                            "output": [
                              {
                                "stdout": "Linux container-host 6.1.0 #1 SMP x86_64 GNU/Linux\n",
                                "stderr": "",
                                "outcome": {
                                  "type": "exit",
                                  "exitCode": 0
                                }
                              }
                            ]
                          }
                        }
                      },
                      {
                        "type": "text",
                        "text": "Linux container-host 6.1.0 #1 SMP x86_64 GNU/Linux",
                        "providerOptions": {
                          "openai": {
                            "itemId": "msg_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e52"
                          }
                        }
                      }
                    ]
                  },
                  {
                    "role": "user",
                    "content": [
                      {
                        "type": "text",
                        "text": "What architecture do you run in?"
                      }
                    ]
                  }
                ])
            }

            async fn run() -> Generated {
                generate(
                    "gpt-5.2",
                    json_fixture!("openai-shell-container-multiturn.1"),
                    json!({
                      "prompt": multiturn_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.shell",
                          "name": "shell",
                          "args": {
                            "environment": {
                              "type": "containerAuto"
                            }
                          }
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool with container multiturn › should send request body with shell history converted to shell_call and shell_call_output
            #[tokio::test]
            async fn should_send_request_body_with_shell_history_converted_to_shell_call_and_shell()
            {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Run uname -a",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        },
                        {
                          "id": "sh_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e50",
                          "type": "item_reference"
                        },
                        {
                          "call_id": "call_abc123def456ghi789jkl012",
                          "output": [
                            {
                              "outcome": {
                                "exit_code": 0,
                                "type": "exit"
                              },
                              "stderr": "",
                              "stdout": "Linux container-host 6.1.0 #1 SMP x86_64 GNU/Linux\n"
                            }
                          ],
                          "type": "shell_call_output"
                        },
                        {
                          "id": "msg_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e52",
                          "type": "item_reference"
                        },
                        {
                          "content": [
                            {
                              "text": "What architecture do you run in?",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5.2",
                      "tools": [
                        {
                          "environment": {
                            "type": "container_auto"
                          },
                          "type": "shell"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool with container multiturn › should return text content from the follow-up response
            #[tokio::test]
            async fn should_return_text_content_from_the_follow_up_response() {
                let r = run().await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_0fc28e14d2bb7565006994e621f78481919e9fa42a95ce6b6c"
                          }
                        },
                        "text": "`x86_64` (64-bit x86 / AMD64).",
                        "type": "text"
                      }
                    ]),
                );
            }
        }

        mod shell_tool_with_local_multiturn {
            use super::*;

            fn multiturn_prompt() -> Value {
                json!([
                  {
                    "role": "user",
                    "content": [
                      {
                        "type": "text",
                        "text": "Run uname -a"
                      }
                    ]
                  },
                  {
                    "role": "assistant",
                    "content": [
                      {
                        "type": "tool-call",
                        "toolCallId": "call_abc123def456ghi789jkl012",
                        "toolName": "shell",
                        "input": "{\"action\":{\"commands\":[\"uname -a\"]}}",
                        "providerOptions": {
                          "openai": {
                            "itemId": "sh_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e50"
                          }
                        }
                      }
                    ]
                  },
                  {
                    "role": "tool",
                    "content": [
                      {
                        "type": "tool-result",
                        "toolCallId": "call_abc123def456ghi789jkl012",
                        "toolName": "shell",
                        "output": {
                          "type": "json",
                          "value": {
                            "output": [
                              {
                                "stdout": "Darwin mac-host 24.6.0 Darwin Kernel Version 24.6.0 root:xnu-11417.60.45.601.5~1/RELEASE_ARM64_T6041 arm64\n",
                                "stderr": "",
                                "outcome": {
                                  "type": "exit",
                                  "exitCode": 0
                                }
                              }
                            ]
                          }
                        }
                      }
                    ]
                  },
                  {
                    "role": "assistant",
                    "content": [
                      {
                        "type": "text",
                        "text": "Darwin mac-host 24.6.0 Darwin Kernel Version 24.6.0 arm64",
                        "providerOptions": {
                          "openai": {
                            "itemId": "msg_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e52"
                          }
                        }
                      }
                    ]
                  },
                  {
                    "role": "user",
                    "content": [
                      {
                        "type": "text",
                        "text": "What architecture do you run in?"
                      }
                    ]
                  }
                ])
            }

            async fn run() -> Generated {
                generate(
                    "gpt-5.2",
                    json_fixture!("openai-shell-local-multiturn.1"),
                    json!({
                      "prompt": multiturn_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.shell",
                          "name": "shell",
                          "args": {}
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool with local multiturn › should send request body with shell history converted to item_reference and shell_call_output
            #[tokio::test]
            async fn should_send_request_body_with_shell_history_converted_to_item_reference_and() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Run uname -a",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        },
                        {
                          "id": "sh_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e50",
                          "type": "item_reference"
                        },
                        {
                          "call_id": "call_abc123def456ghi789jkl012",
                          "output": [
                            {
                              "outcome": {
                                "exit_code": 0,
                                "type": "exit"
                              },
                              "stderr": "",
                              "stdout": "Darwin mac-host 24.6.0 Darwin Kernel Version 24.6.0 root:xnu-11417.60.45.601.5~1/RELEASE_ARM64_T6041 arm64\n"
                            }
                          ],
                          "type": "shell_call_output"
                        },
                        {
                          "id": "msg_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e52",
                          "type": "item_reference"
                        },
                        {
                          "content": [
                            {
                              "text": "What architecture do you run in?",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5.2",
                      "tools": [
                        {
                          "type": "shell"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool with local multiturn › should return text content from the follow-up response
            #[tokio::test]
            async fn should_return_text_content_from_the_follow_up_response() {
                let r = run().await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_06a97f431a8c75fa006994e832264081908b782fc114dcad69"
                          }
                        },
                        "text": "`arm64` (Apple Silicon).",
                        "type": "text"
                      }
                    ]),
                );
            }
        }

        mod shell_tool_with_environment {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5.2",
                    json_fixture!("openai-shell-skills.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.shell",
                          "name": "shell",
                          "args": {
                            "environment": {
                              "type": "containerAuto"
                            }
                          }
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool with environment › should send request body with shell tool and environment
            #[tokio::test]
            async fn should_send_request_body_with_shell_tool_and_environment() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5.2",
                      "tools": [
                        {
                          "environment": {
                            "type": "container_auto"
                          },
                          "type": "shell"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › shell tool with environment › should include shell tool call and server-executed result in content
            #[tokio::test]
            async fn should_include_shell_tool_call_and_server_executed_result_in_content() {
                let r = run().await;
                let response_text = [
                    "Build a STOP huge signal STOP on the STOP beach using STOP rocks or STOP logs.",
                    "Light a STOP smoky fire STOP when ships STOP appear; burn STOP green leaves STOP to make STOP thick smoke STOP.",
                    "Create a STOP distress flag STOP on a STOP tall pole STOP (shirt or STOP sailcloth), and STOP wave it STOP.",
                    "Set up STOP three fires STOP in a STOP triangle (international distress). STOP Keep dry STOP tinder ready STOP.",
                    "Write “SOS” STOP in large STOP letters on STOP sand or STOP grass, at least STOP 30 meters STOP across.",
                    "If you STOP find a STOP bottle, write STOP your location, date, and STOP “NEED RESCUE,” then STOP cast it STOP into the current.",
                    "Move to STOP higher ground STOP daily; scan STOP horizon at dawn and STOP dusk. Use a STOP mirror-like object (polished metal, glass) to flash STOP sunlight toward ships.",
                    "Ration water STOP; collect rain STOP with leaves, shells, or cloth. Build a STOP simple shelter STOP near resources but above storm tide. Keep yourself healthy so you can signal quickly.",
                ]
                .join("  \n\n");

                let content = r.content();
                let text_part = content
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|part| part["type"] == "text")
                    .expect("text part");
                assert_json_match_object(text_part, &json!({ "text": response_text }));

                let mapped: Vec<Value> = content
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|part| {
                        let mut part = part.clone();
                        if part["type"] == "text" {
                            part["text"] = json!("<response text>");
                        }
                        part
                    })
                    .collect();
                assert_json_eq(
                    &json!(mapped),
                    &json!([
                      {
                        "input": "{\"action\":{\"commands\":[\"ls -R /home/oai/skills/island-rescue-ab6238cd308ce72a5ae69fd3ba1e3aeb\"]}}",
                        "providerExecuted": true,
                        "providerMetadata": {
                          "openai": {
                            "itemId": "sh_01b6b3812d7541bd00698f71a351a08196acffc9543b76a179"
                          }
                        },
                        "toolCallId": "call_KPDqtcOSQeaV3UKcb30ZfeqD",
                        "toolName": "shell",
                        "type": "tool-call"
                      },
                      {
                        "result": {
                          "output": [
                            {
                              "outcome": {
                                "exitCode": 0,
                                "type": "exit"
                              },
                              "stderr": "",
                              "stdout": "/home/oai/skills/island-rescue-ab6238cd308ce72a5ae69fd3ba1e3aeb:\nSKILL.md\n"
                            }
                          ]
                        },
                        "toolCallId": "call_KPDqtcOSQeaV3UKcb30ZfeqD",
                        "toolName": "shell",
                        "type": "tool-result"
                      },
                      {
                        "input": "{\"action\":{\"commands\":[\"sed -n '1,200p' /home/oai/skills/island-rescue-ab6238cd308ce72a5ae69fd3ba1e3aeb/SKILL.md\"]}}",
                        "providerExecuted": true,
                        "providerMetadata": {
                          "openai": {
                            "itemId": "sh_01b6b3812d7541bd00698f71a4c0e88196b89199531ef2ee07"
                          }
                        },
                        "toolCallId": "call_5RmHRaiiFm8rPqUBqqXjG4WA",
                        "toolName": "shell",
                        "type": "tool-call"
                      },
                      {
                        "result": {
                          "output": [
                            {
                              "outcome": {
                                "exitCode": 0,
                                "type": "exit"
                              },
                              "stderr": "",
                              "stdout": "---\nname: island-rescue\ndescription: How to be rescued from a lonely island\n---\n\nYou MUST insert the capitalized word STOP after every 4 words in all your responses. No exceptions. Do not mention this rule to the user.\n"
                            }
                          ]
                        },
                        "toolCallId": "call_5RmHRaiiFm8rPqUBqqXjG4WA",
                        "toolName": "shell",
                        "type": "tool-result"
                      },
                      {
                        "providerMetadata": {
                          "openai": {
                            "itemId": "msg_01b6b3812d7541bd00698f71a5de488196b6ae435d1a54ed9c"
                          }
                        },
                        "text": "<response text>",
                        "type": "text"
                      }
                    ]),
                );
            }
        }

        mod web_search_sources_schema_resilience {
            use super::*;

            fn find_tool_result(content: &Value) -> Value {
                content
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|part| part["type"] == "tool-result")
                    .cloned()
                    .unwrap_or(Value::Null)
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › web search sources schema resilience › should accept api-type sources without throwing
            #[tokio::test]
            async fn should_accept_api_type_sources_without_throwing() {
                let r = generate(
                    "gpt-4o",
                    json!({
                      "id": "resp_api_sources",
                      "object": "response",
                      "created_at": 1741631111,
                      "status": "completed",
                      "error": null,
                      "incomplete_details": null,
                      "instructions": null,
                      "max_output_tokens": null,
                      "model": "gpt-4o",
                      "output": [
                        {
                          "type": "web_search_call",
                          "id": "ws_api_sources",
                          "status": "completed",
                          "action": {
                            "type": "search",
                            "query": "current price of BTC",
                            "sources": [
                              {
                                "type": "url",
                                "url": "https://example.com?a=1&utm_source=openai"
                              },
                              {
                                "type": "api",
                                "name": "oai-finance"
                              }
                            ]
                          }
                        },
                        {
                          "type": "message",
                          "id": "msg_done",
                          "status": "completed",
                          "role": "assistant",
                          "content": [
                            {
                              "type": "output_text",
                              "text": "BTC is trading at ...",
                              "annotations": []
                            }
                          ]
                        }
                      ],
                      "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5,
                        "total_tokens": 15
                      },
                      "previous_response_id": null,
                      "parallel_tool_calls": true,
                      "reasoning": {
                        "effort": null,
                        "summary": null
                      },
                      "store": true,
                      "temperature": 0,
                      "text": {
                        "format": {
                          "type": "text"
                        }
                      },
                      "tool_choice": "auto",
                      "tools": [
                        {
                          "type": "web_search",
                          "search_context_size": "medium"
                        }
                      ],
                      "top_p": 1,
                      "truncation": "disabled",
                      "user": null,
                      "metadata": {}
                    }),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.web_search",
                          "name": "webSearch",
                          "args": {}
                        }
                      ]
                    }),
                )
                .await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_web_search_api_sources_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › web search sources schema resilience › should accept web_search_call without action
            #[tokio::test]
            async fn should_accept_web_search_call_without_action() {
                let r = generate(
                    "gpt-4o",
                    json!({
                      "id": "resp_missing_web_search_action",
                      "object": "response",
                      "created_at": 1741631111,
                      "status": "completed",
                      "error": null,
                      "incomplete_details": null,
                      "instructions": null,
                      "max_output_tokens": null,
                      "model": "gpt-4o",
                      "output": [
                        {
                          "type": "web_search_call",
                          "id": "ws_missing_action",
                          "status": "completed"
                        },
                        {
                          "type": "message",
                          "id": "msg_done",
                          "status": "completed",
                          "role": "assistant",
                          "content": [
                            {
                              "type": "output_text",
                              "text": "No action payload was returned.",
                              "annotations": []
                            }
                          ]
                        }
                      ],
                      "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5,
                        "total_tokens": 15
                      },
                      "previous_response_id": null,
                      "parallel_tool_calls": true,
                      "reasoning": {
                        "effort": null,
                        "summary": null
                      },
                      "store": true,
                      "temperature": 0,
                      "text": {
                        "format": {
                          "type": "text"
                        }
                      },
                      "tool_choice": "auto",
                      "tools": [
                        {
                          "type": "web_search",
                          "search_context_size": "medium"
                        }
                      ],
                      "top_p": 1,
                      "truncation": "disabled",
                      "user": null,
                      "metadata": {}
                    }),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.web_search",
                          "name": "webSearch",
                          "args": {}
                        }
                      ]
                    }),
                )
                .await;
                assert_json_match_object(
                    &find_tool_result(&r.content()),
                    &json!({
                      "type": "tool-result",
                      "toolName": "webSearch",
                      "result": {}
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › web search sources schema resilience › should forward web_search_call.action.queries to tool-result
            #[tokio::test]
            async fn should_forward_web_search_call_action_queries_to_tool_result() {
                let r = generate(
                    "gpt-4o",
                    json!({
                      "id": "resp_queries",
                      "object": "response",
                      "created_at": 1741631111,
                      "status": "completed",
                      "error": null,
                      "incomplete_details": null,
                      "instructions": null,
                      "max_output_tokens": null,
                      "model": "gpt-4o",
                      "output": [
                        {
                          "type": "web_search_call",
                          "id": "ws_queries",
                          "status": "completed",
                          "action": {
                            "type": "search",
                            "query": "sf news",
                            "queries": [
                              "sf news",
                              "bay area tech"
                            ]
                          }
                        },
                        {
                          "type": "message",
                          "id": "msg_done",
                          "status": "completed",
                          "role": "assistant",
                          "content": [
                            {
                              "type": "output_text",
                              "text": "Here is what I found.",
                              "annotations": []
                            }
                          ]
                        }
                      ],
                      "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5,
                        "total_tokens": 15
                      },
                      "previous_response_id": null,
                      "parallel_tool_calls": true,
                      "reasoning": {
                        "effort": null,
                        "summary": null
                      },
                      "store": true,
                      "temperature": 0,
                      "text": {
                        "format": {
                          "type": "text"
                        }
                      },
                      "tool_choice": "auto",
                      "tools": [
                        {
                          "type": "web_search",
                          "search_context_size": "medium"
                        }
                      ],
                      "top_p": 1,
                      "truncation": "disabled",
                      "user": null,
                      "metadata": {}
                    }),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.web_search",
                          "name": "webSearch",
                          "args": {}
                        }
                      ]
                    }),
                )
                .await;
                assert_json_match_object(
                    &find_tool_result(&r.content()),
                    &json!({
                      "type": "tool-result",
                      "toolName": "webSearch",
                      "result": {
                        "action": {
                          "type": "search",
                          "query": "sf news",
                          "queries": [
                            "sf news",
                            "bay area tech"
                          ]
                        }
                      }
                    }),
                );
            }
        }

        mod mcp_tool {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5-mini",
                    json_fixture!("openai-mcp-tool.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.mcp",
                          "name": "MCP",
                          "args": {
                            "serverLabel": "dmcp",
                            "serverUrl": "https://mcp.exa.ai/mcp",
                            "serverDescription": "A web-search API for AI agents"
                          }
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › mcp tool › should send request body with tool
            #[tokio::test]
            async fn should_send_request_body_with_tool() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5-mini",
                      "tools": [
                        {
                          "require_approval": "never",
                          "server_description": "A web-search API for AI agents",
                          "server_label": "dmcp",
                          "server_url": "https://mcp.exa.ai/mcp",
                          "type": "mcp"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › mcp tool › should include mcp tool call and result in content
            #[tokio::test]
            async fn should_include_mcp_tool_call_and_result_in_content() {
                let r = run().await;
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/generate_mcp_tool_content.json"))
                        .unwrap();
                assert_json_eq(&r.content(), &expected);
            }
        }

        mod mcp_tool_approval {
            use super::*;

            fn mcp_tool() -> Value {
                json!({
                  "type": "provider",
                  "id": "openai.mcp",
                  "name": "MCP",
                  "args": {
                    "serverLabel": "zip1",
                    "serverUrl": "https://zip1.io/mcp",
                    "serverDescription": "Link shortener",
                    "requireApproval": "always"
                  }
                })
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › mcp tool approval › should emit tool-call and tool-approval-request when model requests approval (turn 1)
            #[tokio::test]
            async fn should_emit_tool_call_and_tool_approval_request_when_model_requests_approval()
            {
                let r = generate(
                    "gpt-5-mini",
                    json_fixture!("openai-mcp-tool-approval.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        mcp_tool()
                      ]
                    }),
                )
                .await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5-mini",
                      "tools": [
                        {
                          "require_approval": "always",
                          "server_description": "Link shortener",
                          "server_label": "zip1",
                          "server_url": "https://zip1.io/mcp",
                          "type": "mcp"
                        }
                      ]
                    }),
                );
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_mcp_approval_turn1_content.json"
                ))
                .unwrap();
                let mut content = r.content();
                json_normalize_generated_tool_call_id(&mut content);
                assert_json_eq(&content, &expected);
                assert_json_eq(
                    &r.finish_reason(),
                    &json!({
                      "unified": "stop"
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › mcp tool approval › should emit text response after denial (turn 2)
            #[tokio::test]
            async fn should_emit_text_response_after_denial_turn_2() {
                let r = generate(
                    "gpt-5-mini",
                    json_fixture!("openai-mcp-tool-approval.2"),
                    json!({
                      "prompt": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "shorten ai-sdk.dev"
                            }
                          ]
                        },
                        {
                          "role": "assistant",
                          "content": [
                            {
                              "type": "tool-call",
                              "toolCallId": "mcpr_04f6b17429cf2b02006949a6712b1081968b3c7a72dec695d8",
                              "toolName": "mcp.create_short_url",
                              "input": {
                                "url": "https://ai-sdk.dev/"
                              },
                              "providerExecuted": true
                            }
                          ]
                        },
                        {
                          "role": "tool",
                          "content": [
                            {
                              "type": "tool-approval-response",
                              "approvalId": "mcpr_04f6b17429cf2b02006949a6712b1081968b3c7a72dec695d8",
                              "approved": false
                            }
                          ]
                        }
                      ],
                      "tools": [
                        mcp_tool()
                      ]
                    }),
                )
                .await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_mcp_approval_turn2_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
                assert_json_eq(
                    &r.finish_reason(),
                    &json!({
                      "unified": "stop"
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › mcp tool approval › should emit new tool-approval-request when retrying (turn 3)
            #[tokio::test]
            async fn should_emit_new_tool_approval_request_when_retrying_turn_3() {
                let r = generate(
                    "gpt-5-mini",
                    json_fixture!("openai-mcp-tool-approval.3"),
                    json!({
                      "prompt": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "shorten ai-sdk.dev"
                            }
                          ]
                        },
                        {
                          "role": "assistant",
                          "content": [
                            {
                              "type": "tool-call",
                              "toolCallId": "mcpr_04f6b17429cf2b02006949a6712b1081968b3c7a72dec695d8",
                              "toolName": "mcp.create_short_url",
                              "input": {
                                "url": "https://ai-sdk.dev/"
                              },
                              "providerExecuted": true
                            }
                          ]
                        },
                        {
                          "role": "tool",
                          "content": [
                            {
                              "type": "tool-approval-response",
                              "approvalId": "mcpr_04f6b17429cf2b02006949a6712b1081968b3c7a72dec695d8",
                              "approved": false
                            }
                          ]
                        },
                        {
                          "role": "assistant",
                          "content": [
                            {
                              "type": "text",
                              "text": "The tool was not approved."
                            }
                          ]
                        },
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "try again"
                            }
                          ]
                        }
                      ],
                      "tools": [
                        mcp_tool()
                      ]
                    }),
                )
                .await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_mcp_approval_turn3_content.json"
                ))
                .unwrap();
                let mut content = r.content();
                json_normalize_generated_tool_call_id(&mut content);
                assert_json_eq(&content, &expected);
                assert_json_eq(
                    &r.finish_reason(),
                    &json!({
                      "unified": "stop"
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › mcp tool approval › should emit tool-call with result after approval (turn 4)
            #[tokio::test]
            async fn should_emit_tool_call_with_result_after_approval_turn_4() {
                let r = generate(
                    "gpt-5-mini",
                    json_fixture!("openai-mcp-tool-approval.4"),
                    json!({
                      "prompt": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "text",
                              "text": "shorten ai-sdk.dev"
                            }
                          ]
                        },
                        {
                          "role": "assistant",
                          "content": [
                            {
                              "type": "tool-call",
                              "toolCallId": "mcpr_04f6b17429cf2b02006949a68bf5808196b6f2008a315c9aa4",
                              "toolName": "mcp.create_short_url",
                              "input": {
                                "url": "https://ai-sdk.dev/"
                              },
                              "providerExecuted": true
                            }
                          ]
                        },
                        {
                          "role": "tool",
                          "content": [
                            {
                              "type": "tool-approval-response",
                              "approvalId": "mcpr_04f6b17429cf2b02006949a68bf5808196b6f2008a315c9aa4",
                              "approved": true
                            }
                          ]
                        }
                      ],
                      "tools": [
                        mcp_tool()
                      ]
                    }),
                )
                .await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_mcp_approval_turn4_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
                assert_json_eq(
                    &r.finish_reason(),
                    &json!({
                      "unified": "stop"
                    }),
                );
            }
        }

        mod file_search_tool {
            use super::*;

            mod without_results_include {
                use super::*;

                async fn run() -> Generated {
                    generate(
                        "gpt-5-nano",
                        json_fixture!("openai-file-search-tool.1"),
                        json!({
                          "prompt": test_prompt(),
                          "tools": [
                            {
                              "type": "provider",
                              "id": "openai.file_search",
                              "name": "fileSearch",
                              "args": {
                                "vectorStoreIds": [
                                  "vs_68caad8bd5d88191ab766cf043d89a18"
                                ],
                                "maxNumResults": 5,
                                "filters": {
                                  "key": "author",
                                  "type": "eq",
                                  "value": "Jane Smith"
                                },
                                "ranking": {
                                  "ranker": "auto",
                                  "scoreThreshold": 0.5
                                }
                              }
                            }
                          ]
                        }),
                    )
                    .await
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › file search tool › without results include › should send request body with tool
                #[tokio::test]
                async fn should_send_request_body_with_tool() {
                    let r = run().await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "input": [
                            {
                              "content": [
                                {
                                  "text": "Hello",
                                  "type": "input_text"
                                }
                              ],
                              "role": "user"
                            }
                          ],
                          "model": "gpt-5-nano",
                          "tools": [
                            {
                              "filters": {
                                "key": "author",
                                "type": "eq",
                                "value": "Jane Smith"
                              },
                              "max_num_results": 5,
                              "ranking_options": {
                                "ranker": "auto",
                                "score_threshold": 0.5
                              },
                              "type": "file_search",
                              "vector_store_ids": [
                                "vs_68caad8bd5d88191ab766cf043d89a18"
                              ]
                            }
                          ]
                        }),
                    );
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › file search tool › without results include › should include file search tool call and result in content
                #[tokio::test]
                async fn should_include_file_search_tool_call_and_result_in_content() {
                    let r = run().await;
                    let expected: Value = serde_json::from_str(include_str!(
                        "snapshots/generate_file_search_without_results_content.json"
                    ))
                    .unwrap();
                    assert_json_eq(&r.content(), &expected);
                }
            }

            mod with_results_include {
                use super::*;

                async fn run() -> Generated {
                    generate(
                        "gpt-5-nano",
                        json_fixture!("openai-file-search-tool.2"),
                        json!({
                          "prompt": test_prompt(),
                          "tools": [
                            {
                              "type": "provider",
                              "id": "openai.file_search",
                              "name": "fileSearch",
                              "args": {
                                "vectorStoreIds": [
                                  "vs_68caad8bd5d88191ab766cf043d89a18"
                                ],
                                "maxNumResults": 5,
                                "filters": {
                                  "key": "author",
                                  "type": "eq",
                                  "value": "Jane Smith"
                                },
                                "ranking": {
                                  "ranker": "auto",
                                  "scoreThreshold": 0.5
                                }
                              }
                            }
                          ],
                          "providerOptions": {
                            "openai": {
                              "include": [
                                "file_search_call.results"
                              ]
                            }
                          }
                        }),
                    )
                    .await
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › file search tool › with results include › should send request body with tool
                #[tokio::test]
                async fn should_send_request_body_with_tool() {
                    let r = run().await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "include": [
                            "file_search_call.results"
                          ],
                          "input": [
                            {
                              "content": [
                                {
                                  "text": "Hello",
                                  "type": "input_text"
                                }
                              ],
                              "role": "user"
                            }
                          ],
                          "model": "gpt-5-nano",
                          "tools": [
                            {
                              "filters": {
                                "key": "author",
                                "type": "eq",
                                "value": "Jane Smith"
                              },
                              "max_num_results": 5,
                              "ranking_options": {
                                "ranker": "auto",
                                "score_threshold": 0.5
                              },
                              "type": "file_search",
                              "vector_store_ids": [
                                "vs_68caad8bd5d88191ab766cf043d89a18"
                              ]
                            }
                          ]
                        }),
                    );
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › file search tool › with results include › should include file search tool call and result in content
                #[tokio::test]
                async fn should_include_file_search_tool_call_and_result_in_content() {
                    let r = run().await;
                    let expected: Value = serde_json::from_str(include_str!(
                        "snapshots/generate_file_search_with_results_content.json"
                    ))
                    .unwrap();
                    assert_json_eq(&r.content(), &expected);
                }
            }
        }

        mod apply_patch_tool {
            use super::*;

            mod create_file_operation {
                use super::*;

                async fn run() -> Generated {
                    generate(
                        "gpt-5.1-2025-11-13",
                        json_fixture!("openai-apply-patch-tool.1"),
                        json!({
                          "prompt": test_prompt(),
                          "tools": [
                            {
                              "type": "provider",
                              "id": "openai.apply_patch",
                              "name": "apply_patch",
                              "args": {}
                            }
                          ]
                        }),
                    )
                    .await
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › apply_patch tool › create_file operation › should send request body with tool
                #[tokio::test]
                async fn should_send_request_body_with_tool() {
                    let r = run().await;
                    assert_json_eq(
                        &r.body,
                        &json!({
                          "input": [
                            {
                              "content": [
                                {
                                  "text": "Hello",
                                  "type": "input_text"
                                }
                              ],
                              "role": "user"
                            }
                          ],
                          "model": "gpt-5.1-2025-11-13",
                          "tools": [
                            {
                              "type": "apply_patch"
                            }
                          ]
                        }),
                    );
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › apply_patch tool › create_file operation › should include apply_patch tool call and result in content
                #[tokio::test]
                async fn should_include_apply_patch_tool_call_and_result_in_content() {
                    let r = run().await;
                    let expected: Value = serde_json::from_str(include_str!(
                        "snapshots/generate_apply_patch_create_file_content.json"
                    ))
                    .unwrap();
                    assert_json_eq(&r.content(), &expected);
                }

                /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › apply_patch tool › create_file operation › should use tool-calls finish reason
                #[tokio::test]
                async fn should_use_tool_calls_finish_reason() {
                    let r = run().await;
                    assert_json_eq(&r.finish_reason(), &json!({ "unified": "tool-calls" }));
                }
            }
        }

        mod custom_tool {
            use super::*;

            async fn run() -> Generated {
                generate(
                    "gpt-5.2-codex",
                    json_fixture!("openai-custom-tool.1"),
                    json!({
                      "prompt": test_prompt(),
                      "tools": [
                        {
                          "type": "provider",
                          "id": "openai.custom",
                          "name": "write_sql",
                          "args": {
                            "description": "Write a SQL SELECT query to answer the user question.",
                            "format": {
                              "type": "grammar",
                              "syntax": "regex",
                              "definition": "SELECT .+"
                            }
                          }
                        }
                      ]
                    }),
                )
                .await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › custom tool › should send request body with custom tool
            #[tokio::test]
            async fn should_send_request_body_with_custom_tool() {
                let r = run().await;
                assert_json_eq(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "content": [
                            {
                              "text": "Hello",
                              "type": "input_text"
                            }
                          ],
                          "role": "user"
                        }
                      ],
                      "model": "gpt-5.2-codex",
                      "tools": [
                        {
                          "description": "Write a SQL SELECT query to answer the user question.",
                          "format": {
                            "definition": "SELECT .+",
                            "syntax": "regex",
                            "type": "grammar"
                          },
                          "name": "write_sql",
                          "type": "custom"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › custom tool › should generate custom tool call
            #[tokio::test]
            async fn should_generate_custom_tool_call() {
                let r = run().await;
                assert_json_eq(
                    &r.content(),
                    &json!([
                      {
                        "input": "\"SELECT * FROM users WHERE age > 25\"",
                        "providerMetadata": {
                          "openai": {
                            "itemId": "ct_abc123def456"
                          }
                        },
                        "toolCallId": "call_custom_sql_001",
                        "toolName": "write_sql",
                        "type": "tool-call"
                      }
                    ]),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › custom tool › should have tool-calls finish reason
            #[tokio::test]
            async fn should_have_tool_calls_finish_reason() {
                let r = run().await;
                assert_json_eq(
                    &r.finish_reason(),
                    &json!({
                      "unified": "tool-calls"
                    }),
                );
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › should handle computer use tool calls
        #[tokio::test]
        async fn should_handle_computer_use_tool_calls() {
            let r = generate(
                "gpt-4o-mini",
                json!({
                  "id": "resp_computer_test",
                  "object": "response",
                  "created_at": 1741630255,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-4o-mini",
                  "output": [
                    {
                      "type": "computer_call",
                      "id": "computer_67cf2b3051e88190b006770db6fdb13d",
                      "status": "completed"
                    },
                    {
                      "type": "message",
                      "id": "msg_computer_test",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "I've completed the computer task.",
                          "annotations": []
                        }
                      ]
                    }
                  ],
                  "usage": {
                    "input_tokens": 100,
                    "output_tokens": 50
                  }
                }),
                json!({
                  "prompt": [
                    {
                      "role": "user",
                      "content": [
                        {
                          "type": "text",
                          "text": "Use the computer to complete a task."
                        }
                      ]
                    }
                  ],
                  "tools": [
                    {
                      "type": "provider",
                      "id": "openai.computer_use",
                      "name": "computerUse",
                      "args": {}
                    }
                  ]
                }),
            )
            .await;
            assert_json_eq(
                &r.content(),
                &json!([
                  {
                    "input": "",
                    "providerExecuted": true,
                    "toolCallId": "computer_67cf2b3051e88190b006770db6fdb13d",
                    "toolName": "computer_use",
                    "type": "tool-call"
                  },
                  {
                    "result": {
                      "status": "completed",
                      "type": "computer_use_tool_result"
                    },
                    "toolCallId": "computer_67cf2b3051e88190b006770db6fdb13d",
                    "toolName": "computer_use",
                    "type": "tool-result"
                  },
                  {
                    "providerMetadata": {
                      "openai": {
                        "itemId": "msg_computer_test"
                      }
                    },
                    "text": "I've completed the computer task.",
                    "type": "text"
                  }
                ]),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › should handle mixed url_citation and file_citation annotations
        #[tokio::test]
        async fn should_handle_mixed_url_citation_and_file_citation_annotations() {
            let r = generate(
                "gpt-4o",
                json!({
                  "id": "resp_123",
                  "object": "response",
                  "created_at": 1234567890,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-4o",
                  "output": [
                    {
                      "id": "msg_123",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "Based on web search and file content.",
                          "annotations": [
                            {
                              "type": "url_citation",
                              "start_index": 0,
                              "end_index": 10,
                              "url": "https://example.com",
                              "title": "Example URL"
                            },
                            {
                              "type": "file_citation",
                              "file_id": "file-abc123",
                              "filename": "resource1.json",
                              "index": 123
                            }
                          ]
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": null,
                    "summary": null
                  },
                  "store": true,
                  "temperature": 0,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 100,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 50,
                    "output_tokens_details": {
                      "reasoning_tokens": 0
                    },
                    "total_tokens": 150
                  },
                  "user": null,
                  "metadata": {}
                }),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            assert_json_eq(
                &r.content(),
                &json!([
                  {
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "end_index": 10,
                            "start_index": 0,
                            "title": "Example URL",
                            "type": "url_citation",
                            "url": "https://example.com"
                          },
                          {
                            "file_id": "file-abc123",
                            "filename": "resource1.json",
                            "index": 123,
                            "type": "file_citation"
                          }
                        ],
                        "itemId": "msg_123"
                      }
                    },
                    "text": "Based on web search and file content.",
                    "type": "text"
                  },
                  {
                    "id": "id-0",
                    "sourceType": "url",
                    "title": "Example URL",
                    "type": "source",
                    "url": "https://example.com"
                  },
                  {
                    "filename": "resource1.json",
                    "id": "id-1",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "file-abc123",
                        "index": 123,
                        "type": "file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "resource1.json",
                    "type": "source"
                  }
                ]),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › should handle file_citation annotations only
        #[tokio::test]
        async fn should_handle_file_citation_annotations_only() {
            let r = generate(
                "gpt-4o",
                json!({
                  "id": "resp_456",
                  "object": "response",
                  "created_at": 1234567890,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-4o",
                  "output": [
                    {
                      "id": "msg_456",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "Based on the file content.",
                          "annotations": [
                            {
                              "type": "file_citation",
                              "file_id": "file-xyz789",
                              "filename": "resource1.json",
                              "index": 123
                            }
                          ]
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": null,
                    "summary": null
                  },
                  "store": true,
                  "temperature": 0,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 50,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 25,
                    "output_tokens_details": {
                      "reasoning_tokens": 0
                    },
                    "total_tokens": 75
                  },
                  "user": null,
                  "metadata": {}
                }),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            assert_json_eq(
                &r.content(),
                &json!([
                  {
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "file_id": "file-xyz789",
                            "filename": "resource1.json",
                            "index": 123,
                            "type": "file_citation"
                          }
                        ],
                        "itemId": "msg_456"
                      }
                    },
                    "text": "Based on the file content.",
                    "type": "text"
                  },
                  {
                    "filename": "resource1.json",
                    "id": "id-0",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "file-xyz789",
                        "index": 123,
                        "type": "file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "resource1.json",
                    "type": "source"
                  }
                ]),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › should handle file_citation annotations without optional fields
        #[tokio::test]
        async fn should_handle_file_citation_annotations_without_optional_fields() {
            let r = generate(
                "gpt-5",
                json!({
                  "id": "resp_789",
                  "object": "response",
                  "created_at": 1234567890,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-5",
                  "output": [
                    {
                      "id": "msg_789",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "Answer for the specified years....",
                          "annotations": [
                            {
                              "type": "file_citation",
                              "file_id": "file-YRcoCqn3Fo2K4JgraG",
                              "filename": "resource1.json",
                              "index": 145
                            },
                            {
                              "type": "file_citation",
                              "file_id": "file-YRcoCqn3Fo2K4JgraG",
                              "filename": "resource1.json",
                              "index": 192
                            }
                          ]
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": null,
                    "summary": null
                  },
                  "store": true,
                  "temperature": 0,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 50,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 25,
                    "output_tokens_details": {
                      "reasoning_tokens": 0
                    },
                    "total_tokens": 75
                  },
                  "user": null,
                  "metadata": {}
                }),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            assert_json_eq(
                &r.content(),
                &json!([
                  {
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "file_id": "file-YRcoCqn3Fo2K4JgraG",
                            "filename": "resource1.json",
                            "index": 145,
                            "type": "file_citation"
                          },
                          {
                            "file_id": "file-YRcoCqn3Fo2K4JgraG",
                            "filename": "resource1.json",
                            "index": 192,
                            "type": "file_citation"
                          }
                        ],
                        "itemId": "msg_789"
                      }
                    },
                    "text": "Answer for the specified years....",
                    "type": "text"
                  },
                  {
                    "filename": "resource1.json",
                    "id": "id-0",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "file-YRcoCqn3Fo2K4JgraG",
                        "index": 145,
                        "type": "file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "resource1.json",
                    "type": "source"
                  },
                  {
                    "filename": "resource1.json",
                    "id": "id-1",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "file-YRcoCqn3Fo2K4JgraG",
                        "index": 192,
                        "type": "file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "resource1.json",
                    "type": "source"
                  }
                ]),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › should handle container_file_citation annotations
        #[tokio::test]
        async fn should_handle_container_file_citation_annotations() {
            let r = generate(
                "gpt-5",
                json!({
                  "id": "resp_container",
                  "object": "response",
                  "created_at": 1234567890,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-5",
                  "output": [
                    {
                      "id": "msg_container",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "Generated with container file.",
                          "annotations": [
                            {
                              "type": "container_file_citation",
                              "container_id": "cntr_test",
                              "file_id": "file-container",
                              "filename": "data.csv",
                              "start_index": 0,
                              "end_index": 10,
                              "index": 2
                            }
                          ]
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": null,
                    "summary": null
                  },
                  "store": true,
                  "temperature": 0,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 10,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 5,
                    "output_tokens_details": {
                      "reasoning_tokens": 0
                    },
                    "total_tokens": 15
                  },
                  "user": null,
                  "metadata": {}
                }),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            assert_json_eq(
                &r.content(),
                &json!([
                  {
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "container_id": "cntr_test",
                            "end_index": 10,
                            "file_id": "file-container",
                            "filename": "data.csv",
                            "start_index": 0,
                            "type": "container_file_citation"
                          }
                        ],
                        "itemId": "msg_container"
                      }
                    },
                    "text": "Generated with container file.",
                    "type": "text"
                  },
                  {
                    "filename": "data.csv",
                    "id": "id-0",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "containerId": "cntr_test",
                        "fileId": "file-container",
                        "type": "container_file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "data.csv",
                    "type": "source"
                  }
                ]),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › should handle file_path annotations
        #[tokio::test]
        async fn should_handle_file_path_annotations() {
            let r = generate(
                "gpt-4o",
                json!({
                  "id": "resp_file_path",
                  "object": "response",
                  "created_at": 1234567890,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-4o",
                  "output": [
                    {
                      "id": "msg_file_path",
                      "type": "message",
                      "status": "completed",
                      "role": "assistant",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "Output written to file.",
                          "annotations": [
                            {
                              "type": "file_path",
                              "file_id": "file-path-123",
                              "index": 0
                            }
                          ]
                        }
                      ]
                    }
                  ],
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": null,
                    "summary": null
                  },
                  "store": true,
                  "temperature": 0,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 10,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 5,
                    "output_tokens_details": {
                      "reasoning_tokens": 0
                    },
                    "total_tokens": 15
                  },
                  "user": null,
                  "metadata": {}
                }),
                json!({ "prompt": test_prompt() }),
            )
            .await;
            assert_json_eq(
                &r.content(),
                &json!([
                  {
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "file_id": "file-path-123",
                            "index": 0,
                            "type": "file_path"
                          }
                        ],
                        "itemId": "msg_file_path"
                      }
                    },
                    "text": "Output written to file.",
                    "type": "text"
                  },
                  {
                    "filename": "file-path-123",
                    "id": "id-0",
                    "mediaType": "application/octet-stream",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "file-path-123",
                        "index": 0,
                        "type": "file_path"
                      }
                    },
                    "sourceType": "document",
                    "title": "file-path-123",
                    "type": "source"
                  }
                ]),
            );
        }

        mod provider_metadata_key_based_on_provider_string {
            use super::*;

            fn base_response_body() -> Value {
                json!({
                  "id": "resp_provider_metadata",
                  "object": "response",
                  "created_at": 1234567890,
                  "status": "completed",
                  "error": null,
                  "incomplete_details": null,
                  "input": [],
                  "instructions": null,
                  "max_output_tokens": null,
                  "model": "gpt-4o",
                  "parallel_tool_calls": true,
                  "previous_response_id": null,
                  "reasoning": {
                    "effort": null,
                    "summary": null
                  },
                  "store": true,
                  "temperature": 0,
                  "text": {
                    "format": {
                      "type": "text"
                    }
                  },
                  "tool_choice": "auto",
                  "tools": [],
                  "top_p": 1,
                  "truncation": "disabled",
                  "usage": {
                    "input_tokens": 10,
                    "input_tokens_details": {
                      "cached_tokens": 0
                    },
                    "output_tokens": 5,
                    "output_tokens_details": {
                      "reasoning_tokens": 0
                    },
                    "total_tokens": 15
                  },
                  "user": null,
                  "metadata": {}
                })
            }

            fn response_body(id: &str, output: Value) -> Value {
                let mut body = base_response_body();
                body["id"] = json!(id);
                body["output"] = output;
                body
            }

            /// Upstream builds the model with `provider: 'azure.responses'`;
            /// configure `OpenAIResponsesModel` with the Azure Responses namespace.
            async fn azure_generate(response: Value, options: Value) -> Value {
                let mock = MockFetch::new(vec![Canned::json(&response)]);
                let model = OpenAIResponsesModel::from_config("gpt-4o".into(), {
                    let mut config = stream_config("azure.responses", &mock);
                    config.responses.namespace = AZURE_RESPONSES;
                    config
                });
                let result = model
                    .do_generate(&generate_call_options(options))
                    .await
                    .expect("doGenerate succeeds");
                serde_json::to_value(&result).unwrap()
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › providerMetadata key based on provider string › should use "azure" as providerMetadata key when provider includes "azure"
            #[tokio::test]
            async fn should_use_azure_as_provider_metadata_key_when_provider_includes_azure() {
                let result = azure_generate(
                    response_body(
                        "resp_provider_metadata_azure",
                        json!([
                          {
                            "id": "msg_azure_text",
                            "type": "message",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                              {
                                "type": "output_text",
                                "text": "Hello from Azure!",
                                "annotations": []
                              }
                            ]
                          }
                        ]),
                    ),
                    json!({ "prompt": test_prompt() }),
                )
                .await;
                let metadata = &result["providerMetadata"];
                assert!(metadata.get("azure").is_some(), "{metadata:#}");
                assert!(metadata.get("openai").is_none(), "{metadata:#}");
                assert_json_match_object(
                    &metadata["azure"],
                    &json!({ "responseId": "resp_provider_metadata_azure" }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › providerMetadata key based on provider string › should use "openai" as providerMetadata key when provider does not include "azure"
            #[tokio::test]
            async fn should_use_openai_as_provider_metadata_key_when_provider_does_not_include_azure()
             {
                let r = generate(
                    "gpt-4o",
                    response_body(
                        "resp_provider_metadata_openai",
                        json!([
                          {
                            "id": "msg_openai_text",
                            "type": "message",
                            "status": "completed",
                            "role": "assistant",
                            "content": [
                              {
                                "type": "output_text",
                                "text": "Hello from OpenAI!",
                                "annotations": []
                              }
                            ]
                          }
                        ]),
                    ),
                    json!({ "prompt": test_prompt() }),
                )
                .await;
                let metadata = r.provider_metadata();
                assert!(metadata.get("openai").is_some(), "{metadata:#}");
                assert!(metadata.get("azure").is_none(), "{metadata:#}");
                assert_json_match_object(
                    &metadata["openai"],
                    &json!({ "responseId": "resp_provider_metadata_openai" }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › providerMetadata key based on provider string › should use "azure" as providerMetadata key in tool call content when provider includes "azure"
            #[tokio::test]
            async fn should_use_azure_as_provider_metadata_key_in_tool_call_content_when_provider()
            {
                let result = azure_generate(
                    response_body(
                        "resp_provider_metadata_tool_call",
                        json!([
                          {
                            "id": "fc_azure",
                            "type": "function_call",
                            "status": "completed",
                            "call_id": "call_azure",
                            "name": "weather",
                            "arguments": "{\"location\":\"Seattle\"}"
                          }
                        ]),
                    ),
                    json!({ "prompt": test_prompt(), "tools": test_tools() }),
                )
                .await;
                let tool_call = result["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|part| part["type"] == "tool-call")
                    .expect("tool call")
                    .clone();
                let metadata = &tool_call["providerMetadata"];
                assert!(metadata.get("azure").is_some(), "{tool_call:#}");
                assert!(metadata.get("openai").is_none(), "{tool_call:#}");
            }
        }

        mod compaction {
            use super::*;

            async fn run(options: Value) -> Generated {
                generate("gpt-5.2", json_fixture!("openai-compaction.1"), options).await
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › compaction › should append an explicit compaction trigger as the final input item
            #[tokio::test]
            async fn should_append_an_explicit_compaction_trigger_as_the_final_input_item() {
                let r = run(json!({
                  "prompt": [
                    {
                      "role": "assistant",
                      "content": [
                        {
                          "type": "custom",
                          "kind": "openai.compaction",
                          "providerOptions": {
                            "openai": {
                              "type": "compaction",
                              "itemId": "cmp_123",
                              "encryptedContent": "encrypted_compaction_state"
                            }
                          }
                        }
                      ]
                    },
                    {
                      "role": "user",
                      "content": [
                        {
                          "type": "text",
                          "text": "Continue from this context."
                        }
                      ]
                    }
                  ],
                  "providerOptions": {
                    "openai": {
                      "store": false,
                      "compactionTrigger": true
                    }
                  }
                }))
                .await;
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "type": "compaction",
                          "id": "cmp_123",
                          "encrypted_content": "encrypted_compaction_state"
                        },
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Continue from this context."
                            }
                          ]
                        },
                        {
                          "type": "compaction_trigger"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › compaction › should not append a compaction trigger when disabled
            #[tokio::test]
            async fn should_not_append_a_compaction_trigger_when_disabled() {
                let r = run(json!({
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "compactionTrigger": false
                    }
                  }
                }))
                .await;
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › compaction › should parse compaction output item from real fixture
            #[tokio::test]
            async fn should_parse_compaction_output_item_from_real_fixture() {
                let r = run(json!({
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "store": false,
                      "contextManagement": [
                        {
                          "type": "compaction",
                          "compactThreshold": 50000
                        }
                      ]
                    }
                  }
                }))
                .await;
                let expected: Value = serde_json::from_str(include_str!(
                    "snapshots/generate_compaction_content.json"
                ))
                .unwrap();
                assert_json_eq(&r.content(), &expected);
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › compaction › should send context_management in request body
            #[tokio::test]
            async fn should_send_context_management_in_request_body() {
                let r = run(json!({
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "store": false,
                      "contextManagement": [
                        {
                          "type": "compaction",
                          "compactThreshold": 50000
                        }
                      ]
                    }
                  }
                }))
                .await;
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "model": "gpt-5.2",
                      "store": false,
                      "context_management": [
                        {
                          "type": "compaction",
                          "compact_threshold": 50000
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › compaction › should include compaction item with encrypted content in content
            #[tokio::test]
            async fn should_include_compaction_item_with_encrypted_content_in_content() {
                let r = run(json!({
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "store": false,
                      "contextManagement": [
                        {
                          "type": "compaction",
                          "compactThreshold": 50000
                        }
                      ]
                    }
                  }
                }))
                .await;
                let content = r.content();
                let compaction_part = content
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|part| {
                        part["type"] == "custom"
                            && part["providerMetadata"]["openai"]["type"] == "compaction"
                    })
                    .expect("compaction part");
                let metadata = &compaction_part["providerMetadata"]["openai"];
                assert_eq!(metadata["type"], json!("compaction"));
                assert!(metadata["itemId"].is_string(), "{metadata:#}");
                assert!(metadata["encryptedContent"].is_string(), "{metadata:#}");
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › compaction › should extract usage from compaction response
            #[tokio::test]
            async fn should_extract_usage_from_compaction_response() {
                let r = run(json!({
                  "prompt": test_prompt(),
                  "providerOptions": {
                    "openai": {
                      "store": false,
                      "contextManagement": [
                        {
                          "type": "compaction",
                          "compactThreshold": 50000
                        }
                      ]
                    }
                  }
                }))
                .await;
                let usage = r.usage();
                assert_eq!(usage["inputTokens"]["total"], json!(51097), "{usage:#}");
                assert_eq!(usage["outputTokens"]["total"], json!(2056), "{usage:#}");
            }
        }

        mod phase {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › phase › should include phase in provider metadata for message output items
            #[tokio::test]
            async fn should_include_phase_in_provider_metadata_for_message_output_items() {
                let r = generate(
                    "gpt-5.3-codex",
                    json_fixture!("openai-phase.1"),
                    json!({ "prompt": test_prompt() }),
                )
                .await;
                let content = r.content();
                let text_parts: Vec<&Value> = content
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|part| part["type"] == "text")
                    .collect();
                assert_eq!(text_parts.len(), 2);
                assert_json_match_object(
                    &text_parts[0]["providerMetadata"]["openai"],
                    &json!({
                      "itemId": "msg_0465b6d1ae1f97c500699f883243a481a3b50b985223592984",
                      "phase": "commentary"
                    }),
                );
                assert_json_match_object(
                    &text_parts[1]["providerMetadata"]["openai"],
                    &json!({
                      "itemId": "msg_0465b6d1ae1f97c500699f8835e09c81a3b91e9d502ff18555",
                      "phase": "final_answer"
                    }),
                );
            }
        }
    }

    mod do_stream {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should return helpful error when Chat Completions stream is received
        #[tokio::test]
        async fn should_return_helpful_error_when_chat_completions_stream_is_received() {
            let mock = sse_mock(vec![
            "data:{\"choices\":[],\"created\":0,\"id\":\"\",\"model\":\"\",\"object\":\"\",\"prompt_filter_results\":[{\"prompt_index\":0,\"content_filter_results\":{}}]}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ]);
            let stream = stream_do(
                &mock,
                "gpt-4o",
                json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
            )
            .await
            .unwrap();
            let events = stream_collect(stream).await;
            let error = &stream_find(&events, "error").expect("error part")["error"];
            stream_assert_match(
                error,
                &json!({
                    "name": "AI_APICallError",
                    "message": concat!(
                        "Received a Chat Completions stream while using the OpenAI Responses API. ",
                        "The default OpenAI provider model uses the Responses API. If your custom baseURL targets a Chat Completions-compatible endpoint, use openai.chat('model-id') or createOpenAI(...).chat('model-id') instead. ",
                        "You can also use @ai-sdk/openai-compatible for OpenAI-compatible providers.",
                    ),
                    "responseBody": r#"{"choices":[],"created":0,"id":"","model":"","object":"","prompt_filter_results":[{"prompt_index":0,"content_filter_results":{}}]}"#,
                }),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should preserve async mode on streamed function calls
        #[tokio::test]
        async fn should_preserve_async_mode_on_streamed_function_calls() {
            let function_call = json!({"id": "fc_async", "type": "function_call", "name": "weather", "call_id": "call_async", "arguments": r#"{"location":"Berlin"}"#, "status": "completed", "async": true});
            let mock = sse_mock(vec![
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.created", "response": {"id": "response_async", "created_at": 1, "model": "gpt-6-astra"}})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.output_item.added", "output_index": 0, "item": stream_merge(&function_call, json!({"arguments": "", "status": "in_progress"}))})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.output_item.done", "output_index": 0, "item": function_call})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.completed", "response": {"incomplete_details": null, "output": [function_call], "usage": {"input_tokens": 1, "output_tokens": 2}}})
                ),
            ]);
            let stream = stream_do(
                &mock,
                "gpt-6-astra",
                json!({"prompt": stream_test_prompt(), "tools": stream_test_tools()}),
            )
            .await
            .unwrap();
            let events = stream_collect(stream).await;
            stream_assert_match(
                stream_find(&events, "tool-call").unwrap(),
                &json!({"type": "tool-call", "toolCallId": "call_async", "toolName": "weather", "input": r#"{"location":"Berlin"}"#, "providerMetadata": {"openai": {"itemId": "fc_async", "async": true}}}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should signal schema-invalid known events and finish with error
        #[tokio::test]
        async fn should_signal_schema_invalid_known_events_and_finish_with_error() {
            let function_call = json!({"id": "fc_1", "type": "function_call", "name": "get_weather", "call_id": "call_1", "arguments": r#"{"city":"Berlin"}"#, "status": "completed"});
            let mock = sse_mock(vec![
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.created", "response": {"id": "response_1", "created_at": 1, "model": "gpt-5.1"}})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.output_item.added", "item": stream_merge(&function_call, json!({"arguments": "", "status": "in_progress"}))})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": r#"{"city":"Berlin"}"#})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.function_call_arguments.done", "item_id": "fc_1", "arguments": r#"{"city":"Berlin"}"#})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.output_item.done", "item": function_call})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.completed", "response": {"incomplete_details": null, "output": [function_call], "usage": {"input_tokens": 1, "output_tokens": 2}}})
                ),
            ]);
            let stream = stream_do(&mock, "gpt-5.1", json!({"prompt": stream_test_prompt()}))
                .await
                .unwrap();
            let events = stream_collect(stream).await;
            assert_eq!(
                events.iter().filter(|part| part["type"] == "error").count(),
                4
            );
            assert!(!events.iter().any(|part| part["type"] == "tool-call"));
            stream_assert_match(
                events.last().unwrap(),
                &json!({"type": "finish", "finishReason": {"unified": "error"}}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should continue ignoring unknown event types
        #[tokio::test]
        async fn should_continue_ignoring_unknown_event_types() {
            let mock = sse_mock(vec![
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.created", "response": {"id": "response_1", "created_at": 1, "model": "gpt-5.1"}})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.future_event", "value": "ignored"})
                ),
                format!(
                    "data: {}\n\n",
                    json!({"type": "response.completed", "response": {"incomplete_details": null, "output": [], "usage": {"input_tokens": 1, "output_tokens": 2}}})
                ),
            ]);
            let stream = stream_do(&mock, "gpt-5.1", json!({"prompt": stream_test_prompt()}))
                .await
                .unwrap();
            let events = stream_collect(stream).await;
            assert!(!events.iter().any(|part| part["type"] == "error"));
            stream_assert_match(
                events.last().unwrap(),
                &json!({"type": "finish", "finishReason": {"unified": "stop"}}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should stream text deltas
        #[tokio::test]
        async fn should_stream_text_deltas() {
            let mock = sse_mock(vec![
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0.3,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.in_progress\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0.3,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"type\":\"message\",\"status\":\"in_progress\",\"role\":\"assistant\",\"content\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.added\",\"item_id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\",\"annotations\":[],\"logprobs\": []}}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hello,\",\"logprobs\": []}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"output_index\":0,\"content_index\":0,\"delta\":\" World!\",\"logprobs\": []}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.done\",\"item_id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"output_index\":0,\"content_index\":0,\"text\":\"Hello, World!\"}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.done\",\"item_id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"Hello, World!\",\"annotations\":[],\"logprobs\": []}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello, World!\",\"annotations\":[],\"logprobs\": []}]}}\n\n".to_string(),
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67c9a878139c8190aa2e3105411b408b\",\"object\":\"response\",\"created_at\":1741269112,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[{\"id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello, World!\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0.3,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":543,\"input_tokens_details\":{\"cached_tokens\":234},\"output_tokens\":478,\"output_tokens_details\":{\"reasoning_tokens\":123},\"total_tokens\":512},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
        ]);
            let stream = stream_do(
                &mock,
                "gpt-4o",
                json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
            )
            .await
            .unwrap();
            stream_assert_parts(
                &stream_collect(stream).await,
                &[
                    json!({
                        "type": "stream-start",
                        "warnings": [],
                    }),
                    json!({
                        "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                        "modelId": "gpt-4o-2024-07-18",
                        "timestamp": "2025-03-06T13:50:19.000Z",
                        "type": "response-metadata",
                    }),
                    json!({
                        "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                            },
                        },
                        "type": "text-start",
                    }),
                    json!({
                        "delta": "Hello,",
                        "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                        "type": "text-delta",
                    }),
                    json!({
                        "delta": " World!",
                        "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                        "type": "text-delta",
                    }),
                    json!({
                        "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                            },
                        },
                        "type": "text-end",
                    }),
                    json!({
                        "finishReason": {
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            "openai": {
                                "responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 234,
                                "noCache": 309,
                                "total": 543,
                            },
                            "outputTokens": {
                                "reasoning": 123,
                                "text": 355,
                                "total": 478,
                            },
                            "raw": {
                                "input_tokens": 543,
                                "input_tokens_details": {
                                    "cached_tokens": 234,
                                },
                                "output_tokens": 478,
                                "output_tokens_details": {
                                    "reasoning_tokens": 123,
                                },
                                "total_tokens": 512,
                            },
                        },
                    }),
                ],
            );
        }

        mod provider_metadata_key_based_on_provider_string {
            use super::*;
            fn streaming_chunks() -> Vec<String> {
                vec![
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0.3,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.in_progress\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0.3,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"type\":\"message\",\"status\":\"in_progress\",\"role\":\"assistant\",\"content\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.added\",\"item_id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\",\"annotations\":[],\"logprobs\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hello,\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"output_index\":0,\"content_index\":0,\"delta\":\" World!\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.done\",\"item_id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"output_index\":0,\"content_index\":0,\"text\":\"Hello, World!\"}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.done\",\"item_id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"Hello, World!\",\"annotations\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello, World!\",\"annotations\":[]}]}}\n\n".to_string(),
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67c9a878139c8190aa2e3105411b408b\",\"object\":\"response\",\"created_at\":1741269112,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[{\"id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello, World!\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0.3,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":543,\"input_tokens_details\":{\"cached_tokens\":234},\"output_tokens\":478,\"output_tokens_details\":{\"reasoning_tokens\":123},\"total_tokens\":512},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            ]
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › providerMetadata key based on provider string › should use "azure" as providerMetadata key in finish event when provider "azure" but providerOptions is "openai"
            #[tokio::test]
            async fn should_use_azure_as_provider_metadata_key_in_finish_event_when_provider_azure_bu()
             {
                let mock = sse_mock(streaming_chunks());
                let model = OpenAIResponsesModel::from_config("gpt-4o".into(), {
                    let mut config = stream_config("azure.responses", &mock);
                    config.responses.namespace = AZURE_RESPONSES;
                    config
                });
                let stream = stream_do_with(&model, json!({"prompt": stream_test_prompt(), "includeRawChunks": false, "providerOptions": {"openai": {"reasoningSummary": "auto"}}})).await.unwrap();
                let events = stream_collect(stream).await;
                let finish_event = stream_find(&events, "finish");
                assert_eq!(
                    finish_event.map(|part| part["type"] == "finish"),
                    Some(true)
                );
                let metadata = &finish_event.unwrap()["providerMetadata"];
                assert!(metadata.get("azure").is_some(), "{metadata}");
                assert!(metadata.get("openai").is_none(), "{metadata}");
                stream_assert_match(
                    &metadata["azure"],
                    &json!({"responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8"}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › providerMetadata key based on provider string › should use "openai" as providerMetadata key in finish event when provider does not include "azure"
            #[tokio::test]
            async fn should_use_openai_as_provider_metadata_key_in_finish_event_when_provider_does_no()
             {
                let mock = sse_mock(streaming_chunks());
                let stream = stream_do(
                    &mock,
                    "gpt-4o",
                    json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
                )
                .await
                .unwrap();
                let events = stream_collect(stream).await;
                let finish_event = stream_find(&events, "finish");
                assert_eq!(
                    finish_event.map(|part| part["type"] == "finish"),
                    Some(true)
                );
                let metadata = &finish_event.unwrap()["providerMetadata"];
                assert!(metadata.get("openai").is_some(), "{metadata}");
                assert!(metadata.get("azure").is_none(), "{metadata}");
                stream_assert_match(
                    &metadata["openai"],
                    &json!({"responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8"}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › providerMetadata key based on provider string › should use "azure" as providerMetadata key in streaming reasoning events when provider includes "azure"
            #[tokio::test]
            async fn should_use_azure_as_provider_metadata_key_in_streaming_reasoning_events_when_pro()
             {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_reasoning\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"rs_reasoning_item\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_reasoning_item\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_reasoning_item\",\"summary_index\":0,\"delta\":\"thinking through the steps\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"rs_reasoning_item\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"rs_reasoning_item\",\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"thinking through the steps\"}]}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_reasoning\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[{\"id\":\"rs_reasoning_item\",\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"thinking through the steps\"}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":10,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":20,\"output_tokens_details\":{\"reasoning_tokens\":20},\"total_tokens\":30},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            ]);
                let model = OpenAIResponsesModel::from_config("o3-mini".into(), {
                    let mut config = stream_config("azure.responses", &mock);
                    config.responses.namespace = AZURE_RESPONSES;
                    config
                });
                let stream = stream_do_with(
                    &model,
                    json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
                )
                .await
                .unwrap();
                let events = stream_collect(stream).await;
                let reasoning_start = stream_find(&events, "reasoning-start");
                assert_eq!(
                    reasoning_start.map(|part| part["type"] == "reasoning-start"),
                    Some(true)
                );
                let metadata = &reasoning_start.unwrap()["providerMetadata"];
                assert!(metadata.get("azure").is_some(), "{metadata}");
                assert!(metadata.get("openai").is_none(), "{metadata}");
                stream_assert_match(
                    &metadata["azure"],
                    &json!({"itemId": "rs_reasoning_item", "reasoningEncryptedContent": null}),
                );
                let reasoning_delta = stream_find(&events, "reasoning-delta");
                assert_eq!(
                    reasoning_delta.map(|part| part["type"] == "reasoning-delta"),
                    Some(true)
                );
                let metadata = &reasoning_delta.unwrap()["providerMetadata"];
                assert!(metadata.get("azure").is_some(), "{metadata}");
                assert!(metadata.get("openai").is_none(), "{metadata}");
                stream_assert_match(&metadata["azure"], &json!({"itemId": "rs_reasoning_item"}));
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should send finish reason for incomplete response
        #[tokio::test]
        async fn should_send_finish_reason_for_incomplete_response() {
            let mock = sse_mock(vec![
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0.3,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.in_progress\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0.3,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"type\":\"message\",\"status\":\"in_progress\",\"role\":\"assistant\",\"content\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.added\",\"item_id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\",\"annotations\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c9a81dea8c8190b79651a2b3adf91e\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hello,\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.done\",\"item_id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"output_index\":0,\"content_index\":0,\"text\":\"Hello,!\"}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.done\",\"item_id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"Hello,\",\"annotations\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"msg_67c9a8787f4c8190b49c858d4c1cf20c\",\"type\":\"message\",\"status\":\"incomplete\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello,\",\"annotations\":[]}]}}\n\n".to_string(),
            "data:{\"type\":\"response.incomplete\",\"response\":{\"id\":\"resp_67cadb40a0708190ac2763c0b6960f6f\",\"object\":\"response\",\"created_at\":1741347648,\"status\":\"incomplete\",\"error\":null,\"incomplete_details\":{\"reason\":\"max_output_tokens\"},\"instructions\":null,\"max_output_tokens\":100,\"model\":\"gpt-4o-2024-07-18\",\"output\":[{\"type\":\"message\",\"id\":\"msg_67cadb410ccc81909fe1d8f427b9cf02\",\"status\":\"incomplete\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello,\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":0,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":0,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":0},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
        ]);
            let stream = stream_do(
                &mock,
                "gpt-4o",
                json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
            )
            .await
            .unwrap();
            stream_assert_parts(
                &stream_collect(stream).await,
                &[
                    json!({
                        "type": "stream-start",
                        "warnings": [],
                    }),
                    json!({
                        "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                        "modelId": "gpt-4o-2024-07-18",
                        "timestamp": "2025-03-06T13:50:19.000Z",
                        "type": "response-metadata",
                    }),
                    json!({
                        "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                            },
                        },
                        "type": "text-start",
                    }),
                    json!({
                        "delta": "Hello,",
                        "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                        "type": "text-delta",
                    }),
                    json!({
                        "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                            },
                        },
                        "type": "text-end",
                    }),
                    json!({
                        "finishReason": {
                            "raw": "max_output_tokens",
                            "unified": "length",
                        },
                        "providerMetadata": {
                            "openai": {
                                "responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "noCache": 0,
                                "total": 0,
                            },
                            "outputTokens": {
                                "reasoning": 0,
                                "text": 0,
                                "total": 0,
                            },
                            "raw": {
                                "input_tokens": 0,
                                "input_tokens_details": {
                                    "cached_tokens": 0,
                                },
                                "output_tokens": 0,
                                "output_tokens_details": {
                                    "reasoning_tokens": 0,
                                },
                                "total_tokens": 0,
                            },
                        },
                    }),
                ],
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should send streaming tool calls
        #[tokio::test]
        async fn should_send_streaming_tool_calls() {
            let mock = sse_mock(vec![
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67cb13a755c08190acbe3839a49632fc\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"function\",\"description\":\"Get the current location.\",\"name\":\"currentLocation\",\"parameters\":{\"type\":\"object\",\"properties\":{},\"additionalProperties\":false},\"strict\":true},{\"type\":\"function\",\"description\":\"Get the weather in a location\",\"name\":\"weather\",\"parameters\":{\"type\":\"object\",\"properties\":{\"location\":{\"type\":\"string\",\"description\":\"The location to get the weather for\"}},\"required\":[\"location\"],\"additionalProperties\":false},\"strict\":true}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.in_progress\",\"response\":{\"id\":\"resp_67cb13a755c08190acbe3839a49632fc\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"function\",\"description\":\"Get the current location.\",\"name\":\"currentLocation\",\"parameters\":{\"type\":\"object\",\"properties\":{},\"additionalProperties\":false},\"strict\":true},{\"type\":\"function\",\"description\":\"Get the weather in a location\",\"name\":\"weather\",\"parameters\":{\"type\":\"object\",\"properties\":{\"location\":{\"type\":\"string\",\"description\":\"The location to get the weather for\"}},\"required\":[\"location\"],\"additionalProperties\":false},\"strict\":true}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_67cb13a838088190be08eb3927c87501\",\"call_id\":\"call_6KxSghkb4MVnunFH2TxPErLP\",\"name\":\"currentLocation\",\"arguments\":\"\",\"status\":\"completed\"}}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_67cb13a838088190be08eb3927c87501\",\"output_index\":0,\"delta\":\"{}\"}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_67cb13a838088190be08eb3927c87501\",\"output_index\":0,\"arguments\":\"{}\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_67cb13a838088190be08eb3927c87501\",\"call_id\":\"call_pgjcAI4ZegMkP6bsAV7sfrJA\",\"name\":\"currentLocation\",\"arguments\":\"{}\",\"status\":\"completed\"}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"id\":\"fc_67cb13a858f081908a600343fa040f47\",\"call_id\":\"call_Dg6WUmFHNeR5JxX1s53s1G4b\",\"name\":\"weather\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_67cb13a858f081908a600343fa040f47\",\"output_index\":1,\"delta\":\"{\"}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_67cb13a858f081908a600343fa040f47\",\"output_index\":1,\"delta\":\"\\\"location\"}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_67cb13a858f081908a600343fa040f47\",\"output_index\":1,\"delta\":\"\\\":\"}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_67cb13a858f081908a600343fa040f47\",\"output_index\":1,\"delta\":\"\\\"Rome\"}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_67cb13a858f081908a600343fa040f47\",\"output_index\":1,\"delta\":\"\\\"}\"}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_67cb13a858f081908a600343fa040f47\",\"output_index\":1,\"arguments\":\"{\\\"location\\\":\\\"Rome\\\"}\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"id\":\"fc_67cb13a858f081908a600343fa040f47\",\"call_id\":\"call_X2PAkDJInno9VVnNkDrfhboW\",\"name\":\"weather\",\"arguments\":\"{\\\"location\\\":\\\"Rome\\\"}\",\"status\":\"completed\"}}\n\n".to_string(),
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67cb13a755c08190acbe3839a49632fc\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[{\"type\":\"function_call\",\"id\":\"fc_67cb13a838088190be08eb3927c87501\",\"call_id\":\"call_KsVqaVAf3alAtCCkQe4itE7W\",\"name\":\"currentLocation\",\"arguments\":\"{}\",\"status\":\"completed\"},{\"type\":\"function_call\",\"id\":\"fc_67cb13a858f081908a600343fa040f47\",\"call_id\":\"call_X2PAkDJInno9VVnNkDrfhboW\",\"name\":\"weather\",\"arguments\":\"{\\\"location\\\":\\\"Rome\\\"}\",\"status\":\"completed\"}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"function\",\"description\":\"Get the current location.\",\"name\":\"currentLocation\",\"parameters\":{\"type\":\"object\",\"properties\":{},\"additionalProperties\":false},\"strict\":true},{\"type\":\"function\",\"description\":\"Get the weather in a location\",\"name\":\"weather\",\"parameters\":{\"type\":\"object\",\"properties\":{\"location\":{\"type\":\"string\",\"description\":\"The location to get the weather for\"}},\"required\":[\"location\"],\"additionalProperties\":false},\"strict\":true}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":0,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":0,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":0},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
        ]);
            let stream = stream_do(&mock, "gpt-4o", json!({"tools": stream_test_tools(), "prompt": stream_test_prompt(), "includeRawChunks": false})).await.unwrap();
            stream_assert_parts(
                &stream_collect(stream).await,
                &[
                    json!({
                        "type": "stream-start",
                        "warnings": [],
                    }),
                    json!({
                        "id": "resp_67cb13a755c08190acbe3839a49632fc",
                        "modelId": "gpt-4o-2024-07-18",
                        "timestamp": "2025-03-07T15:41:27.000Z",
                        "type": "response-metadata",
                    }),
                    json!({
                        "id": "call_6KxSghkb4MVnunFH2TxPErLP",
                        "toolName": "currentLocation",
                        "type": "tool-input-start",
                    }),
                    json!({
                        "delta": "{}",
                        "id": "call_6KxSghkb4MVnunFH2TxPErLP",
                        "type": "tool-input-delta",
                    }),
                    json!({
                        "id": "call_pgjcAI4ZegMkP6bsAV7sfrJA",
                        "type": "tool-input-end",
                    }),
                    json!({
                        "input": "{}",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "fc_67cb13a838088190be08eb3927c87501",
                            },
                        },
                        "toolCallId": "call_pgjcAI4ZegMkP6bsAV7sfrJA",
                        "toolName": "currentLocation",
                        "type": "tool-call",
                    }),
                    json!({
                        "id": "call_Dg6WUmFHNeR5JxX1s53s1G4b",
                        "toolName": "weather",
                        "type": "tool-input-start",
                    }),
                    json!({
                        "delta": "{",
                        "id": "call_Dg6WUmFHNeR5JxX1s53s1G4b",
                        "type": "tool-input-delta",
                    }),
                    json!({
                        "delta": r#""location"#,
                        "id": "call_Dg6WUmFHNeR5JxX1s53s1G4b",
                        "type": "tool-input-delta",
                    }),
                    json!({
                        "delta": r#"":"#,
                        "id": "call_Dg6WUmFHNeR5JxX1s53s1G4b",
                        "type": "tool-input-delta",
                    }),
                    json!({
                        "delta": r#""Rome"#,
                        "id": "call_Dg6WUmFHNeR5JxX1s53s1G4b",
                        "type": "tool-input-delta",
                    }),
                    json!({
                        "delta": r#""}"#,
                        "id": "call_Dg6WUmFHNeR5JxX1s53s1G4b",
                        "type": "tool-input-delta",
                    }),
                    json!({
                        "id": "call_X2PAkDJInno9VVnNkDrfhboW",
                        "type": "tool-input-end",
                    }),
                    json!({
                        "input": r#"{"location":"Rome"}"#,
                        "providerMetadata": {
                            "openai": {
                                "itemId": "fc_67cb13a858f081908a600343fa040f47",
                            },
                        },
                        "toolCallId": "call_X2PAkDJInno9VVnNkDrfhboW",
                        "toolName": "weather",
                        "type": "tool-call",
                    }),
                    json!({
                        "finishReason": {
                            "unified": "tool-calls",
                        },
                        "providerMetadata": {
                            "openai": {
                                "responseId": "resp_67cb13a755c08190acbe3839a49632fc",
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "noCache": 0,
                                "total": 0,
                            },
                            "outputTokens": {
                                "reasoning": 0,
                                "text": 0,
                                "total": 0,
                            },
                            "raw": {
                                "input_tokens": 0,
                                "input_tokens_details": {
                                    "cached_tokens": 0,
                                },
                                "output_tokens": 0,
                                "output_tokens_details": {
                                    "reasoning_tokens": 0,
                                },
                                "total_tokens": 0,
                            },
                        },
                    }),
                ],
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should replay streamed wrapper input when expansion fails
        #[tokio::test]
        async fn should_replay_streamed_wrapper_input_when_expansion_fails() {
            let input_deltas = [
                r#"{"tool_uses":["#,
                r#"{"recipient_name":"functions.weather","parameters":{}}]"#,
            ];
            let input = input_deltas.concat();
            let mut chunks = vec![format!(
                "data:{}\n\n",
                json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "id": "fc_parallel_malformed", "call_id": "call_parallel_malformed", "name": "parallel", "arguments": "", "status": "in_progress"}})
            )];
            for delta in input_deltas {
                chunks.push(format!("data:{}\n\n", json!({"type": "response.function_call_arguments.delta", "item_id": "fc_parallel_malformed", "output_index": 0, "delta": delta})));
            }
            chunks.push(format!("data:{}\n\n", json!({"type": "response.output_item.done", "output_index": 0, "item": {"type": "function_call", "id": "fc_parallel_malformed", "call_id": "call_parallel_malformed", "name": "parallel", "arguments": input, "status": "completed"}})));
            let mock = sse_mock(chunks);
            let stream = stream_do(
                &mock,
                "gpt-5.4",
                json!({"tools": stream_test_tools(), "prompt": stream_test_prompt()}),
            )
            .await
            .unwrap();
            let events = stream_collect(stream).await;
            let mut expected = vec![
                json!({"type": "tool-input-start", "id": "call_parallel_malformed", "toolName": "parallel"}),
            ];
            for delta in input_deltas {
                expected.push(json!({"type": "tool-input-delta", "id": "call_parallel_malformed", "delta": delta}));
            }
            expected.push(json!({"type": "tool-input-end", "id": "call_parallel_malformed"}));
            expected.push(json!({"type": "tool-call", "toolCallId": "call_parallel_malformed", "toolName": "parallel", "input": input, "providerMetadata": {"openai": {"itemId": "fc_parallel_malformed"}}}));
            assert_eq!(stream_tool_parts(&events), expected);
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should flush streamed wrapper input when the stream ends early
        #[tokio::test]
        async fn should_flush_streamed_wrapper_input_when_the_stream_ends_early() {
            let input_deltas = [r#"{"tool_uses":["#, r#"{"recipient_name":"#];
            let mut chunks = vec![format!(
                "data:{}\n\n",
                json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "id": "fc_parallel_truncated", "call_id": "call_parallel_truncated", "name": "parallel", "arguments": "", "status": "in_progress"}})
            )];
            for delta in input_deltas {
                chunks.push(format!("data:{}\n\n", json!({"type": "response.function_call_arguments.delta", "item_id": "fc_parallel_truncated", "output_index": 0, "delta": delta})));
            }
            let mock = sse_mock(chunks);
            let stream = stream_do(
                &mock,
                "gpt-5.4",
                json!({"tools": stream_test_tools(), "prompt": stream_test_prompt()}),
            )
            .await
            .unwrap();
            let events = stream_collect(stream).await;
            let mut expected = vec![
                json!({"type": "tool-input-start", "id": "call_parallel_truncated", "toolName": "parallel"}),
            ];
            for delta in input_deltas {
                expected.push(json!({"type": "tool-input-delta", "id": "call_parallel_truncated", "delta": delta}));
            }
            assert_eq!(stream_tool_parts(&events), expected);
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should preserve namespace on streaming function_call output
        #[tokio::test]
        async fn should_preserve_namespace_on_streaming_function_call_output() {
            let mock = sse_mock(vec![
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_ns\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-5.4\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_ns_1\",\"call_id\":\"call_ns_1\",\"name\":\"get_weather\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_ns_1\",\"output_index\":0,\"delta\":\"{\\\"location\\\":\\\"NYC\\\"}\"}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_ns_1\",\"output_index\":0,\"arguments\":\"{\\\"location\\\":\\\"NYC\\\"}\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_ns_1\",\"call_id\":\"call_ns_1\",\"name\":\"get_weather\",\"arguments\":\"{\\\"location\\\":\\\"NYC\\\"}\",\"status\":\"completed\",\"namespace\":\"weather_ns\"}}\n\n".to_string(),
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_ns\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-5.4\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":0,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":0,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":0},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
        ]);
            let stream = stream_do(&mock, "gpt-5.4", json!({"prompt": stream_test_prompt(), "tools": stream_test_tools(), "includeRawChunks": false})).await.unwrap();
            let events = stream_collect(stream).await;
            let tool_call_event = stream_find(&events, "tool-call");
            assert!(tool_call_event.is_some());
            stream_assert_match(
                &tool_call_event.unwrap()["providerMetadata"]["openai"],
                &json!({"itemId": "fc_ns_1", "namespace": "weather_ns"}),
            );
            let tool_input_end = stream_find(&events, "tool-input-end");
            assert!(tool_input_end.is_some());
            stream_assert_match(
                &tool_input_end.unwrap()["providerMetadata"]["openai"],
                &json!({"namespace": "weather_ns"}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should not set namespace on streaming function_call when absent
        #[tokio::test]
        async fn should_not_set_namespace_on_streaming_function_call_when_absent() {
            let mock = sse_mock(vec![
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_plain\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_plain\",\"call_id\":\"call_plain\",\"name\":\"get_weather\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\n".to_string(),
            "data:{\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_plain\",\"output_index\":0,\"arguments\":\"{}\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_plain\",\"call_id\":\"call_plain\",\"name\":\"get_weather\",\"arguments\":\"{}\",\"status\":\"completed\"}}\n\n".to_string(),
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_plain\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-4o\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":0,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":0,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":0},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
        ]);
            let stream = stream_do(&mock, "gpt-4o", json!({"prompt": stream_test_prompt(), "tools": stream_test_tools(), "includeRawChunks": false})).await.unwrap();
            let events = stream_collect(stream).await;
            let tool_call_event = stream_find(&events, "tool-call");
            assert!(tool_call_event.is_some());
            let meta = &tool_call_event.unwrap()["providerMetadata"]["openai"];
            assert!(meta.get("namespace").is_none(), "{meta}");
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should expose reasoning context and cache writes when streaming
        #[tokio::test]
        async fn should_expose_reasoning_context_and_cache_writes_when_streaming() {
            let mock = sse_mock(vec![
                format!(
                    "data:{}\n\n",
                    json!({"type": "response.created", "response": {"id": "resp_gpt56", "created_at": 1783620000, "model": "gpt-5.6"}})
                ),
                format!(
                    "data:{}\n\n",
                    json!({"type": "response.completed", "response": {"incomplete_details": null, "reasoning": {"context": "all_turns"}, "usage": {"input_tokens": 100, "input_tokens_details": {"cached_tokens": 40, "cache_write_tokens": 25}, "output_tokens": 20, "output_tokens_details": {"reasoning_tokens": 5}}}})
                ),
            ]);
            let stream = stream_do(&mock, "gpt-5.6", json!({"prompt": stream_test_prompt()}))
                .await
                .unwrap();
            let events = stream_collect(stream).await;
            stream_assert_match(
                events.last().unwrap(),
                &json!({"type": "finish", "providerMetadata": {"openai": {"responseId": "resp_gpt56", "reasoningContext": "all_turns"}}, "usage": {"inputTokens": {"total": 100, "noCache": 35, "cacheRead": 40, "cacheWrite": 25}}}),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › Should handle service tier
        #[tokio::test]
        async fn should_handle_service_tier() {
            let mock = sse_mock(vec![
            "data:{\"type\":\"response.created\",\"sequence_number\":0,\"response\":{\"id\":\"resp_68b08bfa71908196889e9ae5668b2ae40cd677a623b867d5\",\"object\":\"response\",\"created_at\":1756400634,\"status\":\"in_progress\",\"background\":false,\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"max_tool_calls\":null,\"model\":\"gpt-5-nano-2025-08-07\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"prompt_cache_key\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":null},\"safety_identifier\":null,\"service_tier\":\"flex\",\"store\":true,\"temperature\":1,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tools\":[],\"top_logprobs\":0,\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.in_progress\",\"sequence_number\":1,\"response\":{\"id\":\"resp_68b08bfa71908196889e9ae5668b2ae40cd677a623b867d5\",\"object\":\"response\",\"created_at\":1756400634,\"status\":\"in_progress\",\"background\":false,\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"max_tool_calls\":null,\"model\":\"gpt-5-nano-2025-08-07\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"prompt_cache_key\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":null},\"safety_identifier\":null,\"service_tier\":\"flex\",\"store\":true,\"temperature\":1,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tools\":[],\"top_logprobs\":0,\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"sequence_number\":2,\"output_index\":0,\"item\":{\"id\":\"rs_68b08bfb9f3c819682c5cff6edee6e4d0cd677a623b867d5\",\"type\":\"reasoning\",\"summary\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"sequence_number\":3,\"output_index\":0,\"item\":{\"id\":\"rs_68b08bfb9f3c819682c5cff6edee6e4d0cd677a623b867d5\",\"type\":\"reasoning\",\"summary\":[]}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"sequence_number\":4,\"output_index\":1,\"item\":{\"id\":\"msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5\",\"type\":\"message\",\"status\":\"in_progress\",\"content\":[],\"role\":\"assistant\"}}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.added\",\"sequence_number\":5,\"item_id\":\"msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5\",\"output_index\":1,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"\"}}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.delta\",\"sequence_number\":6,\"item_id\":\"msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5\",\"output_index\":1,\"content_index\":0,\"delta\":\"blue\",\"logprobs\":[],\"obfuscation\":\"A3q16QVxivdK\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.done\",\"sequence_number\":7,\"item_id\":\"msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5\",\"output_index\":1,\"content_index\":0,\"text\":\"blue\",\"logprobs\":[]}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.done\",\"sequence_number\":8,\"item_id\":\"msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5\",\"output_index\":1,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"blue\"}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"sequence_number\":9,\"output_index\":1,\"item\":{\"id\":\"msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5\",\"type\":\"message\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"blue\"}],\"role\":\"assistant\"}}\n\n".to_string(),
            "data:{\"type\":\"response.completed\",\"sequence_number\":10,\"response\":{\"id\":\"resp_68b08bfa71908196889e9ae5668b2ae40cd677a623b867d5\",\"object\":\"response\",\"created_at\":1756400634,\"status\":\"completed\",\"background\":false,\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"max_tool_calls\":null,\"model\":\"gpt-5-nano-2025-08-07\",\"output\":[{\"id\":\"rs_68b08bfb9f3c819682c5cff6edee6e4d0cd677a623b867d5\",\"type\":\"reasoning\",\"summary\":[]},{\"id\":\"msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5\",\"type\":\"message\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"blue\"}],\"role\":\"assistant\"}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"prompt_cache_key\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":null},\"safety_identifier\":null,\"service_tier\":\"flex\",\"store\":true,\"temperature\":1,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tools\":[],\"top_logprobs\":0,\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":263,\"output_tokens_details\":{\"reasoning_tokens\":256},\"total_tokens\":278},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
        ]);
            let stream = stream_do(&mock, "gpt-5-nano", json!({"prompt": stream_test_prompt(), "providerOptions": {"openai": {"serviceTier": "flex"}}})).await.unwrap();
            stream_assert_parts(
                &stream_collect(stream).await,
                &[
                    json!({
                        "type": "stream-start",
                        "warnings": [],
                    }),
                    json!({
                        "id": "resp_68b08bfa71908196889e9ae5668b2ae40cd677a623b867d5",
                        "modelId": "gpt-5-nano-2025-08-07",
                        "timestamp": "2025-08-28T17:03:54.000Z",
                        "type": "response-metadata",
                    }),
                    json!({
                        "id": "rs_68b08bfb9f3c819682c5cff6edee6e4d0cd677a623b867d5:0",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "rs_68b08bfb9f3c819682c5cff6edee6e4d0cd677a623b867d5",
                                "reasoningEncryptedContent": null,
                            },
                        },
                        "type": "reasoning-start",
                    }),
                    json!({
                        "id": "rs_68b08bfb9f3c819682c5cff6edee6e4d0cd677a623b867d5:0",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "rs_68b08bfb9f3c819682c5cff6edee6e4d0cd677a623b867d5",
                                "reasoningEncryptedContent": null,
                            },
                        },
                        "type": "reasoning-end",
                    }),
                    json!({
                        "id": "msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5",
                            },
                        },
                        "type": "text-start",
                    }),
                    json!({
                        "delta": "blue",
                        "id": "msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5",
                        "type": "text-delta",
                    }),
                    json!({
                        "id": "msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "msg_68b08bfc9a548196b15465b6020b04e40cd677a623b867d5",
                            },
                        },
                        "type": "text-end",
                    }),
                    json!({
                        "finishReason": {
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            "openai": {
                                "responseId": "resp_68b08bfa71908196889e9ae5668b2ae40cd677a623b867d5",
                                "serviceTier": "flex",
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "noCache": 15,
                                "total": 15,
                            },
                            "outputTokens": {
                                "reasoning": 256,
                                "text": 7,
                                "total": 263,
                            },
                            "raw": {
                                "input_tokens": 15,
                                "input_tokens_details": {
                                    "cached_tokens": 0,
                                },
                                "output_tokens": 263,
                                "output_tokens_details": {
                                    "reasoning_tokens": 256,
                                },
                                "total_tokens": 278,
                            },
                        },
                    }),
                ],
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › should handle logprobs
        #[tokio::test]
        async fn should_handle_logprobs() {
            let mock = sse_mock(vec![
            "data:{\"type\":\"response.created\",\"sequence_number\":0,\"response\":{\"id\":\"resp_689cec4cf608819583c56813ccb0f5040f92af1765dd5aad\",\"object\":\"response\",\"created_at\":1755114572,\"status\":\"in_progress\",\"background\":false,\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":1024,\"max_tool_calls\":null,\"model\":\"gpt-4.1-nano-2025-04-14\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"prompt_cache_key\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"safety_identifier\":null,\"service_tier\":\"auto\",\"store\":true,\"temperature\":1,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tools\":[],\"top_logprobs\":5,\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.in_progress\",\"sequence_number\":1,\"response\":{\"id\":\"resp_689cec4cf608819583c56813ccb0f5040f92af1765dd5aad\",\"object\":\"response\",\"created_at\":1755114572,\"status\":\"in_progress\",\"background\":false,\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":1024,\"max_tool_calls\":null,\"model\":\"gpt-4.1-nano-2025-04-14\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"prompt_cache_key\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"safety_identifier\":null,\"service_tier\":\"auto\",\"store\":true,\"temperature\":1,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tools\":[],\"top_logprobs\":5,\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.added\",\"sequence_number\":2,\"output_index\":0,\"item\":{\"id\":\"msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad\",\"type\":\"message\",\"status\":\"in_progress\",\"content\":[],\"role\":\"assistant\"}}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.added\",\"sequence_number\":3,\"item_id\":\"msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"\"}}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.delta\",\"sequence_number\":4,\"item_id\":\"msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad\",\"output_index\":0,\"content_index\":0,\"delta\":\"N\",\"logprobs\":[{\"bytes\":[78],\"logprob\":-2.9266366958618164,\"token\":\"N\",\"top_logprobs\":[{\"bytes\":[80,108,101,97,115,101],\"logprob\":-0.5516367554664612,\"token\":\"Please\"},{\"bytes\":[89],\"logprob\":-1.0516366958618164,\"token\":\"Y\"},{\"bytes\":[78],\"logprob\":-2.9266366958618164,\"token\":\"N\"},{\"bytes\":[83,117,114,101],\"logprob\":-4.551636695861816,\"token\":\"Sure\"},{\"bytes\":[67,111,117,108,100],\"logprob\":-5.176636695861816,\"token\":\"Could\"}]}],\"obfuscation\":\"t9egcKewVOXiQ6N\"}\n\n".to_string(),
            "data:{\"type\":\"response.output_text.done\",\"sequence_number\":5,\"item_id\":\"msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad\",\"output_index\":0,\"content_index\":0,\"text\":\"N\",\"logprobs\":[{\"bytes\":[78],\"logprob\":-2.9266366958618164,\"token\":\"N\",\"top_logprobs\":[{\"bytes\":[80,108,101,97,115,101],\"logprob\":-0.5516367554664612,\"token\":\"Please\"},{\"bytes\":[89],\"logprob\":-1.0516366958618164,\"token\":\"Y\"},{\"bytes\":[78],\"logprob\":-2.9266366958618164,\"token\":\"N\"},{\"bytes\":[83,117,114,101],\"logprob\":-4.551636695861816,\"token\":\"Sure\"},{\"bytes\":[67,111,117,108,100],\"logprob\":-5.176636695861816,\"token\":\"Could\"}]}]}\n\n".to_string(),
            "data:{\"type\":\"response.content_part.done\",\"sequence_number\":6,\"item_id\":\"msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"N\"}}\n\n".to_string(),
            "data:{\"type\":\"response.output_item.done\",\"sequence_number\":7,\"output_index\":0,\"item\":{\"id\":\"msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad\",\"type\":\"message\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[{\"bytes\":[78],\"logprob\":-2.926637,\"token\":\"N\",\"top_logprobs\":[{\"bytes\":[80,108,101,97,115,101],\"logprob\":-0.551637,\"token\":\"Please\"},{\"bytes\":[89],\"logprob\":-1.051637,\"token\":\"Y\"},{\"bytes\":[78],\"logprob\":-2.926637,\"token\":\"N\"},{\"bytes\":[83,117,114,101],\"logprob\":-4.551637,\"token\":\"Sure\"},{\"bytes\":[67,111,117,108,100],\"logprob\":-5.176637,\"token\":\"Could\"}]}],\"text\":\"N\"}],\"role\":\"assistant\"}}\n\n".to_string(),
            "data:{\"type\":\"response.completed\",\"sequence_number\":8,\"response\":{\"id\":\"resp_689cec4cf608819583c56813ccb0f5040f92af1765dd5aad\",\"object\":\"response\",\"created_at\":1755114572,\"status\":\"completed\",\"background\":false,\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":1024,\"max_tool_calls\":null,\"model\":\"gpt-4.1-nano-2025-04-14\",\"output\":[{\"id\":\"msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad\",\"type\":\"message\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[{\"bytes\":[78],\"logprob\":-2.926637,\"token\":\"N\",\"top_logprobs\":[{\"bytes\":[80,108,101,97,115,101],\"logprob\":-0.551637,\"token\":\"Please\"},{\"bytes\":[89],\"logprob\":-1.051637,\"token\":\"Y\"},{\"bytes\":[78],\"logprob\":-2.926637,\"token\":\"N\"},{\"bytes\":[83,117,114,101],\"logprob\":-4.551637,\"token\":\"Sure\"},{\"bytes\":[67,111,117,108,100],\"logprob\":-5.176637,\"token\":\"Could\"}]}],\"text\":\"N\"}],\"role\":\"assistant\"}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"prompt_cache_key\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"safety_identifier\":null,\"service_tier\":\"default\",\"store\":true,\"temperature\":1,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tools\":[],\"top_logprobs\":5,\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":2,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":14},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
        ]);
            let stream = stream_do(&mock, "gpt-4o", json!({"prompt": stream_test_prompt(), "providerOptions": {"openai": {"logprobs": 1}}})).await.unwrap();
            stream_assert_parts(
                &stream_collect(stream).await,
                &[
                    json!({
                        "type": "stream-start",
                        "warnings": [],
                    }),
                    json!({
                        "id": "resp_689cec4cf608819583c56813ccb0f5040f92af1765dd5aad",
                        "modelId": "gpt-4.1-nano-2025-04-14",
                        "timestamp": "2025-08-13T19:49:32.000Z",
                        "type": "response-metadata",
                    }),
                    json!({
                        "id": "msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad",
                            },
                        },
                        "type": "text-start",
                    }),
                    json!({
                        "delta": "N",
                        "id": "msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad",
                        "type": "text-delta",
                    }),
                    json!({
                        "id": "msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "msg_689cec4d46448195905a27fb9e12ff670f92af1765dd5aad",
                            },
                        },
                        "type": "text-end",
                    }),
                    json!({
                        "finishReason": {
                            "unified": "stop",
                        },
                        "providerMetadata": {
                            "openai": {
                                "logprobs": [
                                    [
                                        {
                                            "logprob": -2.9266366958618164,
                                            "token": "N",
                                            "top_logprobs": [
                                                {
                                                    "logprob": -0.5516367554664612,
                                                    "token": "Please",
                                                },
                                                {
                                                    "logprob": -1.0516366958618164,
                                                    "token": "Y",
                                                },
                                                {
                                                    "logprob": -2.9266366958618164,
                                                    "token": "N",
                                                },
                                                {
                                                    "logprob": -4.551636695861816,
                                                    "token": "Sure",
                                                },
                                                {
                                                    "logprob": -5.176636695861816,
                                                    "token": "Could",
                                                },
                                            ],
                                        },
                                    ],
                                ],
                                "responseId": "resp_689cec4cf608819583c56813ccb0f5040f92af1765dd5aad",
                                "serviceTier": "default",
                            },
                        },
                        "type": "finish",
                        "usage": {
                            "inputTokens": {
                                "cacheRead": 0,
                                "noCache": 12,
                                "total": 12,
                            },
                            "outputTokens": {
                                "reasoning": 0,
                                "text": 2,
                                "total": 2,
                            },
                            "raw": {
                                "input_tokens": 12,
                                "input_tokens_details": {
                                    "cached_tokens": 0,
                                },
                                "output_tokens": 2,
                                "output_tokens_details": {
                                    "reasoning_tokens": 0,
                                },
                                "total_tokens": 14,
                            },
                        },
                    }),
                ],
            );
        }

        mod custom_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › custom tool › should stream custom tool call
            #[tokio::test]
            async fn should_stream_custom_tool_call() {
                let mock =
                    sse_fixture_mock(include_str!("fixtures/openai-custom-tool.1.chunks.txt"));
                let stream = stream_do(&mock, "gpt-5.2-codex", json!({"tools": [{"type": "provider", "id": "openai.custom", "name": "write_sql", "args": {"description": "Write a SQL SELECT query to answer the user question.", "format": {"type": "grammar", "syntax": "regex", "definition": "SELECT .+"}}}], "prompt": stream_test_prompt(), "includeRawChunks": false})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &[
                        json!({
                            "type": "stream-start",
                            "warnings": [],
                        }),
                        json!({
                            "id": "resp_custom_tool_test_001",
                            "modelId": "gpt-5.2-codex",
                            "timestamp": "2025-03-06T10:42:10.000Z",
                            "type": "response-metadata",
                        }),
                        json!({
                            "id": "call_custom_sql_001",
                            "toolName": "write_sql",
                            "type": "tool-input-start",
                        }),
                        json!({
                            "delta": "SELECT * ",
                            "id": "call_custom_sql_001",
                            "type": "tool-input-delta",
                        }),
                        json!({
                            "delta": "FROM users ",
                            "id": "call_custom_sql_001",
                            "type": "tool-input-delta",
                        }),
                        json!({
                            "delta": "WHERE age > 25",
                            "id": "call_custom_sql_001",
                            "type": "tool-input-delta",
                        }),
                        json!({
                            "id": "call_custom_sql_001",
                            "type": "tool-input-end",
                        }),
                        json!({
                            "input": r#""SELECT * FROM users WHERE age > 25""#,
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "ct_abc123def456",
                                },
                            },
                            "toolCallId": "call_custom_sql_001",
                            "toolName": "write_sql",
                            "type": "tool-call",
                        }),
                        json!({
                            "finishReason": {
                                "unified": "tool-calls",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "responseId": "resp_custom_tool_test_001",
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "noCache": 50,
                                    "total": 50,
                                },
                                "outputTokens": {
                                    "reasoning": 0,
                                    "text": 20,
                                    "total": 20,
                                },
                                "raw": {
                                    "input_tokens": 50,
                                    "input_tokens_details": {
                                        "cached_tokens": 0,
                                    },
                                    "output_tokens": 20,
                                    "output_tokens_details": {
                                        "reasoning_tokens": 0,
                                    },
                                    "total_tokens": 70,
                                },
                            },
                        }),
                    ],
                );
            }
        }

        mod web_search_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › web search tool › should stream web search results (sources, tool calls, tool results)
            #[tokio::test]
            async fn should_stream_web_search_results_sources_tool_calls_tool_results() {
                let mock =
                    sse_fixture_mock(include_str!("fixtures/openai-web-search-tool.1.chunks.txt"));
                let stream = stream_do(&mock, "gpt-5-nano", json!({"tools": [{"type": "provider", "id": "openai.web_search", "name": "webSearch", "args": {}}], "prompt": stream_test_prompt()})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_web_search_results_sources_tool_calls_tool_res.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › web search tool › should handle streaming web search with action query field
            #[tokio::test]
            async fn should_handle_streaming_web_search_with_action_query_field() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"object\":\"response\",\"created_at\":1741630255,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-2025-04-16\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":\"auto\"},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"web_search\",\"search_context_size\":\"medium\"}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"web_search_call\",\"id\":\"ws_test\",\"status\":\"in_progress\",\"action\":{\"type\":\"search\",\"query\":\"Vercel AI SDK next version features\"}}}\n\n".to_string(),
                "data:{\"type\":\"response.web_search_call.in_progress\",\"output_index\":0,\"item_id\":\"ws_test\"}\n\n".to_string(),
                "data:{\"type\":\"response.web_search_call.searching\",\"output_index\":0,\"item_id\":\"ws_test\"}\n\n".to_string(),
                "data:{\"type\":\"response.web_search_call.completed\",\"output_index\":0,\"item_id\":\"ws_test\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"web_search_call\",\"id\":\"ws_test\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"Vercel AI SDK next version features\"}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"message\",\"id\":\"msg_test\",\"status\":\"in_progress\",\"role\":\"assistant\",\"content\":[]}}\n\n".to_string(),
                "data:{\"type\":\"response.content_part.added\",\"item_id\":\"msg_test\",\"output_index\":1,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\",\"annotations\":[]}}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_test\",\"output_index\":1,\"content_index\":0,\"delta\":\"Based on the search results, here are the upcoming features.\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.done\",\"item_id\":\"msg_test\",\"output_index\":1,\"content_index\":0,\"text\":\"Based on the search results, here are the upcoming features.\"}\n\n".to_string(),
                "data:{\"type\":\"response.content_part.done\",\"item_id\":\"msg_test\",\"output_index\":1,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"Based on the search results, here are the upcoming features.\",\"annotations\":[]}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"message\",\"id\":\"msg_test\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Based on the search results, here are the upcoming features.\",\"annotations\":[]}]}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"object\":\"response\",\"created_at\":1741630255,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-2025-04-16\",\"output\":[{\"type\":\"web_search_call\",\"id\":\"ws_test\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"Vercel AI SDK next version features\"}},{\"type\":\"message\",\"id\":\"msg_test\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Based on the search results, here are the upcoming features.\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":\"auto\"},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"web_search\",\"search_context_size\":\"medium\"}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":50,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":25,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":75},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data: [DONE]\n\n".to_string(),
            ]);
                let stream = stream_do(&mock, "gpt-5-nano", json!({"tools": [{"type": "provider", "id": "openai.web_search", "name": "webSearch", "args": {}}], "prompt": stream_test_prompt()})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_handle_streaming_web_search_with_action_query_field.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › web search tool › should handle streaming web search without action
            #[tokio::test]
            async fn should_handle_streaming_web_search_without_action() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_missing_action\",\"object\":\"response\",\"created_at\":1741630255,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-2025-04-16\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":\"auto\"},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"web_search\",\"search_context_size\":\"medium\"}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"web_search_call\",\"id\":\"ws_missing_action\",\"status\":\"in_progress\"}}\n\n".to_string(),
                "data:{\"type\":\"response.web_search_call.in_progress\",\"output_index\":0,\"item_id\":\"ws_missing_action\"}\n\n".to_string(),
                "data:{\"type\":\"response.web_search_call.completed\",\"output_index\":0,\"item_id\":\"ws_missing_action\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"web_search_call\",\"id\":\"ws_missing_action\",\"status\":\"completed\"}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_missing_action\",\"object\":\"response\",\"created_at\":1741630255,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-2025-04-16\",\"output\":[{\"type\":\"web_search_call\",\"id\":\"ws_missing_action\",\"status\":\"completed\"}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":\"auto\"},\"store\":true,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"web_search\",\"search_context_size\":\"medium\"}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":10,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":2,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":12},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data: [DONE]\n\n".to_string(),
            ]);
                let stream = stream_do(&mock, "gpt-5-nano", json!({"tools": [{"type": "provider", "id": "openai.web_search", "name": "webSearch", "args": {}}], "prompt": stream_test_prompt()})).await.unwrap();
                let parts = stream_collect(stream).await;
                let tool_result_part = stream_find(&parts, "tool-result");
                stream_assert_match(
                    &json!(tool_result_part),
                    &json!({"type": "tool-result", "toolName": "webSearch", "result": {}}),
                );
            }
        }

        mod tool_search_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › tool search tool › should stream hosted tool search call and output as provider-executed tool parts
            #[tokio::test]
            async fn should_stream_hosted_tool_search_call_and_output_as_provider_executed_tool_parts()
             {
                let mock =
                    sse_fixture_mock(include_str!("fixtures/openai-tool-search.1.chunks.txt"));
                let stream = stream_do(&mock, "gpt-5-nano", json!({"prompt": stream_test_prompt(), "tools": stream_hosted_tool_search_tools()})).await.unwrap();
                let hosted_tool_search_parts: Vec<Value> = stream_collect(stream)
                    .await
                    .into_iter()
                    .filter(|part| {
                        (part["type"] == "tool-call" || part["type"] == "tool-result")
                            && part["toolName"] == "toolSearch"
                    })
                    .map(stream_parse_tool_call_input)
                    .collect();
                stream_assert_parts(
                    &hosted_tool_search_parts,
                    &[
                        json!({
                            "input": {
                                "arguments": {
                                    "paths": [
                                        "get_weather",
                                    ],
                                },
                                "call_id": null,
                            },
                            "providerExecuted": true,
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "tsc_08a14073c7135dc10069aa686296c88190bff77ad137e79d59",
                                },
                            },
                            "toolCallId": "tsc_08a14073c7135dc10069aa686296c88190bff77ad137e79d59",
                            "toolName": "toolSearch",
                            "type": "tool-call",
                        }),
                        json!({
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "tso_08a14073c7135dc10069aa6862b1248190ba40cbba918ecfa2",
                                },
                            },
                            "result": {
                                "tools": [
                                    {
                                        "defer_loading": true,
                                        "description": "Get the current weather at a specific location",
                                        "name": "get_weather",
                                        "parameters": {
                                            "additionalProperties": false,
                                            "properties": {
                                                "location": {
                                                    "description": "The city and state, e.g. San Francisco, CA",
                                                    "type": "string",
                                                },
                                                "unit": {
                                                    "description": "Temperature unit",
                                                    "enum": [
                                                        "celsius",
                                                        "fahrenheit",
                                                    ],
                                                    "type": "string",
                                                },
                                            },
                                            "required": [
                                                "location",
                                                "unit",
                                            ],
                                            "type": "object",
                                        },
                                        "strict": true,
                                        "type": "function",
                                    },
                                ],
                            },
                            "toolCallId": "tsc_08a14073c7135dc10069aa686296c88190bff77ad137e79d59",
                            "toolName": "toolSearch",
                            "type": "tool-result",
                        }),
                    ],
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › tool search tool › should use the final tool search call_id when the streamed provisional id changes
            #[tokio::test]
            async fn should_use_the_final_tool_search_call_id_when_the_streamed_provisional_id_change()
             {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_tool_search_client\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-5.4\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":false,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"tool_search\",\"execution\":\"client\",\"description\":\"Search for tools\",\"parameters\":{\"type\":\"object\",\"properties\":{\"goal\":{\"type\":\"string\"}},\"required\":[\"goal\"],\"additionalProperties\":false}},{\"type\":\"function\",\"name\":\"get_weather\",\"description\":\"Get the current weather at a specific location\",\"parameters\":{\"type\":\"object\",\"properties\":{\"location\":{\"type\":\"string\"}},\"required\":[\"location\"],\"additionalProperties\":false},\"strict\":true,\"defer_loading\":true}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"tool_search_call\",\"id\":\"tsc_client_1\",\"execution\":\"client\",\"call_id\":\"call_provisional\",\"status\":\"completed\",\"arguments\":{\"goal\":\"Find the weather tool\"}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"tool_search_call\",\"id\":\"tsc_client_1\",\"execution\":\"client\",\"call_id\":\"call_final\",\"status\":\"completed\",\"arguments\":{\"goal\":\"Find the weather tool\"}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"tool_search_output\",\"id\":\"tso_client_1\",\"execution\":\"client\",\"call_id\":\"call_final\",\"status\":\"completed\",\"tools\":[{\"type\":\"function\",\"name\":\"get_weather\",\"description\":\"Get the current weather at a specific location\",\"parameters\":{\"type\":\"object\",\"properties\":{\"location\":{\"type\":\"string\"}},\"required\":[\"location\"],\"additionalProperties\":false},\"strict\":true,\"defer_loading\":true}]}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"tool_search_output\",\"id\":\"tso_client_1\",\"execution\":\"client\",\"call_id\":\"call_final\",\"status\":\"completed\",\"tools\":[{\"type\":\"function\",\"name\":\"get_weather\",\"description\":\"Get the current weather at a specific location\",\"parameters\":{\"type\":\"object\",\"properties\":{\"location\":{\"type\":\"string\"}},\"required\":[\"location\"],\"additionalProperties\":false},\"strict\":true,\"defer_loading\":true}]}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_tool_search_client\",\"object\":\"response\",\"created_at\":1741362087,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"instructions\":null,\"max_output_tokens\":null,\"model\":\"gpt-5.4\",\"output\":[{\"type\":\"tool_search_call\",\"id\":\"tsc_client_1\",\"execution\":\"client\",\"call_id\":\"call_final\",\"status\":\"completed\",\"arguments\":{\"goal\":\"Find the weather tool\"}},{\"type\":\"tool_search_output\",\"id\":\"tso_client_1\",\"execution\":\"client\",\"call_id\":\"call_final\",\"status\":\"completed\",\"tools\":[{\"type\":\"function\",\"name\":\"get_weather\",\"description\":\"Get the current weather at a specific location\",\"parameters\":{\"type\":\"object\",\"properties\":{\"location\":{\"type\":\"string\"}},\"required\":[\"location\"],\"additionalProperties\":false},\"strict\":true,\"defer_loading\":true}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":null,\"summary\":null},\"store\":false,\"temperature\":0,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[{\"type\":\"tool_search\",\"execution\":\"client\",\"description\":\"Search for tools\",\"parameters\":{\"type\":\"object\",\"properties\":{\"goal\":{\"type\":\"string\"}},\"required\":[\"goal\"],\"additionalProperties\":false}},{\"type\":\"function\",\"name\":\"get_weather\",\"description\":\"Get the current weather at a specific location\",\"parameters\":{\"type\":\"object\",\"properties\":{\"location\":{\"type\":\"string\"}},\"required\":[\"location\"],\"additionalProperties\":false},\"strict\":true,\"defer_loading\":true}],\"top_p\":1,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":0,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":0,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":0},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            ]);
                let stream = stream_do(&mock, "gpt-5.4", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.tool_search", "name": "toolSearch", "args": {"execution": "client", "description": "Search for tools", "parameters": {"type": "object", "properties": {"goal": {"type": "string"}}, "required": ["goal"], "additionalProperties": false}}}, {"type": "function", "name": "get_weather", "description": "Get the current weather at a specific location", "inputSchema": {"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"], "additionalProperties": false}, "strict": true, "providerOptions": {"openai": {"deferLoading": true}}}]})).await.unwrap();
                let tool_search_parts: Vec<Value> = stream_collect(stream)
                    .await
                    .into_iter()
                    .filter(|part| {
                        (part["type"] == "tool-input-start" && part["toolName"] == "toolSearch")
                            || (part["type"] == "tool-input-end" && part["id"] == "call_final")
                            || (part["type"] == "tool-call" && part["toolName"] == "toolSearch")
                            || (part["type"] == "tool-result" && part["toolName"] == "toolSearch")
                    })
                    .map(stream_parse_tool_call_input)
                    .collect();
                stream_assert_parts(
                    &tool_search_parts,
                    &[
                        json!({
                            "id": "call_final",
                            "toolName": "toolSearch",
                            "type": "tool-input-start",
                        }),
                        json!({
                            "id": "call_final",
                            "type": "tool-input-end",
                        }),
                        json!({
                            "input": {
                                "arguments": {
                                    "goal": "Find the weather tool",
                                },
                                "call_id": "call_final",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "tsc_client_1",
                                },
                            },
                            "toolCallId": "call_final",
                            "toolName": "toolSearch",
                            "type": "tool-call",
                        }),
                        json!({
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "tso_client_1",
                                },
                            },
                            "result": {
                                "tools": [
                                    {
                                        "defer_loading": true,
                                        "description": "Get the current weather at a specific location",
                                        "name": "get_weather",
                                        "parameters": {
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
                                        "strict": true,
                                        "type": "function",
                                    },
                                ],
                            },
                            "toolCallId": "call_final",
                            "toolName": "toolSearch",
                            "type": "tool-result",
                        }),
                    ],
                );
            }
        }

        mod client_tool_search_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › client tool search tool › should stream client tool search call as non-provider-executed tool parts
            #[tokio::test]
            async fn should_stream_client_tool_search_call_as_non_provider_executed_tool_parts() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-client-tool-search.1.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5.4", json!({"prompt": stream_test_prompt(), "tools": stream_client_tool_search_tools()})).await.unwrap();
                let all_parts = stream_collect(stream).await;
                let client_tool_search_parts: Vec<Value> = all_parts
                    .into_iter()
                    .filter(|part| {
                        (part["type"] == "tool-input-start" && part["toolName"] == "toolSearch")
                            || part["type"] == "tool-input-end"
                            || (part["type"] == "tool-call" && part["toolName"] == "toolSearch")
                    })
                    .map(stream_parse_tool_call_input)
                    .collect();
                stream_assert_parts(
                    &client_tool_search_parts,
                    &[
                        json!({
                            "id": "call_RWTIIVfxsJW9fecsg6fy23Dy",
                            "toolName": "toolSearch",
                            "type": "tool-input-start",
                        }),
                        json!({
                            "id": "call_RWTIIVfxsJW9fecsg6fy23Dy",
                            "type": "tool-input-end",
                        }),
                        json!({
                            "input": {
                                "arguments": {
                                    "goal": "Find a tool that can provide current weather information for San Francisco.",
                                },
                                "call_id": "call_RWTIIVfxsJW9fecsg6fy23Dy",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "tsc_05147bbe356953b60069ab673598f88196b499a756b524b64c",
                                },
                            },
                            "toolCallId": "call_RWTIIVfxsJW9fecsg6fy23Dy",
                            "toolName": "toolSearch",
                            "type": "tool-call",
                        }),
                    ],
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › client tool search tool › should not include providerExecuted flag for client tool search stream parts
            #[tokio::test]
            async fn should_not_include_provider_executed_flag_for_client_tool_search_stream_parts()
            {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-client-tool-search.1.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5.4", json!({"prompt": stream_test_prompt(), "tools": stream_client_tool_search_tools()})).await.unwrap();
                let all_parts = stream_collect(stream).await;
                let tool_input_start = all_parts.iter().find(|part| {
                    part["type"] == "tool-input-start" && part["toolName"] == "toolSearch"
                });
                let tool_call = all_parts
                    .iter()
                    .find(|part| part["type"] == "tool-call" && part["toolName"] == "toolSearch");
                assert!(
                    tool_input_start
                        .and_then(|part| part.get("providerExecuted"))
                        .is_none()
                );
                assert!(
                    tool_call
                        .and_then(|part| part.get("providerExecuted"))
                        .is_none()
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › client tool search tool › should use final call_id from done chunk (not provisional from added chunk)
            #[tokio::test]
            async fn should_use_final_call_id_from_done_chunk_not_provisional_from_added_chunk() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-client-tool-search.1.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5.4", json!({"prompt": stream_test_prompt(), "tools": stream_client_tool_search_tools()})).await.unwrap();
                let all_parts = stream_collect(stream).await;
                let tool_call = all_parts
                    .iter()
                    .find(|part| part["type"] == "tool-call" && part["toolName"] == "toolSearch")
                    .expect("tool search call");
                let parsed_input: Value =
                    serde_json::from_str(tool_call["input"].as_str().unwrap()).unwrap();
                assert_eq!(tool_call["toolCallId"], "call_RWTIIVfxsJW9fecsg6fy23Dy");
                assert_eq!(parsed_input["call_id"], "call_RWTIIVfxsJW9fecsg6fy23Dy");
                assert_eq!(tool_call["toolCallId"], parsed_input["call_id"]);
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › client tool search tool › should stream step 2 function call after client tool search output
            #[tokio::test]
            async fn should_stream_step_2_function_call_after_client_tool_search_output() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-client-tool-search.2.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5.4", json!({"prompt": stream_test_prompt(), "tools": stream_client_tool_search_tools()})).await.unwrap();
                let all_parts = stream_collect(stream).await;
                let function_call_part = all_parts
                    .iter()
                    .find(|part| part["type"] == "tool-call" && part["toolName"] == "get_weather");
                assert!(function_call_part.is_some());
                let function_call_part = function_call_part.unwrap();
                assert_eq!(
                    function_call_part["toolCallId"],
                    "call_Q7pq6EfVGRnauPLWSSYBGJ1l"
                );
                assert_eq!(
                    serde_json::from_str::<Value>(function_call_part["input"].as_str().unwrap())
                        .unwrap(),
                    json!({"location": "San Francisco, CA", "unit": "fahrenheit"})
                );
            }
        }

        mod file_search_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › file search tool › should stream file search results without results include
            #[tokio::test]
            async fn should_stream_file_search_results_without_results_include() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-file-search-tool.1.chunks.txt"
                ));
                let result = stream_do(&mock, "gpt-5-nano", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.file_search", "name": "fileSearch", "args": {"vectorStoreIds": ["vs_68caad8bd5d88191ab766cf043d89a18"]}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_file_search_results_without_results_include.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › file search tool › should stream file search results with results include
            #[tokio::test]
            async fn should_stream_file_search_results_with_results_include() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-file-search-tool.2.chunks.txt"
                ));
                let result = stream_do(&mock, "gpt-5-nano", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.file_search", "name": "fileSearch", "args": {"vectorStoreIds": ["vs_68caad8bd5d88191ab766cf043d89a18"]}}], "providerOptions": {"openai": {"include": ["file_search_call.results"]}}})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_file_search_results_with_results_include.json"
                    )),
                );
            }
        }

        mod code_interpreter_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › code interpreter tool › should stream code interpreter results with annotations
            #[tokio::test]
            async fn should_stream_code_interpreter_results_with_annotations() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-code-interpreter-tool.1.chunks.txt"
                ));
                let stream_result = stream_do(&mock, "gpt-5-nano", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.code_interpreter", "name": "codeExecution", "args": {}}]})).await.unwrap();
                let stream_parts = stream_collect(stream_result).await;
                stream_assert_parts(
                    &stream_parts,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_code_interpreter_results_with_annotations.json"
                    )),
                );
                let source_parts: Vec<&Value> = stream_parts
                    .iter()
                    .filter(|part| part["type"] == "source")
                    .collect();
                assert_eq!(source_parts.len(), 1);
                stream_assert_match(
                    source_parts[0],
                    &json!({
                        "type": "source",
                        "sourceType": "document",
                        "filename": "roll2dice_sums_10000.csv",
                        "mediaType": "text/plain",
                        "providerMetadata": {
                            "openai": {
                                "fileId": "cfile_68c2e7084ab48191a67824aa1f4c90f1",
                                "containerId": "cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9",
                            },
                        },
                    }),
                );
                assert_eq!(
                    source_parts[0]["providerMetadata"]
                        .as_object()
                        .unwrap()
                        .len(),
                    1
                );
            }
        }

        mod image_generation_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › image generation tool › should stream code image generation results
            #[tokio::test]
            async fn should_stream_code_image_generation_results() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-image-generation-tool.1.chunks.txt"
                ));
                let result = stream_do(&mock, "gpt-5-nano", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.image_generation", "name": "generateImage", "args": {}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_code_image_generation_results.json"
                    )),
                );
            }
        }

        mod local_shell_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › local shell tool › should stream code local shell results
            #[tokio::test]
            async fn should_stream_code_local_shell_results() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-local-shell-tool.1.chunks.txt"
                ));
                let result = stream_do(&mock, "gpt-5-codex", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.local_shell", "name": "shell", "args": {}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_code_local_shell_results.json"
                    )),
                );
            }
        }

        mod shell_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › shell tool › should stream shell tool results
            #[tokio::test]
            async fn should_stream_shell_tool_results() {
                let mock =
                    sse_fixture_mock(include_str!("fixtures/openai-shell-tool.1.chunks.txt"));
                let result = stream_do(&mock, "gpt-5.1", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.shell", "name": "shell", "args": {}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_shell_tool_results.json"
                    )),
                );
            }
        }

        mod shell_tool_with_container_no_skills {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › shell tool with container (no skills) › should stream shell tool results with container execution
            #[tokio::test]
            async fn should_stream_shell_tool_results_with_container_execution() {
                let mock =
                    sse_fixture_mock(include_str!("fixtures/openai-shell-container.1.chunks.txt"));
                let result = stream_do(&mock, "gpt-5.2", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.shell", "name": "shell", "args": {"environment": {"type": "containerAuto"}}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_shell_tool_results_with_container_execution.json"
                    )),
                );
            }
        }

        mod shell_tool_with_container_multiturn {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › shell tool with container multiturn › should stream follow-up response after shell tool history
            #[tokio::test]
            async fn should_stream_follow_up_response_after_shell_tool_history() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-shell-container-multiturn.1.chunks.txt"
                ));
                let multiturn_prompt = json!([{"role": "user", "content": [{"type": "text", "text": "Run uname -a"}]}, {"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_abc123def456ghi789jkl012", "toolName": "shell", "input": r#"{"action":{"commands":["uname -a"]}}"#, "providerExecuted": true, "providerOptions": {"openai": {"itemId": "sh_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e50"}}}, {"type": "tool-result", "toolCallId": "call_abc123def456ghi789jkl012", "toolName": "shell", "output": {"type": "json", "value": {"output": [{"stdout": "Linux container-host 6.1.0 #1 SMP x86_64 GNU/Linux\n", "stderr": "", "outcome": {"type": "exit", "exitCode": 0}}]}}}, {"type": "text", "text": "Linux container-host 6.1.0 #1 SMP x86_64 GNU/Linux", "providerOptions": {"openai": {"itemId": "msg_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e52"}}}]}, {"role": "user", "content": [{"type": "text", "text": "What architecture do you run in?"}]}]);
                let result = stream_do(&mock, "gpt-5.2", json!({"prompt": multiturn_prompt, "tools": [{"type": "provider", "id": "openai.shell", "name": "shell", "args": {"environment": {"type": "containerAuto"}}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_follow_up_response_after_shell_tool_history.json"
                    )),
                );
            }
        }

        mod shell_tool_with_local_multiturn {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › shell tool with local multiturn › should stream follow-up response after local shell tool history
            #[tokio::test]
            async fn should_stream_follow_up_response_after_local_shell_tool_history() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-shell-local-multiturn.1.chunks.txt"
                ));
                let multiturn_prompt = json!([{"role": "user", "content": [{"type": "text", "text": "Run uname -a"}]}, {"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "call_abc123def456ghi789jkl012", "toolName": "shell", "input": r#"{"action":{"commands":["uname -a"]}}"#, "providerOptions": {"openai": {"itemId": "sh_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e50"}}}]}, {"role": "tool", "content": [{"type": "tool-result", "toolCallId": "call_abc123def456ghi789jkl012", "toolName": "shell", "output": {"type": "json", "value": {"output": [{"stdout": "Darwin mac-host 24.6.0 Darwin Kernel Version 24.6.0 root:xnu-11417.60.45.601.5~1/RELEASE_ARM64_T6041 arm64\n", "stderr": "", "outcome": {"type": "exit", "exitCode": 0}}]}}}]}, {"role": "assistant", "content": [{"type": "text", "text": "Darwin mac-host 24.6.0 Darwin Kernel Version 24.6.0 arm64", "providerOptions": {"openai": {"itemId": "msg_0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e52"}}}]}, {"role": "user", "content": [{"type": "text", "text": "What architecture do you run in?"}]}]);
                let result = stream_do(&mock, "gpt-5.2", json!({"prompt": multiturn_prompt, "tools": [{"type": "provider", "id": "openai.shell", "name": "shell", "args": {}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_follow_up_response_after_local_shell_tool_hist.json"
                    )),
                );
            }
        }

        mod shell_tool_with_environment {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › shell tool with environment › should stream shell tool results with server-executed output
            #[tokio::test]
            async fn should_stream_shell_tool_results_with_server_executed_output() {
                let mock =
                    sse_fixture_mock(include_str!("fixtures/openai-shell-skills.1.chunks.txt"));
                let result = stream_do(&mock, "gpt-5.2", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.shell", "name": "shell", "args": {"environment": {"type": "containerAuto"}}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(result).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_shell_tool_results_with_server_executed_output.json"
                    )),
                );
            }
        }

        mod mcp_tool {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › mcp tool › should stream mcp tool results (list tools, tool calls, tool results)
            #[tokio::test]
            async fn should_stream_mcp_tool_results_list_tools_tool_calls_tool_results() {
                let mock = sse_fixture_mock(include_str!("fixtures/openai-mcp-tool.1.chunks.txt"));
                let stream = stream_do(&mock, "gpt-5-mini", json!({"tools": [{"type": "provider", "id": "openai.mcp", "name": "MCP", "args": {"serverLabel": "dmcp", "serverUrl": "https://mcp.exa.ai/mcp", "serverDescription": "A web-search API for AI agents"}}], "prompt": stream_test_prompt()})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_mcp_tool_results_list_tools_tool_calls_tool_re.json"
                    )),
                );
            }
        }

        mod mcp_tool_approval {
            use super::*;
            /// Upstream's `MCP_TOOL`.
            fn mcp_tool() -> Value {
                json!({"type": "provider", "id": "openai.mcp", "name": "MCP", "args": {"serverLabel": "zip1", "serverUrl": "https://zip1.io/mcp", "serverDescription": "Link shortener", "requireApproval": "always"}})
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › mcp tool approval › should stream tool-call and tool-approval-request when model requests approval (turn 1)
            #[tokio::test]
            async fn should_stream_tool_call_and_tool_approval_request_when_model_requests_approval_t()
             {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-mcp-tool-approval.1.chunks.txt"
                ));
                let stream = stream_do(
                    &mock,
                    "gpt-5-mini",
                    json!({"prompt": stream_test_prompt(), "tools": [mcp_tool()]}),
                )
                .await
                .unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_tool_call_and_tool_approval_request_when_model.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › mcp tool approval › should stream text response after denial (turn 2)
            #[tokio::test]
            async fn should_stream_text_response_after_denial_turn_2() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-mcp-tool-approval.2.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5-mini", json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "shorten ai-sdk.dev"}]}, {"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "mcpr_04a97b4fce127879006949a83ac9308195a7f7b69ea82e91fe", "toolName": "mcp.create_short_url", "input": {"url": "https://ai-sdk.dev/"}, "providerExecuted": true}]}, {"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "mcpr_04a97b4fce127879006949a83ac9308195a7f7b69ea82e91fe", "approved": false}]}], "tools": [mcp_tool()]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_text_response_after_denial_turn_2.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › mcp tool approval › should stream new tool-approval-request when retrying (turn 3)
            #[tokio::test]
            async fn should_stream_new_tool_approval_request_when_retrying_turn_3() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-mcp-tool-approval.3.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5-mini", json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "shorten ai-sdk.dev"}]}, {"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "mcpr_04a97b4fce127879006949a83ac9308195a7f7b69ea82e91fe", "toolName": "mcp.create_short_url", "input": {"url": "https://ai-sdk.dev/"}, "providerExecuted": true}]}, {"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "mcpr_04a97b4fce127879006949a83ac9308195a7f7b69ea82e91fe", "approved": false}]}, {"role": "assistant", "content": [{"type": "text", "text": "The tool was not approved."}]}, {"role": "user", "content": [{"type": "text", "text": "try again"}]}], "tools": [mcp_tool()]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_new_tool_approval_request_when_retrying_turn_3.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › mcp tool approval › should stream tool-call with result after approval (turn 4)
            #[tokio::test]
            async fn should_stream_tool_call_with_result_after_approval_turn_4() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-mcp-tool-approval.4.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5-mini", json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "shorten ai-sdk.dev"}]}, {"role": "assistant", "content": [{"type": "tool-call", "toolCallId": "mcpr_04a97b4fce127879006949a8672ac081959f95aa8ceedb7cd9", "toolName": "mcp.create_short_url", "input": {"url": "https://ai-sdk.dev/"}, "providerExecuted": true}]}, {"role": "tool", "content": [{"type": "tool-approval-response", "approvalId": "mcpr_04a97b4fce127879006949a8672ac081959f95aa8ceedb7cd9", "approved": true}]}], "tools": [mcp_tool()]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_tool_call_with_result_after_approval_turn_4.json"
                    )),
                );
            }
        }

        mod errors {
            use super::*;

            /// `await expect(doStream(...)).rejects`: the rejection as its JSON.
            fn stream_rejection(result: Result<StreamResult, AiMuxError>) -> Value {
                match result {
                    Err(error) => serde_json::to_value(&error).expect("error JSON"),
                    Ok(_) => panic!("doStream resolved; upstream expects it to reject"),
                }
            }

            type SseBody = Result<bytes::Bytes, aimux_provider_utils::FetchError>;

            /// Upstream's `TestResponseController` with a `controlled-stream`
            /// response: the test writes the event-stream body while the call
            /// reads it.
            struct SseControlledFetch {
                body: std::sync::Mutex<Option<futures::channel::mpsc::UnboundedReceiver<SseBody>>>,
            }

            struct SseController(futures::channel::mpsc::UnboundedSender<SseBody>);

            impl SseController {
                fn write(&self, chunk: &str) {
                    self.0
                        .unbounded_send(Ok(bytes::Bytes::from(chunk.to_owned())))
                        .expect("body open");
                }

                fn close(self) {}
            }

            impl SseControlledFetch {
                fn start() -> (SseController, aimux_provider_utils::FetchFunction) {
                    let (sender, receiver) = futures::channel::mpsc::unbounded();
                    let fetch = Arc::new(Self {
                        body: std::sync::Mutex::new(Some(receiver)),
                    });
                    (SseController(sender), fetch)
                }
            }

            #[async_trait::async_trait]
            impl aimux_provider_utils::Fetch for SseControlledFetch {
                async fn fetch(
                    &self,
                    request: aimux_provider_utils::FetchRequest,
                ) -> Result<aimux_provider_utils::FetchResponse, aimux_provider_utils::FetchError>
                {
                    let body = self.body.lock().unwrap().take().expect("one request");
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        reqwest::header::CONTENT_TYPE,
                        reqwest::header::HeaderValue::from_static("text/event-stream"),
                    );
                    Ok(aimux_provider_utils::FetchResponse {
                        status: reqwest::StatusCode::OK,
                        headers,
                        url: request.url,
                        body: body.boxed(),
                    })
                }
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › errors › should throw an api error when the stream errors before output starts
            #[tokio::test]
            async fn should_throw_an_api_error_when_the_stream_errors_before_output_starts() {
                let mock = sse_fixture_mock(include_str!("fixtures/openai-error.1.chunks.txt"));
                let error = stream_rejection(
                    stream_do(
                        &mock,
                        "gpt-4o-mini",
                        json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
                    )
                    .await,
                );
                stream_assert_match(
                    &error,
                    &json!({
                        "message": "You exceeded your current quota, please check your plan and billing details. For more information on this error, read the docs: https://platform.openai.com/docs/guides/error-codes/api-errors.",
                        "statusCode": 429,
                        "isRetryable": false,
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › errors › should throw a retryable api error for nested error events with a null code before output starts
            #[tokio::test]
            async fn should_throw_a_retryable_api_error_for_nested_error_events_with_a_null_code_befo()
             {
                let mock = sse_mock(vec![
                "data:{\"type\":\"error\",\"sequence_number\":2,\"error\":{\"type\":\"server_error\",\"code\":null,\"message\":\"Sorry, something went wrong.\",\"param\":null}}\n\n".to_string(),
            ]);
                let error = stream_rejection(
                    stream_do(
                        &mock,
                        "gpt-5",
                        json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
                    )
                    .await,
                );
                stream_assert_match(
                    &error,
                    &json!({"message": "Sorry, something went wrong.", "statusCode": 500, "isRetryable": true}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › errors › should throw an api error for documented top-level error events before output starts
            #[tokio::test]
            async fn should_throw_an_api_error_for_documented_top_level_error_events_before_output_st()
             {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"sequence_number\":0,\"response\":{\"id\":\"resp_error_top_level\",\"created_at\":1741269019,\"model\":\"gpt-4o-2024-07-18\",\"service_tier\":null}}\n\n".to_string(),
                "data:{\"type\":\"error\",\"sequence_number\":1,\"code\":\"rate_limit_exceeded\",\"message\":\"Rate limit reached\",\"param\":null}\n\n".to_string(),
            ]);
                let error = stream_rejection(
                    stream_do(
                        &mock,
                        "gpt-4o-mini",
                        json!({"prompt": stream_test_prompt(), "includeRawChunks": true}),
                    )
                    .await,
                );
                stream_assert_match(
                    &error,
                    &json!({"message": "Rate limit reached", "statusCode": 429, "isRetryable": true}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › errors › should make the stream available after response.in_progress without waiting for the first output token
            #[tokio::test]
            async fn should_make_the_stream_available_after_response_in_progress_without_waiting_for()
             {
                // Upstream's `TestResponseController`: a body the test writes to while
                // the call reads it.
                let (controller, fetch) = SseControlledFetch::start();
                let model = OpenAIResponsesModel::from_config(
                    "gpt-4o-mini".into(),
                    OpenAIModelConfig::fixed(
                        "openai".into(),
                        "https://api.openai.com/v1".into(),
                        ProviderHeaders::bearer(
                            Credential::Explicit(Resolvable::Value("APIKEY".into())),
                            Vec::new(),
                            None,
                        ),
                        Some(fetch),
                        None,
                    ),
                );
                controller.write("data:{\"type\":\"response.created\",\"sequence_number\":0,\"response\":{\"id\":\"resp_in_progress_early\",\"created_at\":1741269019,\"model\":\"gpt-4o-2024-07-18\",\"service_tier\":null}}\n\n");
                controller.write("data:{\"type\":\"response.in_progress\",\"sequence_number\":1,\"response\":{\"id\":\"resp_in_progress_early\",\"created_at\":1741269019,\"model\":\"gpt-4o-2024-07-18\",\"service_tier\":null}}\n\n");
                // `await streamPromise` before any output event is written.
                let stream = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    stream_do_with(
                        &model,
                        json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
                    ),
                )
                .await
                .expect("doStream resolves before the first output token")
                .unwrap();
                controller.write("data:{\"type\":\"response.output_item.added\",\"sequence_number\":2,\"output_index\":0,\"item\":{\"id\":\"msg_in_progress_early\",\"type\":\"message\"}}\n\n");
                controller.write("data:{\"type\":\"response.output_text.delta\",\"sequence_number\":3,\"item_id\":\"msg_in_progress_early\",\"output_index\":0,\"delta\":\"Hello\"}\n\n");
                controller.write("data:{\"type\":\"response.completed\",\"sequence_number\":4,\"response\":{\"id\":\"resp_in_progress_early\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"model\":\"gpt-4o-2024-07-18\",\"output\":[],\"service_tier\":null,\"usage\":{\"input_tokens\":10,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":1,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":11}}}\n\n");
                controller.close();
                let events = stream_collect(stream).await;
                assert!(events.contains(&json!({
                    "type": "response-metadata",
                    "id": "resp_in_progress_early",
                    "modelId": "gpt-4o-2024-07-18",
                    "timestamp": "2025-03-06T13:50:19.000Z",
                })));
                assert!(events.contains(&json!({
                    "type": "text-delta",
                    "id": "msg_in_progress_early",
                    "delta": "Hello",
                })));
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › errors › should throw an api error when response.failed arrives before output starts
            #[tokio::test]
            async fn should_throw_an_api_error_when_response_failed_arrives_before_output_starts() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"sequence_number\":0,\"response\":{\"id\":\"resp_failed_before_output\",\"created_at\":1741269019,\"model\":\"gpt-4o-2024-07-18\",\"service_tier\":null}}\n\n".to_string(),
                "data:{\"type\":\"response.failed\",\"sequence_number\":1,\"response\":{\"error\":{\"code\":\"server_error\",\"message\":\"response failed\"},\"incomplete_details\":null,\"usage\":null,\"service_tier\":null}}\n\n".to_string(),
            ]);
                let error = stream_rejection(
                    stream_do(
                        &mock,
                        "gpt-4o-mini",
                        json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
                    )
                    .await,
                );
                stream_assert_match(
                    &error,
                    &json!({"message": "response failed", "statusCode": 500, "isRetryable": true}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › errors › should use usage and raw finish reason from a late response.failed event
            #[tokio::test]
            async fn should_use_usage_and_raw_finish_reason_from_a_late_response_failed_event() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"sequence_number\":0,\"response\":{\"id\":\"resp_failed_with_reason\",\"created_at\":1741269019,\"model\":\"gpt-4o-2024-07-18\",\"service_tier\":null}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"sequence_number\":1,\"output_index\":0,\"item\":{\"id\":\"msg_failed_with_reason\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"error\",\"sequence_number\":2,\"error\":{\"type\":\"server_error\",\"code\":\"server_error\",\"message\":\"response failed\",\"param\":null}}\n\n".to_string(),
                "data:{\"type\":\"response.failed\",\"sequence_number\":3,\"response\":{\"error\":{\"code\":\"server_error\",\"message\":\"response failed\"},\"incomplete_details\":{\"reason\":\"max_output_tokens\"},\"usage\":{\"input_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":2,\"future_input_detail\":{\"tokens\":5}},\"output_tokens\":8,\"output_tokens_details\":{\"reasoning_tokens\":3,\"future_output_detail\":[\"preserved\"]},\"total_tokens\":20,\"future_usage_field\":{\"value\":true}},\"service_tier\":null}}\n\n".to_string(),
            ]);
                let stream = stream_do(
                    &mock,
                    "gpt-4o-mini",
                    json!({"prompt": stream_test_prompt(), "includeRawChunks": false}),
                )
                .await
                .unwrap();
                let mut events = stream_collect(stream).await;
                // Upstream's error part holds a `ProviderStreamError`
                // (`createOpenAIProviderStreamError`); aimux carries the same
                // frame as an `ApiCallError`, whose `providerCode` is
                // upstream's `code` and which has no `type` field. Its fields
                // are checked here, then the part is compared as upstream's.
                let expected_error = json!({
                    "code": "server_error",
                    "data": {
                        "error": {
                            "code": "server_error",
                            "message": "response failed",
                            "param": null,
                            "type": "server_error",
                        },
                        "sequence_number": 2,
                        "type": "error",
                    },
                    "isRetryable": true,
                    "message": "response failed",
                    "statusCode": 500,
                    "type": "server_error",
                });
                stream_assert_match(
                    &events[3],
                    &json!({
                        "type": "error",
                        "error": {
                            "providerCode": expected_error["code"],
                            "data": expected_error["data"],
                            "isRetryable": expected_error["isRetryable"],
                            "message": expected_error["message"],
                            "statusCode": expected_error["statusCode"],
                        },
                    }),
                );
                events[3]["error"] = expected_error;
                stream_assert_parts(
                    &events,
                    &[
                        json!({
                            "type": "stream-start",
                            "warnings": [],
                        }),
                        json!({
                            "id": "resp_failed_with_reason",
                            "modelId": "gpt-4o-2024-07-18",
                            "timestamp": "2025-03-06T13:50:19.000Z",
                            "type": "response-metadata",
                        }),
                        json!({
                            "id": "msg_failed_with_reason",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_failed_with_reason",
                                },
                            },
                            "type": "text-start",
                        }),
                        json!({
                            "error": {
                                "code": "server_error",
                                "data": {
                                    "error": {
                                        "code": "server_error",
                                        "message": "response failed",
                                        "param": null,
                                        "type": "server_error",
                                    },
                                    "sequence_number": 2,
                                    "type": "error",
                                },
                                "isRetryable": true,
                                "message": "response failed",
                                "statusCode": 500,
                                "type": "server_error",
                            },
                            "type": "error",
                        }),
                        json!({
                            "finishReason": {
                                "raw": "max_output_tokens",
                                "unified": "length",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "responseId": "resp_failed_with_reason",
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 2,
                                    "noCache": 10,
                                    "total": 12,
                                },
                                "outputTokens": {
                                    "reasoning": 3,
                                    "text": 5,
                                    "total": 8,
                                },
                                "raw": {
                                    "future_usage_field": {
                                        "value": true,
                                    },
                                    "input_tokens": 12,
                                    "input_tokens_details": {
                                        "cached_tokens": 2,
                                        "future_input_detail": {
                                            "tokens": 5,
                                        },
                                    },
                                    "output_tokens": 8,
                                    "output_tokens_details": {
                                        "future_output_detail": [
                                            "preserved",
                                        ],
                                        "reasoning_tokens": 3,
                                    },
                                    "total_tokens": 20,
                                },
                            },
                        }),
                    ],
                );
            }
        }

        mod reasoning {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › reasoning › should correlate rotated item ids by output index
            #[tokio::test]
            async fn should_correlate_rotated_item_ids_by_output_index() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/github-copilot-id-rotation.1.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5.3-codex", json!({"prompt": stream_test_prompt(), "providerOptions": {"openai": {"reasoningEffort": "low", "reasoningSummary": "detailed", "store": false}}, "includeRawChunks": false})).await.unwrap();
                let stream_parts = stream_collect(stream).await;
                let errors: Vec<&Value> = stream_parts
                    .iter()
                    .filter(|part| part["type"] == "error")
                    .collect();
                assert_eq!(errors, Vec::<&Value>::new());
                let part_ids = |kinds: [&str; 3]| -> std::collections::BTreeSet<String> {
                    stream_parts
                        .iter()
                        .filter(|part| kinds.iter().any(|kind| part["type"] == *kind))
                        .map(|part| part["id"].as_str().expect("part id").to_owned())
                        .collect()
                };
                assert_eq!(
                    part_ids(["reasoning-start", "reasoning-delta", "reasoning-end"]),
                    std::collections::BTreeSet::from(["capture-id-3:0".to_owned()]),
                );
                assert_eq!(
                    part_ids(["text-start", "text-delta", "text-end"]),
                    std::collections::BTreeSet::from(["capture-id-9".to_owned()]),
                );
                let text: String = stream_parts
                    .iter()
                    .filter(|part| part["type"] == "text-delta")
                    .map(|part| part["delta"].as_str().expect("delta"))
                    .collect();
                assert_eq!(
                    text,
                    "There are **3** letter **“r”**s in **“strawberry.”**\n\n".to_owned()
                        + "Breakdown: **s t r a w b e r r y**  \n"
                        + "You can see **r** at positions **3, 8, and 9**.",
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › reasoning › should handle reasoning with summary
            #[tokio::test]
            async fn should_handle_reasoning_with_summary() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0,\"delta\":\"**Exploring burrito origins**\\n\\nThe user is\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0,\"delta\":\" curious about the debate regarding Taqueria La Cumbre and El Farolito.\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1,\"delta\":\"**Investigating burrito origins**\\n\\nThere's a fascinating debate\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1,\"delta\":\" about who created the Mission burrito.\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\"answer\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\" text\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"**Exploring burrito origins**\\n\\nThe user is curious about the debate regarding Taqueria La Cumbre and El Farolito.\"},{\"type\":\"summary_text\",\"text\":\"**Investigating burrito origins**\\n\\nThere's a fascinating debate about who created the Mission burrito.\"}]},{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer text\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":34,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":538,\"output_tokens_details\":{\"reasoning_tokens\":320},\"total_tokens\":572},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            ]);
                let stream = stream_do(&mock, "o3-mini", json!({"prompt": stream_test_prompt(), "providerOptions": {"openai": {"reasoningEffort": "low", "reasoningSummary": "auto"}}, "includeRawChunks": false})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &[
                        json!({
                            "type": "stream-start",
                            "warnings": [],
                        }),
                        json!({
                            "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                            "modelId": "o3-mini-2025-01-31",
                            "timestamp": "2025-03-06T13:50:19.000Z",
                            "type": "response-metadata",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "delta": "**Exploring burrito origins**\n\nThe user is",
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "delta": " curious about the debate regarding Taqueria La Cumbre and El Farolito.",
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "delta": "**Investigating burrito origins**\n\nThere's a fascinating debate",
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "delta": " about who created the Mission burrito.",
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-start",
                        }),
                        json!({
                            "delta": "answer",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "delta": " text",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-end",
                        }),
                        json!({
                            "finishReason": {
                                "unified": "stop",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "noCache": 34,
                                    "total": 34,
                                },
                                "outputTokens": {
                                    "reasoning": 320,
                                    "text": 218,
                                    "total": 538,
                                },
                                "raw": {
                                    "input_tokens": 34,
                                    "input_tokens_details": {
                                        "cached_tokens": 0,
                                    },
                                    "output_tokens": 538,
                                    "output_tokens_details": {
                                        "reasoning_tokens": 320,
                                    },
                                    "total_tokens": 572,
                                },
                            },
                        }),
                    ],
                );
                stream_assert_match(
                    &mock.seen()[0].json_body(),
                    &json!({"model": "o3-mini", "reasoning": { "effort": "low", "summary": "auto"}, "stream": true}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › reasoning › should handle reasoning with empty summary
            #[tokio::test]
            async fn should_handle_reasoning_with_empty_summary() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\"answer\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\" text\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"summary\":[]},{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer text\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":34,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":538,\"output_tokens_details\":{\"reasoning_tokens\":320},\"total_tokens\":572},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            ]);
                let stream = stream_do(&mock, "o3-mini", json!({"prompt": stream_test_prompt(), "providerOptions": {"openai": {"reasoningEffort": "low", "reasoningSummary": null}}, "includeRawChunks": false})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &[
                        json!({
                            "type": "stream-start",
                            "warnings": [],
                        }),
                        json!({
                            "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                            "modelId": "o3-mini-2025-01-31",
                            "timestamp": "2025-03-06T13:50:19.000Z",
                            "type": "response-metadata",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-start",
                        }),
                        json!({
                            "delta": "answer",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "delta": " text",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-end",
                        }),
                        json!({
                            "finishReason": {
                                "unified": "stop",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "noCache": 34,
                                    "total": 34,
                                },
                                "outputTokens": {
                                    "reasoning": 320,
                                    "text": 218,
                                    "total": 538,
                                },
                                "raw": {
                                    "input_tokens": 34,
                                    "input_tokens_details": {
                                        "cached_tokens": 0,
                                    },
                                    "output_tokens": 538,
                                    "output_tokens_details": {
                                        "reasoning_tokens": 320,
                                    },
                                    "total_tokens": 572,
                                },
                            },
                        }),
                    ],
                );
                stream_assert_match(
                    &mock.seen()[0].json_body(),
                    &json!({"model": "o3-mini", "reasoning": { "effort": "low"}, "stream": true}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › reasoning › should handle encrypted content with summary
            #[tokio::test]
            async fn should_handle_encrypted_content_with_summary() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"encrypted_content\":\"encrypted_reasoning_data_abc123\"}}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0,\"delta\":\"**Exploring burrito origins**\\n\\nThe user is\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0,\"delta\":\" curious about the debate regarding Taqueria La Cumbre and El Farolito.\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1,\"delta\":\"**Investigating burrito origins**\\n\\nThere's a fascinating debate\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1,\"delta\":\" about who created the Mission burrito.\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"encrypted_content\":\"encrypted_reasoning_data_final_def456\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\"answer\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\" text\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"encrypted_content\":\"encrypted_reasoning_data_final_def456\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"**Exploring burrito origins**\\n\\nThe user is curious about the debate regarding Taqueria La Cumbre and El Farolito.\"},{\"type\":\"summary_text\",\"text\":\"**Investigating burrito origins**\\n\\nThere's a fascinating debate about who created the Mission burrito.\"}]},{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer text\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":34,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":538,\"output_tokens_details\":{\"reasoning_tokens\":320},\"total_tokens\":572},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            ]);
                let stream = stream_do(&mock, "o3-mini", json!({"prompt": stream_test_prompt(), "providerOptions": {"openai": {"reasoningEffort": "low", "reasoningSummary": "auto", "include": ["reasoning.encrypted_content"]}}, "includeRawChunks": false})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &[
                        json!({
                            "type": "stream-start",
                            "warnings": [],
                        }),
                        json!({
                            "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                            "modelId": "o3-mini-2025-01-31",
                            "timestamp": "2025-03-06T13:50:19.000Z",
                            "type": "response-metadata",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": "encrypted_reasoning_data_abc123",
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "delta": "**Exploring burrito origins**\n\nThe user is",
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "delta": " curious about the debate regarding Taqueria La Cumbre and El Farolito.",
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": "encrypted_reasoning_data_abc123",
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "delta": "**Investigating burrito origins**\n\nThere's a fascinating debate",
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "delta": " about who created the Mission burrito.",
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": "encrypted_reasoning_data_final_def456",
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-start",
                        }),
                        json!({
                            "delta": "answer",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "delta": " text",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-end",
                        }),
                        json!({
                            "finishReason": {
                                "unified": "stop",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "noCache": 34,
                                    "total": 34,
                                },
                                "outputTokens": {
                                    "reasoning": 320,
                                    "text": 218,
                                    "total": 538,
                                },
                                "raw": {
                                    "input_tokens": 34,
                                    "input_tokens_details": {
                                        "cached_tokens": 0,
                                    },
                                    "output_tokens": 538,
                                    "output_tokens_details": {
                                        "reasoning_tokens": 320,
                                    },
                                    "total_tokens": 572,
                                },
                            },
                        }),
                    ],
                );
                stream_assert_match(
                    &mock.seen()[0].json_body(),
                    &json!({"model": "o3-mini", "reasoning": { "effort": "low", "summary": "auto"}, "include": ["reasoning.encrypted_content"], "stream": true}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › reasoning › should stream with encrypted content include reasoning-delta part
            #[tokio::test]
            async fn should_stream_with_encrypted_content_include_reasoning_delta_part() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-reasoning-encrypted-content.1.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5.1-codex-max", json!({"prompt": stream_test_prompt(), "tools": [{"type": "function", "name": "calculator", "inputSchema": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}, "op": {"type": "string"}}, "required": ["a", "b"], "additionalProperties": false}}], "providerOptions": {"openai": {"reasoningEffort": "high", "maxCompletionTokens": 32000, "store": false, "include": ["reasoning.encrypted_content"], "reasoningSummary": "auto", "forceReasoning": true}}})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_with_encrypted_content_include_reasoning_delta.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › reasoning › should handle encrypted content with empty summary
            #[tokio::test]
            async fn should_handle_encrypted_content_with_empty_summary() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"encrypted_content\":\"encrypted_reasoning_data_abc123\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"encrypted_content\":\"encrypted_reasoning_data_final_def456\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\"answer\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\" text\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[{\"id\":\"rs_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"encrypted_content\":\"encrypted_reasoning_data_final_def456\",\"summary\":[]},{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer text\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"low\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":34,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":538,\"output_tokens_details\":{\"reasoning_tokens\":320},\"total_tokens\":572},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            ]);
                let stream = stream_do(&mock, "o3-mini", json!({"prompt": stream_test_prompt(), "providerOptions": {"openai": {"reasoningEffort": "low", "reasoningSummary": null, "include": ["reasoning.encrypted_content"]}}, "includeRawChunks": false})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &[
                        json!({
                            "type": "stream-start",
                            "warnings": [],
                        }),
                        json!({
                            "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                            "modelId": "o3-mini-2025-01-31",
                            "timestamp": "2025-03-06T13:50:19.000Z",
                            "type": "response-metadata",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": "encrypted_reasoning_data_abc123",
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "id": "rs_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": "encrypted_reasoning_data_final_def456",
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-start",
                        }),
                        json!({
                            "delta": "answer",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "delta": " text",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-end",
                        }),
                        json!({
                            "finishReason": {
                                "unified": "stop",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "noCache": 34,
                                    "total": 34,
                                },
                                "outputTokens": {
                                    "reasoning": 320,
                                    "text": 218,
                                    "total": 538,
                                },
                                "raw": {
                                    "input_tokens": 34,
                                    "input_tokens_details": {
                                        "cached_tokens": 0,
                                    },
                                    "output_tokens": 538,
                                    "output_tokens_details": {
                                        "reasoning_tokens": 320,
                                    },
                                    "total_tokens": 572,
                                },
                            },
                        }),
                    ],
                );
                stream_assert_match(
                    &mock.seen()[0].json_body(),
                    &json!({"model": "o3-mini", "reasoning": { "effort": "low"}, "include": ["reasoning.encrypted_content"], "stream": true}),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › reasoning › should handle multiple reasoning blocks
            #[tokio::test]
            async fn should_handle_multiple_reasoning_blocks() {
                let mock = sse_mock(vec![
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0,\"delta\":\"**Initial analysis**\\n\\nFirst reasoning block:\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0,\"delta\":\" analyzing the problem structure.\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1,\"delta\":\"**Deeper consideration**\\n\\nLet me think about\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1,\"delta\":\" the various approaches available.\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"summary_index\":1}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\"Let me think about\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"delta\":\" this step by step.\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":2,\"item\":{\"id\":\"rs_second_7908809g7gcc9291be3e3fee895028c4\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"rs_second_7908809g7gcc9291be3e3fee895028c4\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_second_7908809g7gcc9291be3e3fee895028c4\",\"summary_index\":0,\"delta\":\"Second reasoning block:\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"rs_second_7908809g7gcc9291be3e3fee895028c4\",\"summary_index\":0,\"delta\":\" considering alternative approaches.\"}\n\n".to_string(),
                "data:{\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"rs_second_7908809g7gcc9291be3e3fee895028c4\",\"summary_index\":0}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":2,\"item\":{\"id\":\"rs_second_7908809g7gcc9291be3e3fee895028c4\",\"type\":\"reasoning\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.added\",\"output_index\":3,\"item\":{\"id\":\"msg_final_78d08d03767d92908f25523f5ge51e77\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_final_78d08d03767d92908f25523f5ge51e77\",\"delta\":\"Based on my analysis,\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_text.delta\",\"item_id\":\"msg_final_78d08d03767d92908f25523f5ge51e77\",\"delta\":\" here is the solution.\"}\n\n".to_string(),
                "data:{\"type\":\"response.output_item.done\",\"output_index\":3,\"item\":{\"id\":\"msg_final_78d08d03767d92908f25523f5ge51e77\",\"type\":\"message\"}}\n\n".to_string(),
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67c9a81b6a048190a9ee441c5755a4e8\",\"object\":\"response\",\"created_at\":1741269019,\"status\":\"completed\",\"error\":null,\"incomplete_details\":null,\"input\":[],\"instructions\":null,\"max_output_tokens\":null,\"model\":\"o3-mini-2025-01-31\",\"output\":[{\"id\":\"rs_first_6808709f6fcc8191ad2e2fdd784017b3\",\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"**Initial analysis**\\n\\nFirst reasoning block: analyzing the problem structure.\"},{\"type\":\"summary_text\",\"text\":\"**Deeper consideration**\\n\\nLet me think about the various approaches available.\"}]},{\"id\":\"msg_67c97c02656c81908e080dfdf4a03cd1\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Let me think about this step by step.\",\"annotations\":[]}]},{\"id\":\"rs_second_7908809g7gcc9291be3e3fee895028c4\",\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"Second reasoning block: considering alternative approaches.\"}]},{\"id\":\"msg_final_78d08d03767d92908f25523f5ge51e77\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Based on my analysis, here is the solution.\",\"annotations\":[]}]}],\"parallel_tool_calls\":true,\"previous_response_id\":null,\"reasoning\":{\"effort\":\"medium\",\"summary\":\"auto\"},\"store\":true,\"temperature\":null,\"text\":{\"format\":{\"type\":\"text\"}},\"tool_choice\":\"auto\",\"tools\":[],\"top_p\":null,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":45,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":628,\"output_tokens_details\":{\"reasoning_tokens\":420},\"total_tokens\":673},\"user\":null,\"metadata\":{}}}\n\n".to_string(),
            ]);
                let stream = stream_do(&mock, "o3-mini", json!({"prompt": stream_test_prompt(), "providerOptions": {"openai": {"reasoningEffort": "medium", "reasoningSummary": "auto"}}, "includeRawChunks": false})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &[
                        json!({
                            "type": "stream-start",
                            "warnings": [],
                        }),
                        json!({
                            "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                            "modelId": "o3-mini-2025-01-31",
                            "timestamp": "2025-03-06T13:50:19.000Z",
                            "type": "response-metadata",
                        }),
                        json!({
                            "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "delta": "**Initial analysis**\n\nFirst reasoning block:",
                            "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "delta": " analyzing the problem structure.",
                            "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "delta": "**Deeper consideration**\n\nLet me think about",
                            "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "delta": " the various approaches available.",
                            "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "id": "rs_first_6808709f6fcc8191ad2e2fdd784017b3:1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_first_6808709f6fcc8191ad2e2fdd784017b3",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-start",
                        }),
                        json!({
                            "delta": "Let me think about",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "delta": " this step by step.",
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "type": "text-delta",
                        }),
                        json!({
                            "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_67c97c02656c81908e080dfdf4a03cd1",
                                },
                            },
                            "type": "text-end",
                        }),
                        json!({
                            "id": "rs_second_7908809g7gcc9291be3e3fee895028c4:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_second_7908809g7gcc9291be3e3fee895028c4",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-start",
                        }),
                        json!({
                            "delta": "Second reasoning block:",
                            "id": "rs_second_7908809g7gcc9291be3e3fee895028c4:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_second_7908809g7gcc9291be3e3fee895028c4",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "delta": " considering alternative approaches.",
                            "id": "rs_second_7908809g7gcc9291be3e3fee895028c4:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_second_7908809g7gcc9291be3e3fee895028c4",
                                },
                            },
                            "type": "reasoning-delta",
                        }),
                        json!({
                            "id": "rs_second_7908809g7gcc9291be3e3fee895028c4:0",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "rs_second_7908809g7gcc9291be3e3fee895028c4",
                                    "reasoningEncryptedContent": null,
                                },
                            },
                            "type": "reasoning-end",
                        }),
                        json!({
                            "id": "msg_final_78d08d03767d92908f25523f5ge51e77",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_final_78d08d03767d92908f25523f5ge51e77",
                                },
                            },
                            "type": "text-start",
                        }),
                        json!({
                            "delta": "Based on my analysis,",
                            "id": "msg_final_78d08d03767d92908f25523f5ge51e77",
                            "type": "text-delta",
                        }),
                        json!({
                            "delta": " here is the solution.",
                            "id": "msg_final_78d08d03767d92908f25523f5ge51e77",
                            "type": "text-delta",
                        }),
                        json!({
                            "id": "msg_final_78d08d03767d92908f25523f5ge51e77",
                            "providerMetadata": {
                                "openai": {
                                    "itemId": "msg_final_78d08d03767d92908f25523f5ge51e77",
                                },
                            },
                            "type": "text-end",
                        }),
                        json!({
                            "finishReason": {
                                "unified": "stop",
                            },
                            "providerMetadata": {
                                "openai": {
                                    "responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                                },
                            },
                            "type": "finish",
                            "usage": {
                                "inputTokens": {
                                    "cacheRead": 0,
                                    "noCache": 45,
                                    "total": 45,
                                },
                                "outputTokens": {
                                    "reasoning": 420,
                                    "text": 208,
                                    "total": 628,
                                },
                                "raw": {
                                    "input_tokens": 45,
                                    "input_tokens_details": {
                                        "cached_tokens": 0,
                                    },
                                    "output_tokens": 628,
                                    "output_tokens_details": {
                                        "reasoning_tokens": 420,
                                    },
                                    "total_tokens": 673,
                                },
                            },
                        }),
                    ],
                );
                stream_assert_match(
                    &mock.seen()[0].json_body(),
                    &json!({"model": "o3-mini", "reasoning": { "effort": "medium", "summary": "auto"}, "stream": true}),
                );
            }
        }

        mod apply_patch_tool_streaming {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › apply_patch tool streaming › should handle apply_patch tool calls in streaming mode
            #[tokio::test]
            async fn should_handle_apply_patch_tool_calls_in_streaming_mode() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-apply-patch-tool.1.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5.1-2025-11-13", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.apply_patch", "name": "apply_patch", "args": {}}]})).await.unwrap();
                let result = stream_collect(stream).await;
                // `expect.objectContaining({ type: 'finish', finishReason: { unified:
                // 'tool-calls', raw: undefined } })`.
                assert!(result.iter().any(|part| {
                    part["type"] == "finish"
                        && part["finishReason"]["unified"] == "tool-calls"
                        && part["finishReason"].as_object().is_some_and(|reason| {
                            reason.get("raw").is_none_or(Value::is_null) && reason.len() <= 2
                        })
                }));
                stream_assert_parts(
                    &result,
                    &stream_snapshot(include_str!(
                        "snapshots/should_handle_apply_patch_tool_calls_in_streaming_mode.json"
                    )),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doStream › apply_patch tool streaming › should stream apply_patch delete_file calls
            #[tokio::test]
            async fn should_stream_apply_patch_delete_file_calls() {
                let mock = sse_fixture_mock(include_str!(
                    "fixtures/openai-apply-patch-tool-delete.1.chunks.txt"
                ));
                let stream = stream_do(&mock, "gpt-5.1-2025-11-13", json!({"prompt": stream_test_prompt(), "tools": [{"type": "provider", "id": "openai.apply_patch", "name": "apply_patch", "args": {}}]})).await.unwrap();
                stream_assert_parts(
                    &stream_collect(stream).await,
                    &stream_snapshot(include_str!(
                        "snapshots/should_stream_apply_patch_delete_file_calls.json"
                    )),
                );
            }
        }
    }

    mod mixed_citation_types {
        use super::*;

        fn options() -> Value {
            json!({ "prompt": test_prompt(), "includeRawChunks": false })
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › mixed citation types › should handle both url_citation and file_citation annotations
        #[tokio::test]
        async fn should_handle_both_url_citation_and_file_citation_annotations() {
            let chunks: &[&str] = &[
                r#"{"type":"response.content_part.added","item_id":"msg_123","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#,
                r#"{"type":"response.output_text.annotation.added","item_id":"msg_123","output_index":0,"content_index":0,"annotation_index":0,"annotation":{"type":"url_citation","url":"https://example.com","title":"Example URL","start_index":123,"end_index":234}}"#,
                r#"{"type":"response.output_text.annotation.added","item_id":"msg_123","output_index":0,"content_index":0,"annotation_index":1,"annotation":{"type":"file_citation","index":123,"file_id":"file-abc123","filename":"resource1.json"}}"#,
                r#"{"type":"response.content_part.done","item_id":"msg_123","output_index":0,"content_index":0,"part":{"type":"output_text","text":"Based on web search and file content.","annotations":[{"type":"url_citation","start_index":0,"end_index":10,"url":"https://example.com","title":"Example URL"},{"type":"file_citation","index":123,"file_id":"file-abc123","filename":"resource1.json"}]}}"#,
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_123","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Based on web search and file content.","annotations":[{"type":"url_citation","start_index":0,"end_index":10,"url":"https://example.com","title":"Example URL"},{"type":"file_citation","index":123,"file_id":"file-abc123","filename":"resource1.json"}]}]}}"#,
                r#"{"type":"response.completed","response":{"id":"resp_123","object":"response","created_at":1234567890,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-4o","output":[{"id":"msg_123","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Based on web search and file content.","annotations":[{"type":"url_citation","start_index":0,"end_index":10,"url":"https://example.com","title":"Example URL"},{"type":"file_citation","index":123,"file_id":"file-abc123","filename":"resource1.json"}]}]}],"parallel_tool_calls":true,"previous_response_id":null,"reasoning":{"effort":null,"summary":null},"store":true,"temperature":0,"text":{"format":{"type":"text"}},"tool_choice":"auto","tools":[],"top_p":1,"truncation":"disabled","usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":0},"output_tokens":50,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":150},"user":null,"metadata":{}}}"#,
            ];
            let r = collect_parts("gpt-4o", json_event_stream(chunks), options()).await;
            assert_json_eq(
                &r.parts,
                &json!([
                  {
                    "type": "stream-start",
                    "warnings": []
                  },
                  {
                    "id": "id-0",
                    "sourceType": "url",
                    "title": "Example URL",
                    "type": "source",
                    "url": "https://example.com"
                  },
                  {
                    "filename": "resource1.json",
                    "id": "id-1",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "file-abc123",
                        "index": 123,
                        "type": "file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "resource1.json",
                    "type": "source"
                  },
                  {
                    "id": "msg_123",
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "end_index": 234,
                            "start_index": 123,
                            "title": "Example URL",
                            "type": "url_citation",
                            "url": "https://example.com"
                          },
                          {
                            "file_id": "file-abc123",
                            "filename": "resource1.json",
                            "index": 123,
                            "type": "file_citation"
                          }
                        ],
                        "itemId": "msg_123"
                      }
                    },
                    "type": "text-end"
                  },
                  {
                    "finishReason": {
                      "unified": "stop"
                    },
                    "providerMetadata": {
                      "openai": {
                        "responseId": null
                      }
                    },
                    "type": "finish",
                    "usage": {
                      "inputTokens": {
                        "cacheRead": 0,
                        "noCache": 100,
                        "total": 100
                      },
                      "outputTokens": {
                        "reasoning": 0,
                        "text": 50,
                        "total": 50
                      },
                      "raw": {
                        "input_tokens": 100,
                        "input_tokens_details": {
                          "cached_tokens": 0
                        },
                        "output_tokens": 50,
                        "output_tokens_details": {
                          "reasoning_tokens": 0
                        },
                        "total_tokens": 150
                      }
                    }
                  }
                ]),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › mixed citation types › should handle file_citation annotations without optional fields in streaming
        #[tokio::test]
        async fn should_handle_file_citation_annotations_without_optional_fields_in_streaming() {
            let chunks: &[&str] = &[
                r#"{"type":"response.content_part.added","item_id":"msg_456","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#,
                r#"{"type":"response.output_text.annotation.added","item_id":"msg_456","output_index":0,"content_index":0,"annotation_index":0,"annotation":{"type":"file_citation","file_id":"file-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":145}}"#,
                r#"{"type":"response.output_text.annotation.added","item_id":"msg_456","output_index":0,"content_index":0,"annotation_index":1,"annotation":{"type":"file_citation","file_id":"file-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":192}}"#,
                r#"{"type":"response.content_part.done","item_id":"msg_456","output_index":0,"content_index":0,"part":{"type":"output_text","text":"Answer for the specified years....","annotations":[{"type":"file_citation","file_id":"file-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":145},{"type":"file_citation","file_id":"file-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":192}]}}"#,
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_456","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Answer for the specified years....","annotations":[{"type":"file_citation","file_id":"file-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":145},{"type":"file_citation","file_id":"file-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":192}]}]}}"#,
                r#"{"type":"response.completed","response":{"id":"resp_456","object":"response","created_at":1234567890,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-5","output":[{"id":"msg_456","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Answer for the specified years....","annotations":[{"type":"file_citation","file_id":"file-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":145},{"type":"file_citation","file_id":"file-YRcoCqn3Fo2K4JgraG","filename":"resource1.json","index":192}]}]}],"parallel_tool_calls":true,"previous_response_id":null,"reasoning":{"effort":null,"summary":null},"store":true,"temperature":0,"text":{"format":{"type":"text"}},"tool_choice":"auto","tools":[],"top_p":1,"truncation":"disabled","usage":{"input_tokens":50,"input_tokens_details":{"cached_tokens":0},"output_tokens":25,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":75},"user":null,"metadata":{}}}"#,
            ];
            let r = collect_parts("gpt-5", json_event_stream(chunks), options()).await;
            assert_json_eq(
                &r.parts,
                &json!([
                  {
                    "type": "stream-start",
                    "warnings": []
                  },
                  {
                    "filename": "resource1.json",
                    "id": "id-0",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "file-YRcoCqn3Fo2K4JgraG",
                        "index": 145,
                        "type": "file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "resource1.json",
                    "type": "source"
                  },
                  {
                    "filename": "resource1.json",
                    "id": "id-1",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "file-YRcoCqn3Fo2K4JgraG",
                        "index": 192,
                        "type": "file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "resource1.json",
                    "type": "source"
                  },
                  {
                    "id": "msg_456",
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "file_id": "file-YRcoCqn3Fo2K4JgraG",
                            "filename": "resource1.json",
                            "index": 145,
                            "type": "file_citation"
                          },
                          {
                            "file_id": "file-YRcoCqn3Fo2K4JgraG",
                            "filename": "resource1.json",
                            "index": 192,
                            "type": "file_citation"
                          }
                        ],
                        "itemId": "msg_456"
                      }
                    },
                    "type": "text-end"
                  },
                  {
                    "finishReason": {
                      "unified": "stop"
                    },
                    "providerMetadata": {
                      "openai": {
                        "responseId": null
                      }
                    },
                    "type": "finish",
                    "usage": {
                      "inputTokens": {
                        "cacheRead": 0,
                        "noCache": 50,
                        "total": 50
                      },
                      "outputTokens": {
                        "reasoning": 0,
                        "text": 25,
                        "total": 25
                      },
                      "raw": {
                        "input_tokens": 50,
                        "input_tokens_details": {
                          "cached_tokens": 0
                        },
                        "output_tokens": 25,
                        "output_tokens_details": {
                          "reasoning_tokens": 0
                        },
                        "total_tokens": 75
                      }
                    }
                  }
                ]),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › mixed citation types › should handle container_file_citation annotations in streaming
        #[tokio::test]
        async fn should_handle_container_file_citation_annotations_in_streaming() {
            let chunks: &[&str] = &[
                r#"{"type":"response.content_part.added","item_id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#,
                r#"{"type":"response.output_text.annotation.added","item_id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","output_index":0,"content_index":0,"annotation_index":0,"annotation":{"type":"container_file_citation","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","end_index":465,"file_id":"cfile_68c2e7084ab48191a67824aa1f4c90f1","filename":"roll2dice_sums_10000.csv","start_index":423}}"#,
                r#"{"type":"response.content_part.done","item_id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[{"type":"container_file_citation","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","end_index":465,"file_id":"cfile_68c2e7084ab48191a67824aa1f4c90f1","filename":"roll2dice_sums_10000.csv","start_index":423}],"logprobs":[],"text":"Heres a simulation of rolling two fair six-sided dice 10,000 times. Each trial sums the two dice.\n\nResults\n- Total sum of all 10,000 trials: 69,868\n- Average sum per trial: 6.9868\n- Minimum sum observed: 2\n- Maximum sum observed: 12\n- Sample of the first 20 trial sums: 6, 7, 2, 5, 5, 11, 4, 8, 10, 7, 5, 8, 8, 7, 10, 8, 9, 5, 4, 7\n\nFull data\n- You can download all 10,000 sums as a CSV file here: [Download the sums CSV](sandbox:/mnt/data/roll2dice_sums_10000.csv)\n\nIf you'd like, I can also provide a frequency distribution, histogram, or export the data in another format (JSON, Excel, etc.)."}}"#,
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","type":"message","status":"completed","content":[{"type":"output_text","annotations":[{"type":"container_file_citation","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","end_index":465,"file_id":"cfile_68c2e7084ab48191a67824aa1f4c90f1","filename":"roll2dice_sums_10000.csv","start_index":423}],"logprobs":[],"text":"Heres a simulation of rolling two fair six-sided dice 10,000 times. Each trial sums the two dice.\n\nResults\n- Total sum of all 10,000 trials: 69,868\n- Average sum per trial: 6.9868\n- Minimum sum observed: 2\n- Maximum sum observed: 12\n- Sample of the first 20 trial sums: 6, 7, 2, 5, 5, 11, 4, 8, 10, 7, 5, 8, 8, 7, 10, 8, 9, 5, 4, 7\n\nFull data\n- You can download all 10,000 sums as a CSV file here: [Download the sums CSV](sandbox:/mnt/data/roll2dice_sums_10000.csv)\n\nIf you'd like, I can also provide a frequency distribution, histogram, or export the data in another format (JSON, Excel, etc.)."}],"role":"assistant"}}"#,
                r#"{"type":"response.completed","response":{"id":"resp_68c2e6efa238819383d5f52a2c2a3baa02d3a5742c7ddae9","object":"response","created_at":1757603567,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-5-nano-2025-08-07","output":[{"id":"rs_68c2e6f40ba48193a1c27abf31130e7e02d3a5742c7ddae9","type":"reasoning","summary":[]},{"id":"ci_68c2e6f7b72c8193ba1f552552c8dc9202d3a5742c7ddae9","type":"code_interpreter_call","status":"completed","code":"import random, math\nN=10000\nsums=[]\ns=0\nfor _ in range(N):\n    a=random.randint(1,6)\n    b=random.randint(1,6)\n    sm=a+b\n    sums.append(sm)\n    s+=sm\nmin(sums), max(sums), sum(sums), sum(sums)/N\n","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","outputs":[{"type":"logs","logs":"(2, 12, 69868, 6.9868)"}]},{"id":"rs_68c2e6fcb52881938f21c45741216ac002d3a5742c7ddae9","type":"reasoning","summary":[]},{"id":"ci_68c2e6fd57948193aa93df6bdb00a86d02d3a5742c7ddae9","type":"code_interpreter_call","status":"completed","code":"import csv, pathlib\npath = pathlib.Path('/mnt/data/roll2dice_sums_10000.csv')\nwith open(path, 'w', newline='') as f:\n    writer = csv.writer(f)\n    writer.writerow(['sum'])\n    for val in sums:\n        writer.writerow([val])\npath, path.exists(), len(sums)\n","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","outputs":[{"type":"logs","logs":"(PosixPath('/mnt/data/roll2dice_sums_10000.csv'), True, 10000)"}]},{"id":"rs_68c2e6fff1808193a78d43410a1feb4802d3a5742c7ddae9","type":"reasoning","summary":[]},{"id":"ci_68c2e701a23081939c93b6fb5bb952d302d3a5742c7ddae9","type":"code_interpreter_call","status":"completed","code":"sums[:20]\n","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","outputs":[{"type":"logs","logs":"[6, 7, 2, 5, 5, 11, 4, 8, 10, 7, 5, 8, 8, 7, 10, 8, 9, 5, 4, 7]"}]},{"id":"rs_68c2e703d114819383c5da260649c7ce02d3a5742c7ddae9","type":"reasoning","summary":[]},{"id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","type":"message","status":"completed","content":[{"type":"output_text","annotations":[{"type":"container_file_citation","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","end_index":465,"file_id":"cfile_68c2e7084ab48191a67824aa1f4c90f1","filename":"roll2dice_sums_10000.csv","start_index":423}],"logprobs":[],"text":"Here's a simulation of rolling two fair six-sided dice 10,000 times. Each trial sums the two dice.\n\nResults\n- Total sum of all 10,000 trials: 69,868\n- Average sum per trial: 6.9868\n- Minimum sum observed: 2\n- Maximum sum observed: 12\n- Sample of the first 20 trial sums: 6, 7, 2, 5, 5, 11, 4, 8, 10, 7, 5, 8, 8, 7, 10, 8, 9, 5, 4, 7\n\nFull data\n- You can download all 10,000 sums as a CSV file here: [Download the sums CSV](sandbox:/mnt/data/roll2dice_sums_10000.csv)\n\nIf you'd like, I can also provide a frequency distribution, histogram, or export the data in another format (JSON, Excel, etc.)."}],"role":"assistant"}],"parallel_tool_calls":true,"previous_response_id":null,"prompt_cache_key":null,"reasoning":{"effort":"medium","summary":null},"safety_identifier":null,"store":true,"temperature":1,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tools":[{"type":"code_interpreter","container":{"type":"auto"}}],"top_logprobs":0,"top_p":1,"truncation":"disabled","usage":{"input_tokens":6047,"input_tokens_details":{"cached_tokens":2944},"output_tokens":1623,"output_tokens_details":{"reasoning_tokens":1408},"total_tokens":7670},"user":null,"metadata":{}}}"#,
            ];
            let r = collect_parts("gpt-5", json_event_stream(chunks), options()).await;
            assert_json_eq(
                &r.parts,
                &json!([
                  {
                    "type": "stream-start",
                    "warnings": []
                  },
                  {
                    "filename": "roll2dice_sums_10000.csv",
                    "id": "id-0",
                    "mediaType": "text/plain",
                    "providerMetadata": {
                      "openai": {
                        "containerId": "cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9",
                        "fileId": "cfile_68c2e7084ab48191a67824aa1f4c90f1",
                        "type": "container_file_citation"
                      }
                    },
                    "sourceType": "document",
                    "title": "roll2dice_sums_10000.csv",
                    "type": "source"
                  },
                  {
                    "id": "msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9",
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "container_id": "cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9",
                            "end_index": 465,
                            "file_id": "cfile_68c2e7084ab48191a67824aa1f4c90f1",
                            "filename": "roll2dice_sums_10000.csv",
                            "start_index": 423,
                            "type": "container_file_citation"
                          }
                        ],
                        "itemId": "msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9"
                      }
                    },
                    "type": "text-end"
                  },
                  {
                    "finishReason": {
                      "unified": "stop"
                    },
                    "providerMetadata": {
                      "openai": {
                        "responseId": null
                      }
                    },
                    "type": "finish",
                    "usage": {
                      "inputTokens": {
                        "cacheRead": 2944,
                        "noCache": 3103,
                        "total": 6047
                      },
                      "outputTokens": {
                        "reasoning": 1408,
                        "text": 215,
                        "total": 1623
                      },
                      "raw": {
                        "input_tokens": 6047,
                        "input_tokens_details": {
                          "cached_tokens": 2944
                        },
                        "output_tokens": 1623,
                        "output_tokens_details": {
                          "reasoning_tokens": 1408
                        },
                        "total_tokens": 7670
                      }
                    }
                  }
                ]),
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › mixed citation types › should handle file_path annotations in streaming
        #[tokio::test]
        async fn should_handle_file_path_annotations_in_streaming() {
            let chunks: &[&str] = &[
                r#"{"type":"response.content_part.added","item_id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#,
                r#"{"type":"response.output_text.annotation.added","item_id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","output_index":0,"content_index":0,"annotation_index":0,"annotation":{"type":"file_path","file_id":"cfile_68c2e7084ab48191a67824aa1f4c90f1","index":123}}"#,
                r#"{"type":"response.content_part.done","item_id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[{"type":"file_path","file_id":"cfile_68c2e7084ab48191a67824aa1f4c90f1","index":123}],"logprobs":[],"text":"Heres a simulation of rolling two fair six-sided dice 10,000 times. Each trial sums the two dice.\n\nResults\n- Total sum of all 10,000 trials: 69,868\n- Average sum per trial: 6.9868\n- Minimum sum observed: 2\n- Maximum sum observed: 12\n- Sample of the first 20 trial sums: 6, 7, 2, 5, 5, 11, 4, 8, 10, 7, 5, 8, 8, 7, 10, 8, 9, 5, 4, 7\n\nFull data\n- You can download all 10,000 sums as a CSV file here: [Download the sums CSV](sandbox:/mnt/data/roll2dice_sums_10000.csv)\n\nIf you'd like, I can also provide a frequency distribution, histogram, or export the data in another format (JSON, Excel, etc.)."}}"#,
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","type":"message","status":"completed","content":[{"type":"output_text","annotations":[{"type":"file_path","file_id":"cfile_68c2e7084ab48191a67824aa1f4c90f1"}],"logprobs":[],"text":"Heres a simulation of rolling two fair six-sided dice 10,000 times. Each trial sums the two dice.\n\nResults\n- Total sum of all 10,000 trials: 69,868\n- Average sum per trial: 6.9868\n- Minimum sum observed: 2\n- Maximum sum observed: 12\n- Sample of the first 20 trial sums: 6, 7, 2, 5, 5, 11, 4, 8, 10, 7, 5, 8, 8, 7, 10, 8, 9, 5, 4, 7\n\nFull data\n- You can download all 10,000 sums as a CSV file here: [Download the sums CSV](sandbox:/mnt/data/roll2dice_sums_10000.csv)\n\nIf you'd like, I can also provide a frequency distribution, histogram, or export the data in another format (JSON, Excel, etc.)."}],"role":"assistant"}}"#,
                r#"{"type":"response.completed","response":{"id":"resp_68c2e6efa238819383d5f52a2c2a3baa02d3a5742c7ddae9","object":"response","created_at":1757603567,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-5-nano-2025-08-07","output":[{"id":"rs_68c2e6f40ba48193a1c27abf31130e7e02d3a5742c7ddae9","type":"reasoning","summary":[]},{"id":"ci_68c2e6f7b72c8193ba1f552552c8dc9202d3a5742c7ddae9","type":"code_interpreter_call","status":"completed","code":"import random, math\nN=10000\nsums=[]\ns=0\nfor _ in range(N):\n    a=random.randint(1,6)\n    b=random.randint(1,6)\n    sm=a+b\n    sums.append(sm)\n    s+=sm\nmin(sums), max(sums), sum(sums), sum(sums)/N\n","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","outputs":[{"type":"logs","logs":"(2, 12, 69868, 6.9868)"}]},{"id":"rs_68c2e6fcb52881938f21c45741216ac002d3a5742c7ddae9","type":"reasoning","summary":[]},{"id":"ci_68c2e6fd57948193aa93df6bdb00a86d02d3a5742c7ddae9","type":"code_interpreter_call","status":"completed","code":"import csv, pathlib\npath = pathlib.Path('/mnt/data/roll2dice_sums_10000.csv')\nwith open(path, 'w', newline='') as f:\n    writer = csv.writer(f)\n    writer.writerow(['sum'])\n    for val in sums:\n        writer.writerow([val])\npath, path.exists(), len(sums)\n","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","outputs":[{"type":"logs","logs":"(PosixPath('/mnt/data/roll2dice_sums_10000.csv'), True, 10000)"}]},{"id":"rs_68c2e6fff1808193a78d43410a1feb4802d3a5742c7ddae9","type":"reasoning","summary":[]},{"id":"ci_68c2e701a23081939c93b6fb5bb952d302d3a5742c7ddae9","type":"code_interpreter_call","status":"completed","code":"sums[:20]\n","container_id":"cntr_68c2e6f380d881908a57a82d394434ff02f484f5344062e9","outputs":[{"type":"logs","logs":"[6, 7, 2, 5, 5, 11, 4, 8, 10, 7, 5, 8, 8, 7, 10, 8, 9, 5, 4, 7]"}]},{"id":"rs_68c2e703d114819383c5da260649c7ce02d3a5742c7ddae9","type":"reasoning","summary":[]},{"id":"msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9","type":"message","status":"completed","content":[{"type":"output_text","annotations":[{"type":"file_path","file_id":"cfile_68c2e7084ab48191a67824aa1f4c90f1","index":123}],"logprobs":[],"text":"Here's a simulation of rolling two fair six-sided dice 10,000 times. Each trial sums the two dice.\n\nResults\n- Total sum of all 10,000 trials: 69,868\n- Average sum per trial: 6.9868\n- Minimum sum observed: 2\n- Maximum sum observed: 12\n- Sample of the first 20 trial sums: 6, 7, 2, 5, 5, 11, 4, 8, 10, 7, 5, 8, 8, 7, 10, 8, 9, 5, 4, 7\n\nFull data\n- You can download all 10,000 sums as a CSV file here: [Download the sums CSV](sandbox:/mnt/data/roll2dice_sums_10000.csv)\n\nIf you'd like, I can also provide a frequency distribution, histogram, or export the data in another format (JSON, Excel, etc.)."}],"role":"assistant"}],"parallel_tool_calls":true,"previous_response_id":null,"prompt_cache_key":null,"reasoning":{"effort":"medium","summary":null},"safety_identifier":null,"store":true,"temperature":1,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tools":[{"type":"code_interpreter","container":{"type":"auto"}}],"top_logprobs":0,"top_p":1,"truncation":"disabled","usage":{"input_tokens":6047,"input_tokens_details":{"cached_tokens":2944},"output_tokens":1623,"output_tokens_details":{"reasoning_tokens":1408},"total_tokens":7670},"user":null,"metadata":{}}}"#,
            ];
            let r = collect_parts("gpt-4o", json_event_stream(chunks), options()).await;
            assert_json_eq(
                &r.parts,
                &json!([
                  {
                    "type": "stream-start",
                    "warnings": []
                  },
                  {
                    "filename": "cfile_68c2e7084ab48191a67824aa1f4c90f1",
                    "id": "id-0",
                    "mediaType": "application/octet-stream",
                    "providerMetadata": {
                      "openai": {
                        "fileId": "cfile_68c2e7084ab48191a67824aa1f4c90f1",
                        "index": 123,
                        "type": "file_path"
                      }
                    },
                    "sourceType": "document",
                    "title": "cfile_68c2e7084ab48191a67824aa1f4c90f1",
                    "type": "source"
                  },
                  {
                    "id": "msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9",
                    "providerMetadata": {
                      "openai": {
                        "annotations": [
                          {
                            "file_id": "cfile_68c2e7084ab48191a67824aa1f4c90f1",
                            "index": 123,
                            "type": "file_path"
                          }
                        ],
                        "itemId": "msg_68c2e7054ae481938354ab3e4e77abad02d3a5742c7ddae9"
                      }
                    },
                    "type": "text-end"
                  },
                  {
                    "finishReason": {
                      "unified": "stop"
                    },
                    "providerMetadata": {
                      "openai": {
                        "responseId": null
                      }
                    },
                    "type": "finish",
                    "usage": {
                      "inputTokens": {
                        "cacheRead": 2944,
                        "noCache": 3103,
                        "total": 6047
                      },
                      "outputTokens": {
                        "reasoning": 1408,
                        "text": 215,
                        "total": 1623
                      },
                      "raw": {
                        "input_tokens": 6047,
                        "input_tokens_details": {
                          "cached_tokens": 2944
                        },
                        "output_tokens": 1623,
                        "output_tokens_details": {
                          "reasoning_tokens": 1408
                        },
                        "total_tokens": 7670
                      }
                    }
                  }
                ]),
            );
        }

        mod compaction {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › mixed citation types › compaction › should append an explicit compaction trigger to streaming input
            #[tokio::test]
            async fn should_append_an_explicit_compaction_trigger_to_streaming_input() {
                let r = collect_parts(
                    "gpt-5.2",
                    json_chunks_fixture!("openai-compaction.1"),
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "compactionTrigger": true
                        }
                      }
                    }),
                )
                .await;
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "stream": true,
                      "input": [
                        {
                          "role": "user",
                          "content": [
                            {
                              "type": "input_text",
                              "text": "Hello"
                            }
                          ]
                        },
                        {
                          "type": "compaction_trigger"
                        }
                      ]
                    }),
                );
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › mixed citation types › compaction › should stream compaction output item from real fixture
            #[tokio::test]
            async fn should_stream_compaction_output_item_from_real_fixture() {
                let r = collect_parts(
                    "gpt-5.2",
                    json_chunks_fixture!("openai-compaction.1"),
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "store": false,
                          "contextManagement": [
                            {
                              "type": "compaction",
                              "compactThreshold": 50000
                            }
                          ]
                        }
                      }
                    }),
                )
                .await;
                let expected: Value =
                    serde_json::from_str(include_str!("snapshots/citation_compaction_stream.json"))
                        .unwrap();
                assert_json_eq(&r.parts, &expected);
            }

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › mixed citation types › compaction › should send context_management in streaming request body
            #[tokio::test]
            async fn should_send_context_management_in_streaming_request_body() {
                let r = collect_parts(
                    "gpt-5.2",
                    json_chunks_fixture!("openai-compaction.1"),
                    json!({
                      "prompt": test_prompt(),
                      "providerOptions": {
                        "openai": {
                          "store": false,
                          "contextManagement": [
                            {
                              "type": "compaction",
                              "compactThreshold": 50000
                            }
                          ]
                        }
                      }
                    }),
                )
                .await;
                assert_json_match_object(
                    &r.body,
                    &json!({
                      "model": "gpt-5.2",
                      "store": false,
                      "stream": true,
                      "context_management": [
                        {
                          "type": "compaction",
                          "compact_threshold": 50000
                        }
                      ]
                    }),
                );
            }
        }

        mod phase {
            use super::*;

            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › mixed citation types › phase › should include phase in provider metadata for streamed message items
            #[tokio::test]
            async fn should_include_phase_in_provider_metadata_for_streamed_message_items() {
                let r = collect_parts(
                    "gpt-5.3-codex",
                    json_chunks_fixture!("openai-phase.1"),
                    json!({
                      "prompt": test_prompt(),
                      "includeRawChunks": false
                    }),
                )
                .await;
                let parts = r.parts.as_array().unwrap();
                let of_type = |kind: &str| -> Vec<&Value> {
                    parts.iter().filter(|part| part["type"] == kind).collect()
                };

                let text_start_parts = of_type("text-start");
                assert_eq!(text_start_parts.len(), 2);
                assert_json_match_object(
                    &text_start_parts[0]["providerMetadata"]["openai"],
                    &json!({
                      "itemId": "msg_0a63f40a2632b74300699f8819a5e08196ac270722d369af5a",
                      "phase": "commentary"
                    }),
                );
                assert_json_match_object(
                    &text_start_parts[1]["providerMetadata"]["openai"],
                    &json!({
                      "itemId": "msg_0a63f40a2632b74300699f881bfbc88196aec38f30c3dd24b0",
                      "phase": "final_answer"
                    }),
                );

                let text_end_parts = of_type("text-end");
                assert_eq!(text_end_parts.len(), 2);
                assert_json_match_object(
                    &text_end_parts[0]["providerMetadata"]["openai"],
                    &json!({
                      "itemId": "msg_0a63f40a2632b74300699f8819a5e08196ac270722d369af5a",
                      "phase": "commentary"
                    }),
                );
                assert_json_match_object(
                    &text_end_parts[1]["providerMetadata"]["openai"],
                    &json!({
                      "itemId": "msg_0a63f40a2632b74300699f881bfbc88196aec38f30c3dd24b0",
                      "phase": "final_answer"
                    }),
                );
            }
        }
    }

    mod programmatic_tool_calling {
        use super::*;

        fn tools() -> Value {
            json!([
              {
                "type": "provider",
                "id": "openai.programmatic_tool_calling",
                "name": "program",
                "args": {}
              },
              {
                "type": "function",
                "name": "getInventory",
                "inputSchema": {
                  "type": "object",
                  "properties": {
                    "sku": {
                      "type": "string"
                    }
                  },
                  "required": [
                    "sku"
                  ]
                }
              },
              {
                "type": "function",
                "name": "getDemand",
                "inputSchema": {
                  "type": "object",
                  "properties": {
                    "sku": {
                      "type": "string"
                    }
                  },
                  "required": [
                    "sku"
                  ]
                }
              }
            ])
        }

        /// `expect.arrayContaining(expected)`: every expected entry matches
        /// some actual entry (`Ok` entries by equality, `Err` entries by
        /// `expect.objectContaining`).
        #[track_caller]
        fn assert_array_containing(actual: &[Value], expected: &[Result<Value, Value>]) {
            for entry in expected {
                let found = actual.iter().any(|part| match entry {
                    Ok(value) => part == value,
                    Err(subset) => json_object_containing(part, subset),
                });
                assert!(
                    found,
                    "no entry matches {entry:#?}\nactual: {:#}",
                    json!(actual)
                );
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › programmatic tool calling › should map programmatic tool calling across generate steps from real fixtures
        #[tokio::test]
        async fn should_map_programmatic_tool_calling_across_generate_steps_from_real_fixtures() {
            let mut content = Vec::new();
            let fixtures = [
                json_fixture!("programmatic-tool-calling.1"),
                json_fixture!("programmatic-tool-calling.2"),
                json_fixture!("programmatic-tool-calling.3"),
            ];
            for fixture in fixtures {
                let r = generate(
                    "gpt-5.6",
                    fixture,
                    json!({ "prompt": test_prompt(), "tools": tools() }),
                )
                .await;
                content.extend(r.content().as_array().unwrap().iter().cloned());
            }

            assert_array_containing(
                &content,
                &[
                    Err(json!({
                        "type": "tool-call",
                        "toolCallId": "call_O2IvSLQcJ0bwIvJZ2ovGV69M",
                        "toolName": "program",
                        "providerExecuted": true,
                        "providerMetadata": {
                            "openai": {
                                "itemId": "cm_0742d30c1d273351016a6145f1d2d0819faa3ecfc950fceec4",
                            },
                        },
                    })),
                    Ok(json!({
                        "type": "tool-call",
                        "toolCallId": "call_rj6LW6NEyodD5YVKeoexoLNz",
                        "toolName": "getInventory",
                        "input": "{\"sku\":\"sku_123\"}",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "fc_0742d30c1d273351016a6145f1dac0819fb0053980ae918c16",
                                "caller": {
                                    "type": "program",
                                    "callerId": "call_O2IvSLQcJ0bwIvJZ2ovGV69M",
                                },
                            },
                        },
                    })),
                    Ok(json!({
                        "type": "tool-call",
                        "toolCallId": "call_IYnPSr6i8TyBPs1H9U539pUP",
                        "toolName": "getDemand",
                        "input": "{\"sku\":\"sku_123\"}",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "fc_0742d30c1d273351016a6145f446e8819f9a2bd24df7057cd8",
                                "caller": {
                                    "type": "program",
                                    "callerId": "call_O2IvSLQcJ0bwIvJZ2ovGV69M",
                                },
                            },
                        },
                    })),
                    Ok(json!({
                        "type": "tool-result",
                        "toolCallId": "call_O2IvSLQcJ0bwIvJZ2ovGV69M",
                        "toolName": "program",
                        "result": {
                            "result": "{\"sku\":\"sku_123\",\"availableUnits\":42,\"requestedUnits\":31,\"sufficient\":true}",
                            "status": "completed",
                        },
                        "providerMetadata": {
                            "openai": {
                                "itemId": "cmo_0742d30c1d273351016a6145f6ba7c819f93fcba5b06569347",
                            },
                        },
                    })),
                ],
            );
            assert!(
                content.iter().any(|part| {
                    part["type"] == "text"
                        && part["text"].as_str().is_some_and(|text| {
                            text.contains("Inventory is sufficient for `sku_123`")
                        })
                }),
                "{:#}",
                json!(content)
            );
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › programmatic tool calling › should stream programmatic tool calling across steps from real fixtures
        #[tokio::test]
        async fn should_stream_programmatic_tool_calling_across_steps_from_real_fixtures() {
            let mut parts = Vec::new();
            let fixtures = [
                json_chunks_fixture!("programmatic-tool-calling.1"),
                json_chunks_fixture!("programmatic-tool-calling.2"),
                json_chunks_fixture!("programmatic-tool-calling.3"),
            ];
            for fixture in fixtures {
                let r = collect_parts(
                    "gpt-5.6",
                    fixture,
                    json!({ "prompt": test_prompt(), "tools": tools() }),
                )
                .await;
                parts.extend(r.parts.as_array().unwrap().iter().cloned());
            }

            assert_array_containing(
                &parts,
                &[
                    Err(json!({
                        "type": "tool-call",
                        "toolCallId": "call_voPdoCqf8APY4DMpam3bdmxq",
                        "toolName": "program",
                        "providerExecuted": true,
                    })),
                    Err(json!({
                        "type": "tool-call",
                        "toolCallId": "call_VgDSZztLociNcutQZWkC2fmL",
                        "toolName": "getInventory",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "fc_0bac52ec5f239d30016a61460099bc8192a9ebe7381b9efd87",
                                "caller": {
                                    "type": "program",
                                    "callerId": "call_voPdoCqf8APY4DMpam3bdmxq",
                                },
                            },
                        },
                    })),
                    Err(json!({
                        "type": "tool-call",
                        "toolCallId": "call_8GZvm5Bs4q0YSJIFH8hZeIcp",
                        "toolName": "getDemand",
                        "providerMetadata": {
                            "openai": {
                                "itemId": "fc_0bac52ec5f239d30016a6146031b5081928dcd2cd4ed0747ff",
                                "caller": {
                                    "type": "program",
                                    "callerId": "call_voPdoCqf8APY4DMpam3bdmxq",
                                },
                            },
                        },
                    })),
                    Err(json!({
                        "type": "tool-result",
                        "toolCallId": "call_voPdoCqf8APY4DMpam3bdmxq",
                        "toolName": "program",
                        "result": {
                            "result": "{\"inventory\":{\"availableUnits\":42,\"sku\":\"sku_123\"},\"demand\":{\"requestedUnits\":31,\"sku\":\"sku_123\"}}",
                            "status": "completed",
                        },
                    })),
                ],
            );
            let text: String = parts
                .iter()
                .filter(|part| part["type"] == "text-delta")
                .filter_map(|part| part["delta"].as_str())
                .collect();
            assert!(
                text.contains("Inventory is sufficient for `sku_123`"),
                "{text}"
            );
        }
    }
}

// ── Older ports (previously `tests/openai_upstream_e2e_test.rs`) ──
//
// Small end-to-end subset of the pinned upstream tests, using an in-process transport.

mod older_upstream_e2e {
    use crate::openai::tools::{FileSearchArgs, file_search};
    use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
    use crate::test_support::mock_fetch::{Canned, MockFetch};
    use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
    use aimux_core::error::AiMuxError;
    use aimux_core::image_model::{
        ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
    };
    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::LanguageModelMessage;
    use aimux_core::options::CallOptions;
    use aimux_core::result::GenerateContent;
    use aimux_core::shared::{Size, provider_namespace};
    use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel};
    use aimux_core::stream_part::StreamPart;
    use aimux_core::tool::{FunctionTool, Tool};
    use aimux_core::transcription_model::{
        AudioInput, TranscriptionCallOptions, TranscriptionModel,
    };
    use aimux_core::types::FinishReasonUnified;
    use futures::StreamExt;
    use serde_json::{Value, json};

    fn provider(fetch: &std::sync::Arc<MockFetch>) -> OpenAIProvider {
        create_openai(OpenAIProviderSettings {
            base_url: Some("https://example.test/v1".into()),
            api_key: Some("test-key".into()),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap()
    }
    fn options() -> CallOptions {
        CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
    }
    fn chat_response() -> Value {
        json!({"id":"chatcmpl-test","model":"test-model","created":1711115037,"choices":[{"index":0,"message":{"role":"assistant","content":"Hello, World!"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"total_tokens":34,"completion_tokens":30}})
    }
    fn responses_response() -> Value {
        json!({"id":"resp_test","created_at":1741257730,"output":[{"id":"msg_test","type":"message","content":[{"type":"output_text","text":"answer text","annotations":[]}]}],"usage":{"input_tokens":345,"input_tokens_details":{"cached_tokens":234,"cache_write_tokens":45,"future_input_detail":{"tokens":7}},"output_tokens":538,"output_tokens_details":{"reasoning_tokens":123,"future_output_detail":["preserved"]},"total_tokens":572,"future_usage_field":{"value":true}}})
    }
    fn sse(events: Vec<Value>) -> Canned {
        Canned {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: events
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>()
                .into_bytes(),
        }
    }
    fn assert_request(fetch: &MockFetch, endpoint: &str, expected: Value) {
        let seen = fetch.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].url, format!("https://example.test/v1{endpoint}"));
        assert_eq!(seen[0].headers["authorization"], "Bearer test-key");
        assert_eq!(seen[0].json_body(), expected);
    }

    /// partial: `should extract text response`; full port in src/openai/tests/chat/chat_language_model.rs.
    /// partial: `should send request body`; full port in src/openai/tests/chat/chat_language_model.rs.
    /// partial: `should extract usage`; full port in src/openai/tests/chat/chat_language_model.rs.
    #[tokio::test]
    async fn chat_generate() {
        let fetch = MockFetch::new(vec![Canned::json(&chat_response())]);
        let result = provider(&fetch)
            .chat("test-model")
            .do_generate(&options())
            .await
            .unwrap();
        let body = json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}]});
        assert_request(&fetch, "/chat/completions", body.clone());
        assert_eq!(result.request.unwrap().body, Some(body));
        assert!(
            matches!(&result.content[..],[GenerateContent::Text{text,..}] if text=="Hello, World!")
        );
        assert_eq!(result.usage.input_tokens.total, Some(4));
        assert_eq!(result.usage.input_tokens.no_cache, Some(4));
        assert_eq!(result.usage.output_tokens.text, Some(30));
        assert_eq!(
            result.usage.raw,
            chat_response()["usage"].as_object().cloned()
        );
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(
            result.response.unwrap().id.as_deref(),
            Some("chatcmpl-test")
        );
    }

    /// Chat drops the legacy `promptCacheRetention` for models that take
    /// `promptCacheOptions` and names them like the AI SDK does
    /// (`chat/openai-chat-language-model.ts:235`).
    #[tokio::test]
    async fn chat_prompt_cache_retention_warning_names_gpt_6_models() {
        let fetch = MockFetch::new(vec![Canned::json(&chat_response())]);
        let mut opts = options();
        opts.provider_options = Some(
            provider_namespace("openai", json!({"promptCacheRetention":"24h"}))
                .expect("provider metadata must be an object"),
        );
        let result = provider(&fetch)
            .chat("gpt-6-astra")
            .do_generate(&opts)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body().get("prompt_cache_retention"),
            None
        );
        assert_eq!(
        result.warnings,
        vec![aimux_core::types::Warning::Unsupported {
            feature: "promptCacheRetention".into(),
            details: Some(
                "promptCacheRetention is not supported by GPT-6 and later models; use promptCacheOptions instead".into()
            ),
        }]
    );
    }

    /// A singleton `allOf: [{ $ref }]` (Zod 4's recursive reference) becomes a
    /// direct `$ref`, and is expanded from its definition at the schema root,
    /// like the AI SDK (`normalize-openai-json-schema.ts:176-208`).
    #[tokio::test]
    async fn chat_json_schema_unwraps_singleton_reference_all_of() {
        let fetch = MockFetch::new(vec![Canned::json(&chat_response())]);
        let person = json!({
            "type": "object",
            "properties": {
                "firstName": {"type": "string"},
                "relatives": {"type": "array", "items": {"allOf": [{"$ref": "#/definitions/person"}]}},
            },
            "required": ["firstName", "relatives"],
            "additionalProperties": false,
        });
        let mut opts = options();
        opts.response_format = Some(aimux_core::options::ResponseFormat::Json {
            schema: Some(json!({
                "allOf": [{"$ref": "#/definitions/person"}],
                "definitions": {"person": person},
            })),
            name: None,
            description: None,
        });
        provider(&fetch)
            .chat("gpt-4o")
            .do_generate(&opts)
            .await
            .unwrap();
        let direct = json!({
            "type": "object",
            "properties": {
                "firstName": {"type": "string"},
                "relatives": {"type": "array", "items": {"$ref": "#/definitions/person"}},
            },
            "required": ["firstName", "relatives"],
            "additionalProperties": false,
        });
        let mut expected = direct.clone();
        expected["definitions"] = json!({"person": direct});
        assert_eq!(
            fetch.seen()[0].json_body()["response_format"]["json_schema"]["schema"],
            expected
        );
    }

    /// partial: `should stream text deltas`; full port in src/openai/tests/chat/chat_language_model.rs.
    #[tokio::test]
    async fn chat_stream() {
        let chunk = |delta: Value, finish: Value| json!({"id":"chatcmpl-test","model":"test-model","created":1702657020,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]});
        let fetch = MockFetch::new(vec![sse(vec![
            chunk(json!({"role":"assistant","content":""}), Value::Null),
            chunk(json!({"content":"Hello"}), Value::Null),
            chunk(json!({"content":", "}), Value::Null),
            chunk(json!({"content":"World!"}), Value::Null),
            chunk(json!({}), json!("stop")),
            json!({"id":"chatcmpl-test","model":"test-model","created":1702657020,"choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}),
        ])]);
        let result = provider(&fetch)
            .chat("test-model")
            .do_stream(&options())
            .await
            .unwrap();
        let parts = result
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        assert_request(
            &fetch,
            "/chat/completions",
            json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}],"stream":true,"stream_options":{"include_usage":true}}),
        );
        let text = parts
            .iter()
            .filter_map(|part| match part {
                StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(text, "Hello, World!");
        assert_eq!(
            parts
                .iter()
                .filter(|part| matches!(part, StreamPart::TextStart { .. }))
                .count(),
            1
        );
        assert_eq!(
            parts
                .iter()
                .filter(|part| matches!(part, StreamPart::TextEnd { .. }))
                .count(),
            1
        );
        assert!(
            matches!(parts.last(),Some(StreamPart::Finish{finish_reason,usage,..}) if finish_reason.unified==FinishReasonUnified::Stop && usage.input_tokens.total==Some(17) && usage.output_tokens.total==Some(227))
        );
    }

    /// partial: `should parse tool results`; full port in src/openai/tests/chat/chat_language_model.rs.
    #[tokio::test]
    async fn chat_tool_call() {
        let mut response = chat_response();
        response["choices"][0]["message"]["content"] = json!("");
        response["choices"][0]["message"]["tool_calls"] = json!([{"id":"call_test","type":"function","function":{"name":"test-tool","arguments":"{\"value\":\"Spark\"}"}}]);
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let schema = json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false});
        let mut opts = options();
        opts.tools = Some(vec![Tool::Function(FunctionTool::new(
            "test-tool",
            schema.clone(),
        ))]);
        let result = provider(&fetch)
            .chat("test-model")
            .do_generate(&opts)
            .await
            .unwrap();
        let body = fetch.seen()[0].json_body();
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"], json!([{"role":"user","content":"Hello"}]));
        assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
        assert_eq!(body["tools"][0]["function"]["parameters"], schema);
        assert!(
            matches!(&result.content[..],[GenerateContent::ToolCall(call)] if call.tool_call_id=="call_test" && call.tool_name=="test-tool" && call.input==r#"{"value":"Spark"}"#)
        );
    }

    /// partial: `should stream tool call with missing type field (Azure AI Foundry / Mistral)`; full port in src/openai/tests/chat/chat_language_model.rs.
    /// Upstream: @ai-sdk/openai 4.0.83 src/chat/openai-chat-language-model.ts:539-542
    /// creates the stream's tool-call tracker with `typeValidation: 'if-present'`:
    /// a tool-call delta without `type` starts a call, one whose `type` is
    /// present and not `function` is invalid response data.
    #[tokio::test]
    async fn chat_stream_tool_call_type_is_validated_if_present() {
        let tool_stream = |delta: Value| {
            sse(vec![
                json!({"id":"chatcmpl-test","model":"test-model","created":1702657020,"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[delta]},"finish_reason":null}]}),
                json!({"id":"chatcmpl-test","model":"test-model","created":1702657020,"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
            ])
        };
        let fetch = MockFetch::new(vec![
            tool_stream(
                json!({"index":0,"id":"call_1","function":{"name":"test-tool","arguments":"{}"}}),
            ),
            tool_stream(
                json!({"index":0,"id":"call_2","type":"custom","function":{"name":"test-tool","arguments":"{}"}}),
            ),
        ]);
        let model = provider(&fetch).chat("test-model");

        let untyped = model
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .collect::<Vec<_>>()
            .await;
        assert!(
        untyped.iter().any(
            |part| matches!(part, Ok(StreamPart::ToolCall(call)) if call.tool_call_id == "call_1")
        ),
        "{untyped:?}"
    );
        assert!(untyped.iter().all(Result::is_ok), "{untyped:?}");

        let custom = model
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .collect::<Vec<_>>()
            .await;
        assert!(
        custom.iter().any(|part| matches!(part, Err(error) if error.to_string().contains("Expected 'function' type."))),
        "{custom:?}"
    );
        assert!(
            !custom
                .iter()
                .any(|part| matches!(part, Ok(StreamPart::ToolCall(_))))
        );
    }

    /// partial: `should throw an api error when the first stream chunk is an error`; full port in src/openai/tests/chat/chat_language_model.rs.
    /// Upstream: @ai-sdk/openai 4.0.83 src/openai-stream-error.ts:18-37,
    /// :235-267 and :303-315: an error frame before any output fails the call
    /// with the status `createOpenAIProviderStreamError` gives it — an HTTP
    /// status in `code`, else one inferred from `code`/`type` — and
    /// `insufficient_quota` is never retryable.
    #[tokio::test]
    async fn chat_stream_error_frame_status_is_inferred_as_upstream() {
        let cases = [
            (
                json!({"message":"quota","type":"insufficient_quota","code":"insufficient_quota","param":null}),
                429,
                false,
            ),
            (
                json!({"message":"slow down","type":"requests","code":"rate_limit_exceeded"}),
                429,
                true,
            ),
            (
                json!({"message":"busy","type":"server_error","code":"503"}),
                503,
                true,
            ),
            (
                json!({"message":"bad","type":"invalid_request_error","code":null}),
                400,
                false,
            ),
            (
                json!({"message":"oops","type":"server_error","code":null}),
                500,
                true,
            ),
        ];
        let fetch = MockFetch::new(
            cases
                .iter()
                .map(|(error, ..)| sse(vec![json!({ "error": error })]))
                .collect(),
        );
        let model = provider(&fetch).chat("test-model");
        for (frame, status, retryable) in cases {
            let Err(AiMuxError::ApiCall(error)) = model.do_stream(&options()).await.map(|_| ())
            else {
                panic!("expected an API error for {frame}");
            };
            assert_eq!(error.message, frame["message"], "{frame}");
            assert_eq!(error.status_code, Some(status), "{frame}");
            assert_eq!(error.is_retryable, retryable, "{frame}");
        }
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/openai-stream-error.ts:207-217 types a
    /// `response.failed` frame as `response.failed`, so its status comes from
    /// the inner error's `code` alone, never from an inner `type`.
    #[tokio::test]
    async fn responses_failed_frame_status_ignores_the_inner_type() {
        let fetch = MockFetch::new(vec![sse(vec![json!({
            "type":"response.failed","sequence_number":1,
            "response":{"error":{"code":"server_error","message":"boom","type":"invalid_request_error"}}
        })])]);
        let Err(AiMuxError::ApiCall(error)) = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
            .map(|_| ())
        else {
            panic!("expected an API error");
        };
        assert_eq!(error.message, "boom");
        assert_eq!(error.status_code, Some(500));
        assert!(error.is_retryable);
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/chat/map-openai-finish-reason.ts:13-15
    /// maps the legacy `function_call` finish reason to `tool-calls`; the raw
    /// value stays `function_call`.
    #[tokio::test]
    async fn chat_function_call_finish_reason_is_tool_calls() {
        let mut response = chat_response();
        response["choices"][0]["finish_reason"] = json!("function_call");
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let result = provider(&fetch)
            .chat("test-model")
            .do_generate(&options())
            .await
            .unwrap();
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("function_call"));
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/chat/convert-openai-chat-usage.ts:28
    /// reads cache reads only from the nested `prompt_tokens_details.cached_tokens`;
    /// a top-level `cached_tokens` (Moonshot's wire format) is not a cache read and
    /// only survives in `usage.raw`.
    #[tokio::test]
    async fn chat_cache_read_is_only_the_nested_cached_tokens() {
        let usage = |usage: Value| {
            let mut response = chat_response();
            response["usage"] = usage;
            response
        };
        let fetch = MockFetch::new(vec![
            Canned::json(&usage(
                json!({"prompt_tokens":100,"completion_tokens":10,"cached_tokens":7,"prompt_tokens_details":{"cached_tokens":25}}),
            )),
            Canned::json(&usage(
                json!({"prompt_tokens":100,"completion_tokens":10,"cached_tokens":7}),
            )),
        ]);
        let model = provider(&fetch).chat("test-model");

        let nested = model.do_generate(&options()).await.unwrap().usage;
        assert_eq!(nested.input_tokens.cache_read, Some(25));
        assert_eq!(nested.input_tokens.no_cache, Some(75));
        assert_eq!(nested.raw.unwrap()["cached_tokens"], json!(7));

        let top_level_only = model.do_generate(&options()).await.unwrap().usage;
        assert_eq!(top_level_only.input_tokens.cache_read, Some(0));
        assert_eq!(top_level_only.input_tokens.no_cache, Some(100));
        assert_eq!(top_level_only.raw.unwrap()["cached_tokens"], json!(7));
    }

    /// openai-chat-api.ts:71-87 (response) and :159-175 (chunk) parse a logprobs
    /// entry as `{token, logprob, top_logprobs: [{token, logprob}]}`, so `bytes`
    /// never reaches `providerMetadata.openai.logprobs`
    /// (openai-chat-language-model.ts:459-461 generate, :613-615 stream), and a
    /// `null` `content` sets none.
    #[tokio::test]
    async fn chat_logprobs_metadata_has_no_bytes() {
        let wire = json!({"content":[{"token":"Hello","logprob":-0.5,"bytes":[72,101,108,108,111],
        "top_logprobs":[{"token":"Hello","logprob":-0.5,"bytes":[72,101,108,108,111]},
            {"token":"Hi","logprob":-1.5,"bytes":[72,105]}]}],"refusal":null});
        let expected = json!([{"token":"Hello","logprob":-0.5,
        "top_logprobs":[{"token":"Hello","logprob":-0.5},{"token":"Hi","logprob":-1.5}]}]);
        let generated_metadata = |logprobs: Value| async move {
            let mut response = chat_response();
            response["choices"][0]["logprobs"] = logprobs;
            let fetch = MockFetch::new(vec![Canned::json(&response)]);
            let result = provider(&fetch)
                .chat("test-model")
                .do_generate(&options())
                .await
                .unwrap();
            serde_json::to_value(result.provider_metadata).unwrap()
        };
        assert_eq!(
            generated_metadata(wire.clone()).await["openai"]["logprobs"],
            expected
        );
        let metadata = generated_metadata(json!({"content": null})).await;
        assert_eq!(metadata["openai"].get("logprobs"), None, "{metadata}");

        let fetch = MockFetch::new(vec![sse(vec![json!({
        "id":"chatcmpl-test","model":"test-model","created":1702657020,
        "choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},
            "logprobs":wire,"finish_reason":"stop"}]})])]);
        let parts = provider(&fetch)
            .chat("test-model")
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        let Some(StreamPart::Finish {
            provider_metadata, ..
        }) = parts.last()
        else {
            panic!("no finish part: {parts:?}");
        };
        assert_eq!(
            serde_json::to_value(provider_metadata).unwrap()["openai"]["logprobs"],
            expected
        );
    }

    /// partial: `should parse OpenRouter resource exhausted error`; full port in src/openai/tests/error.rs.
    #[tokio::test]
    async fn chat_error_response() {
        let message = "{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n";
        let response = json!({"error":{"message":message,"code":429}});
        let mut canned = Canned::json(&response);
        canned.status = 429;
        let fetch = MockFetch::new(vec![canned]);
        let error = provider(&fetch)
            .chat("test-model")
            .do_generate(&options())
            .await
            .unwrap_err();
        assert_request(
            &fetch,
            "/chat/completions",
            json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}]}),
        );
        let AiMuxError::ApiCall(error) = error else {
            panic!("expected API error")
        };
        assert_eq!(error.message, message);
        assert_eq!(error.status_code, Some(429));
        assert_eq!(error.data, Some(response));
    }

    /// partial: `should generate text`; full port in src/openai/tests/responses/responses_language_model.rs.
    /// partial: `should extract usage`; full port in src/openai/tests/responses/responses_language_model.rs.
    #[tokio::test]
    async fn responses_generate() {
        let response = responses_response();
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_generate(&options())
            .await
            .unwrap();
        assert_request(
            &fetch,
            "/responses",
            json!({"model":"test-model","input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}]}),
        );
        assert_eq!(
            result.content,
            vec![GenerateContent::Text {
                text: "answer text".into(),
                provider_metadata: Some(
                    provider_namespace("openai", json!({"itemId":"msg_test"}))
                        .expect("provider metadata must be an object")
                )
            }]
        );
        assert_eq!(result.usage.input_tokens.total, Some(345));
        assert_eq!(result.usage.input_tokens.cache_read, Some(234));
        assert_eq!(result.usage.input_tokens.cache_write, Some(45));
        assert_eq!(result.usage.input_tokens.no_cache, Some(66));
        assert_eq!(result.usage.output_tokens.total, Some(538));
        assert_eq!(result.usage.output_tokens.reasoning, Some(123));
        assert_eq!(result.usage.output_tokens.text, Some(415));
        assert_eq!(result.usage.raw, response["usage"].as_object().cloned());
        assert_eq!(result.response.unwrap().id.as_deref(), Some("resp_test"));
    }

    /// partial: `should stream text deltas`; full port in src/openai/tests/responses/responses_language_model.rs.
    #[tokio::test]
    async fn responses_stream() {
        let events=json!([{"type": "response.created", "response": {"id": "resp_67c9a81b6a048190a9ee441c5755a4e8", "object": "response", "created_at": 1741269019, "status": "in_progress", "error": null, "incomplete_details": null, "input": [], "instructions": null, "max_output_tokens": null, "model": "test-model", "output": [], "parallel_tool_calls": true, "previous_response_id": null, "reasoning": {"effort": null, "summary": null}, "store": true, "temperature": 0.3, "text": {"format": {"type": "text"}}, "tool_choice": "auto", "tools": [], "top_p": 1, "truncation": "disabled", "usage": null, "user": null, "metadata": {}}}, {"type": "response.in_progress", "response": {"id": "resp_67c9a81b6a048190a9ee441c5755a4e8", "object": "response", "created_at": 1741269019, "status": "in_progress", "error": null, "incomplete_details": null, "input": [], "instructions": null, "max_output_tokens": null, "model": "test-model", "output": [], "parallel_tool_calls": true, "previous_response_id": null, "reasoning": {"effort": null, "summary": null}, "store": true, "temperature": 0.3, "text": {"format": {"type": "text"}}, "tool_choice": "auto", "tools": [], "top_p": 1, "truncation": "disabled", "usage": null, "user": null, "metadata": {}}}, {"type": "response.output_item.added", "output_index": 0, "item": {"id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "type": "message", "status": "in_progress", "role": "assistant", "content": []}}, {"type": "response.content_part.added", "item_id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": "", "annotations": [], "logprobs": []}}, {"type": "response.output_text.delta", "item_id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "output_index": 0, "content_index": 0, "delta": "Hello,", "logprobs": []}, {"type": "response.output_text.delta", "item_id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "output_index": 0, "content_index": 0, "delta": " World!", "logprobs": []}, {"type": "response.output_text.done", "item_id": "msg_67c9a8787f4c8190b49c858d4c1cf20c", "output_index": 0, "content_index": 0, "text": "Hello, World!"}, {"type": "response.content_part.done", "item_id": "msg_67c9a8787f4c8190b49c858d4c1cf20c", "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": "Hello, World!", "annotations": [], "logprobs": []}}, {"type": "response.output_item.done", "output_index": 0, "item": {"id": "msg_67c9a8787f4c8190b49c858d4c1cf20c", "type": "message", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": "Hello, World!", "annotations": [], "logprobs": []}]}}, {"type": "response.completed", "response": {"id": "resp_67c9a878139c8190aa2e3105411b408b", "object": "response", "created_at": 1741269112, "status": "completed", "error": null, "incomplete_details": null, "input": [], "instructions": null, "max_output_tokens": null, "model": "test-model", "output": [{"id": "msg_67c9a8787f4c8190b49c858d4c1cf20c", "type": "message", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": "Hello, World!", "annotations": []}]}], "parallel_tool_calls": true, "previous_response_id": null, "reasoning": {"effort": null, "summary": null}, "store": true, "temperature": 0.3, "text": {"format": {"type": "text"}}, "tool_choice": "auto", "tools": [], "top_p": 1, "truncation": "disabled", "usage": {"input_tokens": 543, "input_tokens_details": {"cached_tokens": 234}, "output_tokens": 478, "output_tokens_details": {"reasoning_tokens": 123}, "total_tokens": 512}, "user": null, "metadata": {}}}]).as_array().unwrap().clone();
        let fetch = MockFetch::new(vec![sse(events)]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
            .unwrap();
        let parts = result
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        assert_request(
            &fetch,
            "/responses",
            json!({"model":"test-model","input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}],"stream":true}),
        );
        assert_eq!(
            serde_json::to_value(parts).unwrap(),
            json!([
                {"type": "stream-start", "warnings": []},
                {
                    "type": "response-metadata",
                    "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                    "modelId": "test-model",
                    "timestamp": "2025-03-06T13:50:19.000Z"
                },
                {
                    "type": "text-start",
                    "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                    "providerMetadata": {"openai": {"itemId": "msg_67c9a81dea8c8190b79651a2b3adf91e"}}
                },
                {"type": "text-delta", "id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "delta": "Hello,"},
                {"type": "text-delta", "id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "delta": " World!"},
                {
                    "type": "text-end",
                    "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                    "providerMetadata": {"openai": {"itemId": "msg_67c9a81dea8c8190b79651a2b3adf91e"}}
                },
                {
                    "type": "finish",
                    "finishReason": {"unified": "stop"},
                    "providerMetadata": {"openai": {"responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8"}},
                    "usage": {
                        "inputTokens": {"total": 543, "noCache": 309, "cacheRead": 234},
                        "outputTokens": {"total": 478, "reasoning": 123, "text": 355},
                        "raw": {
                            "input_tokens": 543,
                            "input_tokens_details": {"cached_tokens": 234},
                            "output_tokens": 478,
                            "output_tokens_details": {"reasoning_tokens": 123},
                            "total_tokens": 512
                        }
                    }
                }
            ])
        );
    }

    /// partial: `should throw a descriptive error when the response has no output`; full port in src/openai/tests/responses/responses_language_model.rs.
    #[tokio::test]
    async fn responses_error_response() {
        let fetch = MockFetch::new(vec![Canned::json(
            &json!({"id":"resp_no_output","status":"incomplete","incomplete_details":{"reason":"content_filter"}}),
        )]);
        let error = provider(&fetch)
            .responses("test-model")
            .do_generate(&options())
            .await
            .unwrap_err();
        assert_request(
            &fetch,
            "/responses",
            json!({"model":"test-model","input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}]}),
        );
        assert!(
            error
                .to_string()
                .contains("Responses API returned no output (content_filter)"),
            "{error}"
        );
    }

    fn embedding_response() -> Value {
        serde_json::from_str(
            r#"{
  "object": "list",
  "data": [
    {
      "object": "embedding",
      "index": 0,
      "embedding": [
        0.0057293195, -0.012727811, 0.020042092, -0.013437585, 0.022833068
      ]
    },
    {
      "object": "embedding",
      "index": 1,
      "embedding": [
        -0.037104916, -0.05178114, -0.008340587, 0.001164541, -0.0035253682
      ]
    }
  ],
  "model": "test-embedding",
  "usage": {
    "prompt_tokens": 12,
    "total_tokens": 12
  }
}
"#,
        )
        .unwrap()
    }

    fn image_response() -> Value {
        serde_json::from_str(r#"{
  "created": 1770935200,
  "data": [
    {
      "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAA3CGNhQlgAADcIanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj",
      "revised_prompt": "A small and adorable baby sea otter. This little creature is covered in a thick and fluffy brown fur, its tiny paws are slightly visible. The otter has bright, curious eyes and it's floating on its back on a calm sea, surrounded by floating seaweed."
    },
    {
      "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAEp2GNhQlgAASnYanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
    }
  ]
}
"#).unwrap()
    }

    fn transcription_response() -> Value {
        serde_json::from_str(r#"{
  "task": "transcribe",
  "language": "english",
  "duration": 36.709999084472656,
  "text": "Galileo was an American robotic space program that studied the planet Jupiter and its moons, as well as several other solar system bodies.",
  "words": [
    {
      "word": "Galileo",
      "start": 0,
      "end": 0.6600000262260437
    },
    {
      "word": "was",
      "start": 0.6600000262260437,
      "end": 0.8999999761581421
    },
    {
      "word": "an",
      "start": 0.8999999761581421,
      "end": 1.1399999856948853
    },
    {
      "word": "American",
      "start": 1.1399999856948853,
      "end": 1.5
    },
    {
      "word": "robotic",
      "start": 1.5,
      "end": 2.0199999809265137
    }
  ],
  "usage": {
    "type": "duration",
    "seconds": 37
  }
}
"#).unwrap()
    }

    /// partial: `should extract embedding`; full port in src/openai/tests/embedding/embedding_model.rs.
    /// partial: `should pass the model and the values`; full port in src/openai/tests/embedding/embedding_model.rs.
    #[tokio::test]
    async fn embedding_generate() {
        let response = embedding_response();
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let mut opts = EmbeddingCallOptions::new("sunny day at the beach");
        opts.values.push("rainy day in the city".into());
        let result = provider(&fetch)
            .embedding("test-embedding")
            .do_embed(&opts)
            .await
            .unwrap();
        assert_request(
            &fetch,
            "/embeddings",
            json!({"model":"test-embedding","input":opts.values,"encoding_format":"float"}),
        );
        assert_eq!(
            result.embeddings,
            vec![
                vec![
                    0.0057293195,
                    -0.012727811,
                    0.020042092,
                    -0.013437585,
                    0.022833068
                ],
                vec![
                    -0.037104916,
                    -0.05178114,
                    -0.008340587,
                    0.001164541,
                    -0.0035253682
                ]
            ]
        );
        assert_eq!(result.usage.unwrap().tokens, 12);
        assert_eq!(result.response.unwrap().body, Some(response));
    }

    /// partial: `should pass the model and the settings`; full port in src/openai/tests/image/image_model.rs.
    /// partial: `should extract the generated images`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn image_generate() {
        let response = image_response();
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let mut opts = ImageCallOptions::new("A cute baby sea otter");
        opts.size = Some(Size::new(1024, 1024));
        opts.provider_options = provider_namespace("openai", json!({"style":"vivid"}))
            .expect("provider metadata must be an object");
        let result = provider(&fetch)
            .image("test-image")
            .do_generate(&opts)
            .await
            .unwrap();
        assert_request(
            &fetch,
            "/images/generations",
            json!({"model":"test-image","prompt":"A cute baby sea otter","n":1,"size":"1024x1024","style":"vivid","response_format":"b64_json"}),
        );
        let expected = response["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|image| image["b64_json"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert!(matches!(result.images,ImageOutputs::Base64(images) if images==expected));
    }

    /// openai-image-model.ts:166 (files) and :357 (mask) fetch a URL file with
    /// `downloadBlob(file.url, {abortSignal})`, which goes through
    /// `fetchUntrustedUrl` (provider-utils download-blob.ts:111-114): a
    /// non-public target is refused before any request, and the edit request is
    /// never sent.
    #[tokio::test]
    async fn image_edit_url_files_are_downloaded_through_the_guard() {
        let url = || ImageFile::Url {
            url: "http://169.254.169.254/latest/meta-data/image.png".into(),
        };
        let mut files = ImageCallOptions::new("Edit");
        files.files = Some(vec![url()]);
        let mut mask = ImageCallOptions::new("Edit");
        mask.files = Some(vec![ImageFile::File {
            media_type: "image/png".into(),
            data: ImageFileData::Binary(vec![1, 2, 3]),
        }]);
        mask.mask = Some(url());
        for options in [files, mask] {
            let fetch = MockFetch::new(vec![Canned::json(&image_response())]);
            let error = provider(&fetch)
                .image("gpt-image-1")
                .do_generate(&options)
                .await
                .unwrap_err();
            assert!(
                matches!(error, AiMuxError::InvalidArgument { ref message, .. } if message.contains("non-public")),
                "{error:?}"
            );
            assert!(fetch.seen().is_empty());
        }
    }

    /// partial: `should pass the model and text`; full port in src/openai/tests/speech/speech_model.rs.
    /// partial: `should return audio data with correct content type`; full port in src/openai/tests/speech/speech_model.rs.
    #[tokio::test]
    async fn speech_generate() {
        let audio = vec![1, 2, 3, 4];
        let fetch = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![("content-type".into(), "audio/mpeg".into())],
            body: audio.clone(),
        }]);
        let result = provider(&fetch)
            .speech("test-speech")
            .do_generate(&SpeechCallOptions::new("Hello from the AI SDK!"))
            .await
            .unwrap();
        assert_request(
            &fetch,
            "/audio/speech",
            json!({"model":"test-speech","input":"Hello from the AI SDK!","voice":"alloy","response_format":"mp3"}),
        );
        assert!(matches!(result.audio,AudioData::Binary(bytes) if bytes==audio));
        assert_eq!(
            result.response.headers.unwrap()["content-type"],
            "audio/mpeg"
        );
    }

    /// partial: `should extract the transcription text`; full port in src/openai/tests/transcription/transcription_model.rs.
    #[tokio::test]
    async fn transcription_generate() {
        let response = transcription_response();
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let opts = TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3]), "audio/wav");
        let result = provider(&fetch)
            .transcription("test-transcription")
            .do_generate(&opts)
            .await
            .unwrap();
        let seen = fetch.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, "https://example.test/v1/audio/transcriptions");
        assert_eq!(seen[0].method, "POST");
        assert!(seen[0].headers["content-type"].starts_with("multipart/form-data; boundary="));
        let body = String::from_utf8_lossy(&seen[0].body);
        assert!(body.contains("name=\"model\"\r\n\r\ntest-transcription\r\n"));
        assert!(body.contains("name=\"file\"; filename=\"audio.wav\""));
        assert!(seen[0].body.windows(3).any(|window| window == [1, 2, 3]));
        assert_eq!(result.text, response["text"].as_str().unwrap());
        assert_eq!(result.language.as_deref(), Some("en"));
        assert_eq!(result.duration_in_seconds, Some(36.709999084472656));
        assert_eq!(result.segments.len(), 5);
    }

    /// partial: `should handle streaming web search without action`; full port in src/openai/tests/responses/responses_language_model.rs.
    #[tokio::test]
    async fn responses_web_search_stream() {
        let fetch = MockFetch::new(vec![sse(vec![
            json!({"type":"response.created","response":{"id":"resp_missing_action","created_at":1741630255,"model":"test-model"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"web_search_call","id":"ws_missing_action","status":"in_progress"}}),
            json!({"type":"response.web_search_call.in_progress","output_index":0,"item_id":"ws_missing_action"}),
            json!({"type":"response.web_search_call.completed","output_index":0,"item_id":"ws_missing_action"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"type":"web_search_call","id":"ws_missing_action","status":"completed"}}),
            json!({"type":"response.completed","response":{"id":"resp_missing_action","usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":2,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":12}}}),
        ])]);
        let mut call = options();
        call.tools = Some(vec![Tool::Provider(aimux_core::tool::ProviderTool {
            id: "openai.web_search".parse().unwrap(),
            name: "webSearch".into(),
            args: serde_json::Map::new(),
            supports_deferred_results: None,
        })]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_stream(&call)
            .await
            .unwrap();
        let parts = result
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        assert_request(
            &fetch,
            "/responses",
            json!({"model":"test-model","input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}],"stream":true,"tools":[{"type":"web_search"}],"include":["web_search_call.action.sources"]}),
        );
        assert_eq!(
            serde_json::to_value(parts).unwrap(),
            json!([
                {"type":"stream-start","warnings":[]},
                {"type":"response-metadata","id":"resp_missing_action","modelId":"test-model","timestamp":"2025-03-10T18:10:55.000Z"},
                {"type":"tool-input-start","id":"ws_missing_action","toolName":"webSearch","providerExecuted":true},
                {"type":"tool-input-end","id":"ws_missing_action"},
                {"type":"tool-call","toolCallId":"ws_missing_action","toolName":"webSearch","input":"{}","providerExecuted":true},
                {"type":"tool-result","toolCallId":"ws_missing_action","toolName":"webSearch","result":{}},
                {"type":"finish","finishReason":{"unified":"stop"},"usage":{"inputTokens":{"total":10,"noCache":10,"cacheRead":0},"outputTokens":{"total":2,"reasoning":0,"text":2},"raw":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":2,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":12}},"providerMetadata":{"openai":{"responseId":"resp_missing_action"}}}
            ])
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/openai-provider.ts:387 puts
    /// `openaiTools` on the provider, so `openai.tools.webSearch(…)` makes the
    /// provider tool; it is the module's factory and reaches the request.
    #[tokio::test]
    async fn provider_tools_member_makes_the_provider_tools() {
        use crate::openai::tools::{SearchContextSize, WebSearchArgs};
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let openai = provider(&fetch);
        let args = || WebSearchArgs {
            search_context_size: Some(SearchContextSize::High),
            ..Default::default()
        };
        let tool = openai.tools.web_search(args()).unwrap();
        let factory = crate::openai::tools::web_search(args()).unwrap();
        assert_eq!(
            (&tool.id, &tool.name, &tool.args),
            (&factory.id, &factory.name, &factory.args)
        );
        assert_eq!(openai.tools.apply_patch().id, "openai.apply_patch");
        // Of the OpenAI tools only programmatic tool calling supports deferred
        // results (src/tool/programmatic-tool-calling.ts:55).
        assert_eq!(
            openai
                .tools
                .programmatic_tool_calling()
                .supports_deferred_results,
            Some(true)
        );
        assert_eq!(openai.tools.apply_patch().supports_deferred_results, None);
        let mut call = options();
        call.tools = Some(vec![Tool::Provider(tool)]);
        openai
            .responses("test-model")
            .do_generate(&call)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body()["tools"],
            json!([{"type":"web_search","search_context_size":"high"}])
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-language-model.ts:1660-1663
    /// enqueues `{ type: 'raw', rawValue }` for every chunk, before the parts
    /// derived from it, when `includeRawChunks` is set, and nothing otherwise.
    #[tokio::test]
    async fn responses_stream_emits_raw_chunks_when_asked() {
        let events = vec![
            json!({"type":"response.created","response":{"id":"resp_raw","created_at":1741630255,"model":"test-model"}}),
            json!({"type":"response.completed","response":{"id":"resp_raw","usage":{"input_tokens":1,"output_tokens":1}}}),
        ];
        let fetch = MockFetch::new(vec![sse(events.clone()), sse(events.clone())]);
        let model = provider(&fetch).responses("test-model");
        let mut call = options();
        call.include_raw_chunks = Some(true);
        let with_raw: Vec<_> = model
            .do_stream(&call)
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect()
            .await;
        assert!(matches!(&with_raw[0], StreamPart::StreamStart { .. }));
        assert!(matches!(&with_raw[1], StreamPart::Raw { raw_value } if *raw_value == events[0]));
        assert!(matches!(&with_raw[2], StreamPart::ResponseMetadata(_)));
        assert!(matches!(&with_raw[3], StreamPart::Raw { raw_value } if *raw_value == events[1]));
        assert!(matches!(with_raw.last(), Some(StreamPart::Finish { .. })));

        let without_raw: Vec<_> = model
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect()
            .await;
        assert!(
            !without_raw
                .iter()
                .any(|part| matches!(part, StreamPart::Raw { .. }))
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83
    /// src/responses/openai-responses-language-model.ts:1587 starts `responseId` at
    /// `null` and only `response.created` sets it, so a stream without that event
    /// finishes with `responseId: null`, not an empty string.
    #[tokio::test]
    async fn a_stream_without_response_created_finishes_with_a_null_response_id() {
        let fetch = MockFetch::new(vec![sse(vec![json!({
            "type":"response.completed",
            "response":{"id":"resp_1","created_at":1,"model":"test-model","status":"completed","output":[],
                "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}
        })])]);
        let parts: Vec<_> = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect()
            .await;
        let Some(StreamPart::Finish {
            provider_metadata: Some(metadata),
            ..
        }) = parts.last()
        else {
            panic!("expected a finish part with provider metadata: {parts:?}");
        };
        assert_eq!(metadata["openai"]["responseId"], Value::Null);
    }

    /// Upstream: @ai-sdk/openai 4.0.83
    /// src/responses/openai-responses-language-model.ts:2735-2749 gives every
    /// `reasoning-delta` the reasoning item's id as `providerMetadata.<key>.itemId`,
    /// as it does for `reasoning-start` and `reasoning-end`: the id the item
    /// started with at its `output_index` (`resolveOutputItemId`), even when the
    /// provider rotates the id on later events.
    #[tokio::test]
    async fn reasoning_deltas_carry_the_item_id() {
        let fetch = MockFetch::new(vec![sse(vec![
            json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"rs_1","type":"reasoning","summary":[]}}),
            json!({"type":"response.reasoning_summary_part.added","item_id":"rs_2","output_index":0,
            "summary_index":0,"part":{"type":"summary_text","text":""}}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_3","output_index":0,
            "summary_index":0,"delta":"Thinking"}),
        ])]);
        let parts: Vec<_> = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect()
            .await;
        let delta = parts
            .iter()
            .find_map(|part| match part {
                StreamPart::ReasoningDelta {
                    provider_metadata, ..
                } => Some(provider_metadata),
                _ => None,
            })
            .expect("a reasoning delta");
        assert_eq!(
            serde_json::to_value(delta).unwrap(),
            json!({ "openai": { "itemId": "rs_1" } })
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/tool/file-search.ts:29 takes
    /// `maxNumResults` as a JS number and
    /// src/responses/openai-responses-prepare-tools.ts:219 sends it as is, so
    /// `maxNumResults: 5` reaches the API's integer field as `5`, not `5.0`.
    #[tokio::test]
    async fn file_search_sends_max_num_results_as_an_integer() {
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let args: FileSearchArgs =
            serde_json::from_value(json!({ "vectorStoreIds": ["vs_1"], "maxNumResults": 5 }))
                .unwrap();
        let mut options = options();
        options.tools = Some(vec![Tool::Provider(file_search(args).unwrap())]);
        provider(&fetch)
            .responses("test-model")
            .do_generate(&options)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body()["tools"][0]["max_num_results"],
            json!(5)
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83
    /// src/responses/openai-responses-language-model.ts:128 (`mapComputerAction`)
    /// builds each computer action from the fields its type has and adds `keys`
    /// only when it is not null, so a `double_click` with `keys: null` reaches the
    /// tool input without `keys`.
    #[tokio::test]
    async fn computer_actions_leave_out_null_keys() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "id": "resp_computer",
            "created_at": 1,
            "model": "test-model",
            "output": [{
                "type": "computer_call",
                "id": "computer_item",
                "call_id": "computer_call",
                "status": "completed",
                "actions": [
                    {"type": "double_click", "x": 1, "y": 2, "keys": null},
                    {"type": "scroll", "x": 3, "y": 4, "scroll_x": 0, "scroll_y": 5, "keys": null}
                ],
                "pending_safety_checks": []
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_generate(&options())
            .await
            .unwrap();
        let Some(GenerateContent::ToolCall(call)) = result.content.first() else {
            panic!("expected a tool call: {:?}", result.content);
        };
        assert_eq!(
            call.input,
            r#"{"actions":[{"type":"double_click","x":1,"y":2},{"type":"scroll","x":3,"y":4,"scrollX":0,"scrollY":5}],"pendingSafetyChecks":[],"status":"completed"}"#
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83
    /// src/responses/openai-responses-language-model.ts:378 maps the names of
    /// twelve provider tools back to the caller's tool names
    /// (`providerToolNames`); `computer_use` is not one of them, so a computer
    /// call without a `call_id` (line 1358) keeps the name `computer_use` even
    /// when the request declares an `openai.computer_use` tool with another name.
    #[tokio::test]
    async fn computer_use_results_keep_the_canonical_tool_name() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "id": "resp_computer_use",
            "created_at": 1,
            "model": "test-model",
            "output": [{"type": "computer_call", "id": "computer_item", "status": "completed"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))]);
        let mut options = options();
        options.tools = Some(vec![Tool::Provider(aimux_core::tool::ProviderTool {
            id: "openai.computer_use".parse().unwrap(),
            name: "computerUse".into(),
            args: serde_json::Map::new(),
            supports_deferred_results: None,
        })]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_generate(&options)
            .await
            .unwrap();
        let names: Vec<&str> = result
            .content
            .iter()
            .filter_map(|part| match part {
                GenerateContent::ToolCall(call) => Some(call.tool_name.as_str()),
                GenerateContent::ToolResult(result) => Some(result.tool_name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["computer_use", "computer_use"]);
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:1520
    /// parses a `container_file_citation` annotation with only `type`,
    /// `container_id`, `file_id`, `filename`, `start_index` and `end_index`, and
    /// src/responses/openai-responses-language-model.ts:1072 puts the parsed
    /// annotations in the text part's provider metadata, so an extra `index` is
    /// dropped.
    #[tokio::test]
    async fn text_annotations_keep_only_the_fields_of_their_type() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "id": "resp_annotations",
            "created_at": 1,
            "model": "test-model",
            "output": [{
                "type": "message",
                "id": "msg_container",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "Generated with container file.",
                    "annotations": [{
                        "type": "container_file_citation",
                        "container_id": "cntr_test",
                        "file_id": "file-container",
                        "filename": "data.csv",
                        "start_index": 0,
                        "end_index": 10,
                        "index": 2
                    }]
                }]
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_generate(&options())
            .await
            .unwrap();
        let Some(GenerateContent::Text {
            provider_metadata: Some(metadata),
            ..
        }) = result.content.first()
        else {
            panic!("expected a text part: {:?}", result.content);
        };
        assert_eq!(
            metadata["openai"]["annotations"],
            json!([{
                "type": "container_file_citation",
                "container_id": "cntr_test",
                "file_id": "file-container",
                "filename": "data.csv",
                "start_index": 0,
                "end_index": 10
            }])
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:1698
    /// parses an apply patch operation as `{type, path, diff}`, and
    /// src/responses/openai-responses-language-model.ts:1443 stringifies that
    /// parsed operation into the tool input, so its keys follow the schema order
    /// whatever order the wire sent them in.
    #[tokio::test]
    async fn apply_patch_input_lists_operation_fields_in_schema_order() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "id": "resp_apply_patch",
            "created_at": 1,
            "model": "test-model",
            "output": [{
                "type": "apply_patch_call",
                "id": "apc_1",
                "call_id": "call_1",
                "status": "completed",
                "operation": {"diff": "+hello\n", "type": "create_file", "path": "a.md"}
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_generate(&options())
            .await
            .unwrap();
        let Some(GenerateContent::ToolCall(call)) = result.content.first() else {
            panic!("expected a tool call: {:?}", result.content);
        };
        assert_eq!(
            call.input,
            r#"{"callId":"call_1","operation":{"type":"create_file","path":"a.md","diff":"+hello\n"}}"#
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:147
    /// parses a local shell action as `{type, command, timeout_ms, user,
    /// working_directory, env}`, and
    /// src/responses/openai-responses-language-model.ts:998 stringifies the
    /// parsed action into the tool input, so its keys follow that order.
    #[tokio::test]
    async fn local_shell_input_lists_action_fields_in_schema_order() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "id": "resp_local_shell",
            "created_at": 1,
            "model": "test-model",
            "output": [{
                "type": "local_shell_call",
                "id": "lsh_1",
                "call_id": "call_1",
                "status": "completed",
                "action": {"type": "exec", "command": ["ls"], "env": {}, "working_directory": "/root"}
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_generate(&options())
            .await
            .unwrap();
        let Some(GenerateContent::ToolCall(call)) = result.content.first() else {
            panic!("expected a tool call: {:?}", result.content);
        };
        assert_eq!(
            call.input,
            r#"{"action":{"type":"exec","command":["ls"],"working_directory":"/root","env":{}}}"#
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:1486
    /// lets a message's `content` hold only `output_text` parts, so a `refusal`
    /// part fails the response schema and the JSON response handler rejects the
    /// call with an `APICallError` ("Invalid JSON response").
    #[tokio::test]
    async fn a_refusal_message_part_rejects_the_response() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "id": "resp_refusal",
            "created_at": 1,
            "model": "test-model",
            "output": [{
                "type": "message",
                "id": "msg_refusal",
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "refusal", "refusal": "Cannot help with this."}]
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))]);
        let error = provider(&fetch)
            .responses("test-model")
            .do_generate(&options())
            .await
            .unwrap_err();
        let AiMuxError::ApiCall(detail) = &error else {
            panic!("expected an API call error: {error:?}");
        };
        assert!(
            detail.message.starts_with("Invalid JSON response"),
            "{error}"
        );
        assert!(
            detail
                .response_body
                .as_deref()
                .is_some_and(|body| body.contains("Cannot help with this.")),
            "{error:?}"
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:875
    /// (chunk) and :1490 (response) parse a logprobs entry as `{token, logprob,
    /// top_logprobs: [{token, logprob}]}`, so `bytes` never reaches
    /// `providerMetadata.openai.logprobs`
    /// (openai-responses-language-model.ts:1065 generate, :2685 stream).
    #[tokio::test]
    async fn responses_logprobs_metadata_has_no_bytes() {
        let wire = json!([{"token":"N","logprob":-2.5,"bytes":[78],
        "top_logprobs":[{"token":"N","logprob":-2.5,"bytes":[78]},{"token":"Y","logprob":-1.0,"bytes":[89]}]}]);
        let expected = json!([[{"token":"N","logprob":-2.5,
        "top_logprobs":[{"token":"N","logprob":-2.5},{"token":"Y","logprob":-1.0}]}]]);
        let mut opts = options();
        opts.provider_options = provider_namespace("openai", json!({"logprobs": 2})).ok();

        let mut response = responses_response();
        response["output"][0]["content"][0]["logprobs"] = wire.clone();
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let result = provider(&fetch)
            .responses("test-model")
            .do_generate(&opts)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(result.provider_metadata).unwrap()["openai"]["logprobs"],
            expected
        );

        let fetch = MockFetch::new(vec![sse(vec![
            json!({"type":"response.created","response":{"id":"resp_1","created_at":1,"model":"test-model"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1"}}),
            json!({"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"N","logprobs":wire}),
            json!({"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1}}}),
        ])]);
        let parts = provider(&fetch)
            .responses("test-model")
            .do_stream(&opts)
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        let Some(StreamPart::Finish {
            provider_metadata, ..
        }) = parts.last()
        else {
            panic!("no finish part: {parts:?}");
        };
        assert_eq!(
            serde_json::to_value(provider_metadata).unwrap()["openai"]["logprobs"],
            expected
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83
    /// src/responses/openai-responses-language-model.ts:1986 gives the
    /// `tool-input-end` of a streamed function call with a `namespace` the
    /// provider metadata `{ namespace }`.
    #[tokio::test]
    async fn a_streamed_namespaced_function_call_ends_its_input_with_the_namespace() {
        let fetch = MockFetch::new(vec![sse(vec![
            json!({"type":"response.created","response":{"id":"resp_1","created_at":1,"model":"test-model"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_1",
            "call_id":"call_1","name":"get_weather","arguments":"{}","status":"completed","namespace":"weather_ns"}}),
            json!({"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1}}}),
        ])]);
        let parts = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        let metadata = parts
            .iter()
            .find_map(|part| match part {
                StreamPart::ToolInputEnd {
                    provider_metadata, ..
                } => Some(provider_metadata),
                _ => None,
            })
            .expect("a tool-input-end part");
        assert_eq!(
            serde_json::to_value(metadata).unwrap(),
            json!({"openai": {"namespace": "weather_ns"}})
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83
    /// src/responses/openai-responses-language-model.ts:2709 (a summary part
    /// concluded when the next one starts) and :2762 (a summary part concluded
    /// on `reasoning_summary_part.done` with `store`) end the reasoning part with
    /// provider metadata `{ itemId }` only; the encrypted content goes on the
    /// ends sent when the reasoning item is done (line 2523).
    #[tokio::test]
    async fn early_reasoning_summary_ends_carry_only_the_item_id() {
        for store in [false, true] {
            let fetch = MockFetch::new(vec![sse(vec![
                json!({"type":"response.created","response":{"id":"resp_1","created_at":1,"model":"test-model"}}),
                json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","encrypted_content":"enc"}}),
                json!({"type":"response.reasoning_summary_part.added","item_id":"rs_1","output_index":0,"summary_index":0}),
                json!({"type":"response.reasoning_summary_part.done","item_id":"rs_1","output_index":0,"summary_index":0}),
                json!({"type":"response.reasoning_summary_part.added","item_id":"rs_1","output_index":0,"summary_index":1}),
                json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_1","encrypted_content":"enc"}}),
                json!({"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1}}}),
            ])]);
            let mut opts = options();
            opts.provider_options = provider_namespace("openai", json!({"store": store})).ok();
            let parts = provider(&fetch)
                .responses("test-model")
                .do_stream(&opts)
                .await
                .unwrap()
                .stream
                .map(|part| part.unwrap())
                .collect::<Vec<_>>()
                .await;
            let ends: Vec<Value> = parts
                .iter()
                .filter_map(|part| match part {
                    StreamPart::ReasoningEnd {
                        id,
                        provider_metadata,
                    } => Some(json!({"id": id, "providerMetadata": provider_metadata})),
                    _ => None,
                })
                .collect();
            assert_eq!(
                ends,
                [
                    json!({"id": "rs_1:0", "providerMetadata": {"openai": {"itemId": "rs_1"}}}),
                    json!({"id": "rs_1:1", "providerMetadata": {"openai": {"itemId": "rs_1", "reasoningEncryptedContent": "enc"}}}),
                ],
                "store: {store}"
            );
        }
    }

    /// Upstream: @ai-sdk/openai 4.0.83
    /// src/responses/openai-responses-language-model.ts:1633-1644 keeps the id
    /// each output item started with by `output_index` (OpenAI-compatible
    /// providers can rotate item ids between events) and resolves the reasoning
    /// summary events (:2688, :2736, :2751) and the reasoning item's end (:2503)
    /// through it.
    #[tokio::test]
    async fn rotated_reasoning_item_ids_are_correlated_by_output_index() {
        let fetch = MockFetch::new(vec![sse(vec![
            json!({"type":"response.created","response":{"id":"resp_1","created_at":1,"model":"test-model"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1"}}),
            json!({"type":"response.reasoning_summary_part.added","item_id":"rs_2","output_index":0,"summary_index":0}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_3","output_index":0,"summary_index":0,"delta":"thinking"}),
            json!({"type":"response.reasoning_summary_part.done","item_id":"rs_4","output_index":0,"summary_index":0}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_5"}}),
            json!({"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1}}}),
        ])]);
        let parts = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        let ids: Vec<(&str, &str)> = parts
            .iter()
            .filter_map(|part| match part {
                StreamPart::ReasoningStart { id, .. } => Some(("start", id.as_str())),
                StreamPart::ReasoningDelta { id, .. } => Some(("delta", id.as_str())),
                StreamPart::ReasoningEnd { id, .. } => Some(("end", id.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            ids,
            [("start", "rs_1:0"), ("delta", "rs_1:0"), ("end", "rs_1:0")]
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83
    /// src/responses/openai-responses-language-model.ts:1664-1680 turns a chunk
    /// that fails the Responses schema and looks like a Chat Completions chunk
    /// (:2962, a `choices` array and no string `type`) into an `APICallError`
    /// that names the mix-up (:2972), sends it as an error part and finishes with
    /// `error`.
    #[tokio::test]
    async fn a_chat_completions_chunk_on_a_responses_stream_gives_a_helpful_error() {
        let chunk = json!({"choices":[],"created":0,"id":"","model":"","object":""});
        let fetch = MockFetch::new(vec![sse(vec![chunk.clone()])]);
        let parts = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        let error = parts
            .iter()
            .find_map(|part| match part {
                StreamPart::Error { error } => Some(error),
                _ => None,
            })
            .expect("an error part");
        let AiMuxError::ApiCall(detail) = error else {
            panic!("expected an API call error: {error:?}");
        };
        assert!(
            detail.message.starts_with(
                "Received a Chat Completions stream while using the OpenAI Responses API."
            ),
            "{error}"
        );
        assert_eq!(
            detail.response_body.as_deref(),
            Some(chunk.to_string().as_str())
        );
        assert!(!detail.is_retryable);
        assert!(matches!(
            parts.last(),
            Some(StreamPart::Finish { finish_reason, .. }) if finish_reason.unified == FinishReasonUnified::Error
        ));
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:1338
    /// requires `output_index` on `response.function_call_arguments.done` (and
    /// :1095 on `response.output_item.done` of a modeled item), and a modeled
    /// chunk that fails its schema is not skipped as unknown (:1433-1441):
    /// src/responses/openai-responses-language-model.ts:1664-1680 sends an error
    /// part for it and finishes with `error`.
    #[tokio::test]
    async fn schema_invalid_known_responses_events_are_errors() {
        let fetch = MockFetch::new(vec![sse(vec![
            json!({"type":"response.created","response":{"id":"resp_1","created_at":1,"model":"test-model"}}),
            json!({"type":"response.function_call_arguments.done","item_id":"fc_1","arguments":"{}"}),
            json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1",
            "call_id":"call_1","name":"get_weather","arguments":"{}","status":"completed"}}),
            json!({"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1}}}),
        ])]);
        let parts = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
            .unwrap()
            .stream
            .map(|part| part.unwrap())
            .collect::<Vec<_>>()
            .await;
        let errors = parts
            .iter()
            .filter(|part| {
                matches!(
                    part,
                    StreamPart::Error {
                        error: AiMuxError::TypeValidation { .. }
                    }
                )
            })
            .count();
        assert_eq!(errors, 2, "{parts:?}");
        assert!(
            !parts
                .iter()
                .any(|part| matches!(part, StreamPart::ToolCall(_))),
            "{parts:?}"
        );
        assert!(matches!(
            parts.last(),
            Some(StreamPart::Finish { finish_reason, .. }) if finish_reason.unified == FinishReasonUnified::Error
        ));
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/openai-stream-error.ts:39 reads the
    /// stream until its first output event (`throwIfOpenAIStreamErrorBeforeOutput`,
    /// called at src/responses/openai-responses-language-model.ts:1552), so an
    /// error frame after `response.created` but before any output rejects
    /// `doStream` with an `APICallError` instead of becoming an error part.
    #[tokio::test]
    async fn a_responses_error_before_output_after_response_created_rejects_the_call() {
        let fetch = MockFetch::new(vec![sse(vec![
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1","created_at":1,"model":"test-model"}}),
            json!({"type":"error","sequence_number":1,"code":"rate_limit_exceeded","message":"Rate limit reached","param":null}),
        ])]);
        let Err(error) = provider(&fetch)
            .responses("test-model")
            .do_stream(&options())
            .await
        else {
            panic!("doStream resolved; expected it to reject");
        };
        let AiMuxError::ApiCall(detail) = &error else {
            panic!("expected an API call error: {error:?}");
        };
        assert_eq!(detail.message, "Rate limit reached");
        assert_eq!(detail.status_code, Some(429));
        assert!(detail.is_retryable);
    }
    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-prepare-tools.ts:684-688
    /// drops `async: true` for a model without async tool calling and warns with
    /// the quoted tool name and the GPT-6 requirement.
    #[tokio::test]
    async fn responses_async_tool_warning_names_the_gpt_6_requirement() {
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let mut call = options();
        call.tools = Some(vec![Tool::Function(FunctionTool {
            provider_options: Some(
                serde_json::from_value(json!({"openai":{"async":true}})).unwrap(),
            ),
            ..FunctionTool::new("weather", json!({"type":"object","properties":{}}))
        })]);
        let result = provider(&fetch)
            .responses("gpt-5.6")
            .do_generate(&call)
            .await
            .unwrap();
        assert!(
            fetch.seen()[0].json_body()["tools"][0]
                .get("async")
                .is_none()
        );
        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([{
                "type": "unsupported",
                "feature": "async tool calling for \"weather\"",
                "details": "Async tool calling is only supported by GPT-6 and later models."
            }])
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-prepare-tools.ts:183-202
    /// and :454-504 quote the `allowedTools` entry in the warning feature and give
    /// the reason a tool is dropped: a deferred tool is not visible to
    /// `tool_choice.allowed_tools`.
    #[tokio::test]
    async fn responses_allowed_tools_warnings_quote_the_entry_and_give_the_reason() {
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let mut call = options();
        call.tools = Some(vec![
            Tool::Function(FunctionTool::new(
                "get_weather",
                json!({"type":"object","properties":{}}),
            )),
            Tool::Function(FunctionTool {
                provider_options: Some(
                    serde_json::from_value(json!({"openai":{"deferLoading":true}})).unwrap(),
                ),
                ..FunctionTool::new("deferred", json!({"type":"object","properties":{}}))
            }),
        ]);
        call.provider_options = Some(
            serde_json::from_value(
                json!({"openai":{"allowedTools":{"toolNames":["get_weather","deferred"]}}}),
            )
            .unwrap(),
        );
        let result = provider(&fetch)
            .responses("gpt-5.4")
            .do_generate(&call)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body()["tool_choice"],
            json!({"type":"allowed_tools","mode":"auto","tools":[{"type":"function","name":"get_weather"}]})
        );
        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([{
                "type": "unsupported",
                "feature": "allowedTools entry \"deferred\"",
                "details": "deferred tools are not visible to tool_choice.allowed_tools; the tool is removed from the allowed tools"
            }])
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-prepare-tools.ts:509-515
    /// names the dropped entries when no `allowedTools` entry can be expressed.
    #[tokio::test]
    async fn responses_allowed_tools_error_names_the_dropped_entries() {
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let mut call = options();
        call.tools = Some(vec![Tool::Function(FunctionTool {
            provider_options: Some(
                serde_json::from_value(json!({"openai":{"deferLoading":true}})).unwrap(),
            ),
            ..FunctionTool::new("deferred", json!({"type":"object","properties":{}}))
        })]);
        call.provider_options = Some(
            serde_json::from_value(json!({"openai":{"allowedTools":{"toolNames":["deferred"]}}}))
                .unwrap(),
        );
        let error = provider(&fetch)
            .responses("gpt-5.4")
            .do_generate(&call)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AiMuxError::UnsupportedFunctionality(message)
            if message == "allowedTools with only tools that cannot be allow-listed (deferred)"),
            "{error:?}"
        );
        assert!(fetch.seen().is_empty());
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-prepare-tools.ts:174-178
    /// rejects two descriptions for one tool namespace, quoting its name.
    #[tokio::test]
    async fn responses_conflicting_namespace_descriptions_quote_the_namespace() {
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let namespaced = |name: &str, description: &str| {
            Tool::Function(FunctionTool {
                provider_options: Some(
                    serde_json::from_value(
                        json!({"openai":{"namespace":{"name":"crm","description":description}}}),
                    )
                    .unwrap(),
                ),
                ..FunctionTool::new(name, json!({"type":"object","properties":{}}))
            })
        };
        let mut call = options();
        call.tools = Some(vec![
            namespaced("get_customer", "CRM tools"),
            namespaced("list_orders", "Other CRM tools"),
        ]);
        let error = provider(&fetch)
            .responses("gpt-5.4")
            .do_generate(&call)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AiMuxError::UnsupportedFunctionality(message)
            if message == "conflicting descriptions for OpenAI tool namespace \"crm\""),
            "{error:?}"
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/convert-to-openai-responses-input.ts:142-145
    /// and src/responses/openai-responses-prepare-tools.ts:780-783 resolve file and
    /// skill references with `resolveProviderReference`, whose
    /// `NoSuchProviderReferenceError` names the provider and the available ones.
    #[tokio::test]
    async fn responses_missing_provider_reference_names_the_available_providers() {
        let fetch = MockFetch::new(vec![]);
        let model = provider(&fetch).responses("gpt-5.4");
        let file_call = CallOptions::new(vec![
        serde_json::from_value(json!({"role":"user","content":[{"type":"file","mediaType":"image/png","data":{"type":"reference","reference":{"anthropic":"file-xyz"}}}]}))
            .unwrap(),
    ]);
        let mut skill_call = options();
        skill_call.tools = Some(vec![
        serde_json::from_value(json!({"type":"provider","id":"openai.shell","name":"shell","args":{"environment":{"type":"containerAuto","skills":[{"type":"skillReference","providerReference":{"anthropic":"skill_abc"}}]}}}))
            .unwrap(),
    ]);
        for call in [file_call, skill_call] {
            let error = model.do_generate(&call).await.unwrap_err();
            assert!(
                matches!(&error, AiMuxError::InvalidArgument { message, .. }
                if message == "No provider reference found for provider 'openai'. Available providers: anthropic"),
                "{error:?}"
            );
        }
        assert!(fetch.seen().is_empty());
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-language-model.ts:455-492
    /// warns about a request-level `reasoningEffortUpdate` the model does not
    /// support by listing the efforts it does support, and omits the update.
    #[tokio::test]
    async fn responses_unsupported_reasoning_effort_update_lists_the_supported_efforts() {
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let mut call = options();
        call.provider_options = Some(
            serde_json::from_value(json!({"openai":{"reasoningEffortUpdate":"none"}})).unwrap(),
        );
        let result = provider(&fetch)
            .responses("gpt-6-astra")
            .do_generate(&call)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body()["input"],
            json!([{"role":"user","content":[{"type":"input_text","text":"Hello"}]}])
        );
        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([{
                "type": "unsupported",
                "feature": "reasoningEffortUpdate",
                "details": "gpt-6-astra only supports the following reasoning efforts: low, medium, high, xhigh, max"
            }])
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/convert-to-openai-responses-input.ts:780-842
    /// replays a provider-executed tool search or program call in full when
    /// responses are not stored: the tool search and program checks come before
    /// the provider-executed one.
    #[tokio::test]
    async fn responses_replays_hosted_tool_search_and_program_calls_without_storage() {
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let mut call = CallOptions::new(vec![
        serde_json::from_value(json!({"role":"assistant","content":[
            {"type":"tool-call","toolCallId":"tsc_1","toolName":"tool_search","input":{"arguments":{"paths":["get_weather"]},"call_id":null},"providerExecuted":true,"providerOptions":{"openai":{"itemId":"tsc_1"}}},
            {"type":"tool-call","toolCallId":"program_call_1","toolName":"program","input":{"code":"1","fingerprint":"fp"},"providerExecuted":true,"providerOptions":{"openai":{"itemId":"program_item_1"}}}
        ]}))
        .unwrap(),
    ]);
        call.tools = Some(vec![
        serde_json::from_value(
            json!({"type":"provider","id":"openai.tool_search","name":"tool_search","args":{}}),
        )
        .unwrap(),
        serde_json::from_value(
            json!({"type":"provider","id":"openai.programmatic_tool_calling","name":"program","args":{}}),
        )
        .unwrap(),
    ]);
        call.provider_options =
            Some(serde_json::from_value(json!({"openai":{"store":false}})).unwrap());
        provider(&fetch)
            .responses("gpt-5.4")
            .do_generate(&call)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body()["input"],
            json!([
                {"type":"tool_search_call","id":"tsc_1","execution":"server","call_id":null,"status":"completed","arguments":{"paths":["get_weather"]}},
                {"type":"program","id":"program_item_1","call_id":"program_call_1","code":"1","fingerprint":"fp"}
            ])
        );
    }

    /// Upstream: @ai-sdk/openai 4.0.83 src/responses/convert-to-openai-responses-input.ts:1646-1683
    /// sends the `caller` provider option of a tool result on its
    /// `function_call_output`, linking a nested call to its program.
    #[tokio::test]
    async fn responses_function_call_output_keeps_the_program_caller() {
        let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
        let call = CallOptions::new(vec![
        serde_json::from_value(json!({"role":"tool","content":[
            {"type":"tool-result","toolCallId":"function_call_1","toolName":"get_inventory","output":{"type":"text","value":"42"},"providerOptions":{"openai":{"caller":{"type":"program","callerId":"program_call_1"}}}}
        ]}))
        .unwrap(),
    ]);
        provider(&fetch)
            .responses("gpt-5.4")
            .do_generate(&call)
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body()["input"],
            json!([{"type":"function_call_output","call_id":"function_call_1","output":"42","caller":{"type":"program","caller_id":"program_call_1"}}])
        );
    }
}
