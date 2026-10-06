//! `create_anthropic` against the recorded AI SDK behavior.
//!
//! `fixtures/aisdk/anthropic/*.json` hold what `@ai-sdk/anthropic`'s
//! `createAnthropic` sent over a mocked fetch for a fixed input: URL, headers,
//! body and the model's `provider` string. Each fixture is replayed here
//! through an injected [`Fetch`] (the factory's `fetch` setting), so the test
//! also proves the transport is the one the settings name. The expected
//! request comes from the fixture; the inputs are rebuilt from the fixture's
//! recorded `sdk.input`.
//!
//! The recorded `x-api-key` is redacted, so the key the case used is
//! substituted back. The recorded SDK and runtime user-agent identifiers are
//! replaced with the pinned provider package's user-agent suffix.

use aimux_core::tool::RawToolCall;
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput};
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool, ToolChoice};
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};
use aimux_providers::anthropic::{AnthropicProviderSettings, create_anthropic};

// ── injected transport ───────────────────────────────────────────────────────

const KEY: &str = "sk-test-fixture";

#[derive(Clone)]
struct Canned {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// One request as the transport saw it.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    url: String,
    /// Lower-cased names (the `http` crate normalizes them).
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Seen {
    fn json_body(&self) -> Value {
        serde_json::from_slice(&self.body).expect("request body is JSON")
    }
}

#[derive(Default)]
struct MockFetch {
    canned: Mutex<VecDeque<Canned>>,
    seen: Mutex<Vec<Seen>>,
}

impl MockFetch {
    fn new(canned: Vec<Canned>) -> Arc<Self> {
        Arc::new(Self {
            canned: Mutex::new(canned.into()),
            seen: Mutex::default(),
        })
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn transport(self: &Arc<Self>) -> FetchFunction {
        self.clone()
    }
}

#[async_trait]
impl Fetch for MockFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.seen.lock().unwrap().push(Seen {
            method: request.method.to_string(),
            url: request.url.to_string(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        value.to_str().expect("ASCII header").to_string(),
                    )
                })
                .collect(),
            body: request.body.to_vec(),
        });
        let canned = self
            .canned
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| FetchError::Other("no canned response left".into()))?;
        let mut headers = HeaderMap::new();
        for (name, value) in &canned.headers {
            headers.insert(
                HeaderName::try_from(name.as_str()).unwrap(),
                HeaderValue::try_from(value.as_str()).unwrap(),
            );
        }
        Ok(FetchResponse::from_bytes(
            StatusCode::from_u16(canned.status).unwrap(),
            headers,
            request.url,
            Bytes::from(canned.body),
        ))
    }
}

fn settings(mock: &Arc<MockFetch>) -> AnthropicProviderSettings {
    AnthropicProviderSettings {
        api_key: Some(KEY.to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    }
}

// ── fixtures ─────────────────────────────────────────────────────────────────

/// Every fixture under `fixtures/aisdk/anthropic/`; the directory listing is
/// compared to this so a new fixture cannot go unreplayed.
const FIXTURES: [&str; 5] = [
    "messages-basic",
    "messages-custom-name",
    "messages-provider-options",
    "messages-stream-basic",
    "messages-system-tools",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/aisdk/anthropic")
}

struct Fixture {
    name: &'static str,
    json: Value,
}

impl Fixture {
    fn load(name: &'static str) -> Self {
        assert!(FIXTURES.contains(&name), "unlisted fixture {name}");
        let path = fixtures_dir().join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        Self {
            name,
            json: serde_json::from_str(&text).expect("fixture is JSON"),
        }
    }

    fn provider_string(&self) -> &str {
        self.json["model"]["provider"].as_str().unwrap()
    }

    fn model_id(&self) -> &str {
        self.json["model"]["modelId"].as_str().unwrap()
    }

    fn sdk_input(&self) -> &Value {
        &self.json["sdk"]["input"]
    }

    /// The transport answering with the recorded response.
    fn mock(&self) -> Arc<MockFetch> {
        let response = &self.json["response"];
        let body = match &response["body"] {
            Value::String(text) => text.clone().into_bytes(),
            other => serde_json::to_vec(other).unwrap(),
        };
        MockFetch::new(vec![Canned {
            status: response["status"].as_u64().unwrap() as u16,
            headers: response["headers"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect(),
            body,
        }])
    }

    /// The recorded request must equal what went out: method, URL, headers
    /// (names case-insensitively, with the pinned user-agent) and body.
    fn assert_request(&self, seen: &Seen) {
        let recorded = &self.json["request"];
        assert_eq!(
            seen.method,
            recorded["method"].as_str().unwrap(),
            "{}",
            self.name
        );
        assert_eq!(seen.url, recorded["url"].as_str().unwrap(), "{}", self.name);

        let mut expected: BTreeMap<String, String> = recorded["headers"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.as_str().unwrap().to_string()))
            .collect();
        expected.insert(
            "user-agent".to_string(),
            "ai-sdk-anthropic/4.0.68 ai-sdk-provider-utils/5.0.51".to_string(),
        );
        assert_eq!(
            expected.get("x-api-key").map(String::as_str),
            Some("<redacted>"),
            "{}: the recording redacts the key",
            self.name
        );
        expected.insert("x-api-key".to_string(), KEY.to_string());
        assert_eq!(seen.headers, expected, "{}: headers", self.name);

        assert_eq!(seen.json_body(), recorded["body"], "{}: body", self.name);
    }

    /// The call options the recording was made with.
    fn call_options(&self) -> CallOptions {
        call_options_from(self.sdk_input())
    }
}

/// Rebuild aimux call options from a recorded `sdk.input` (the subset the
/// Anthropic fixtures use).
fn call_options_from(input: &Value) -> CallOptions {
    let mut prompt: LanguageModelPrompt = Vec::new();
    if let Some(system) = input.get("system").and_then(Value::as_str) {
        prompt.push(LanguageModelMessage::System {
            content: system.to_string(),
            provider_options: None,
        });
    }
    if let Some(text) = input.get("prompt").and_then(Value::as_str) {
        prompt.push(LanguageModelMessage::user_text(text));
    }
    let mut options = CallOptions::new(prompt);
    options.max_output_tokens = input
        .get("maxOutputTokens")
        .and_then(Value::as_u64)
        .map(|v| v as u32);
    if let Some(tools) = input.get("tools").and_then(Value::as_object) {
        options.tools = Some(
            tools
                .iter()
                .map(|(name, tool)| {
                    let mut function = FunctionTool::new(name.clone(), tool["inputSchema"].clone());
                    if let Some(description) = tool["description"].as_str() {
                        function = function.with_description(description);
                    }
                    Tool::Function(function)
                })
                .collect(),
        );
    }
    options.tool_choice = match input.get("toolChoice").and_then(Value::as_str) {
        Some("required") => Some(ToolChoice::Required),
        Some("none") => Some(ToolChoice::None),
        Some("auto") => Some(ToolChoice::Auto),
        None => options
            .tools
            .as_ref()
            .filter(|tools| !tools.is_empty())
            .map(|_| ToolChoice::Auto),
        Some(other) => panic!("unmapped toolChoice {other}"),
    };
    options.provider_options = input
        .get("providerOptions")
        .map(|options| serde_json::from_value(options.clone()).unwrap());
    options
}

fn generated_text(result: &GenerateResult) -> String {
    result
        .content
        .iter()
        .filter_map(|part| match part {
            GenerateContent::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Result fields every recorded `generateText` case pins.
fn assert_generate_result(fixture: &Fixture, result: &GenerateResult) {
    let recorded = &fixture.json["result"];
    assert_eq!(
        generated_text(result),
        recorded["text"].as_str().unwrap(),
        "{}: text",
        fixture.name
    );
    assert_eq!(
        result.response.as_ref().and_then(|r| r.id.as_deref()),
        recorded["response"]["id"].as_str(),
        "{}: response id",
        fixture.name
    );
    assert_eq!(
        result.response.as_ref().and_then(|r| r.model_id.as_deref()),
        recorded["response"]["modelId"].as_str(),
        "{}: response model",
        fixture.name
    );
    assert_eq!(
        result.usage.input_tokens.total,
        recorded["usage"]["inputTokens"].as_u64().map(|v| v as _),
        "{}: input tokens",
        fixture.name
    );
    assert_eq!(
        result.usage.output_tokens.total,
        recorded["usage"]["outputTokens"].as_u64().map(|v| v as _),
        "{}: output tokens",
        fixture.name
    );
    let unified = match recorded["finishReason"].as_str().unwrap() {
        "stop" => FinishReasonUnified::Stop,
        "tool-calls" => FinishReasonUnified::ToolCalls,
        other => panic!("unmapped finish reason {other}"),
    };
    assert_eq!(
        result.finish_reason.unified, unified,
        "{}: finish",
        fixture.name
    );
    assert_eq!(
        result.finish_reason.raw.as_deref(),
        recorded["rawFinishReason"].as_str(),
        "{}: raw finish",
        fixture.name
    );
    // The result-level providerMetadata: the raw usage, stop sequence,
    // iterations, container and context management, under the canonical key
    // and the provider's own.
    assert_eq!(
        serde_json::to_value(&result.provider_metadata).unwrap(),
        recorded["providerMetadata"],
        "{}: providerMetadata",
        fixture.name
    );
}

// ── the fixtures ─────────────────────────────────────────────────────────────

#[test]
fn every_fixture_is_replayed() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures/aisdk/anthropic exists")
        .map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".json")
                .unwrap_or_else(|| panic!("unexpected file {name}"))
                .to_string()
        })
        .collect();
    on_disk.sort();
    assert_eq!(
        on_disk, FIXTURES,
        "add a test for the new fixture and list it"
    );
}

/// Replay a fixture whose request is a plain `generateText`.
async fn replay_generate(name: &'static str) -> (Fixture, Arc<MockFetch>, GenerateResult) {
    let fixture = Fixture::load(name);
    let mock = fixture.mock();
    let provider = create_anthropic(settings(&mock)).unwrap();
    let model = provider.messages(fixture.model_id());
    assert_eq!(
        model.provider(),
        fixture.provider_string(),
        "{name}: provider()"
    );
    assert_eq!(model.model_id(), fixture.model_id());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen.len(), 1, "{name}: exactly one request");
    fixture.assert_request(&seen[0]);
    assert_generate_result(&fixture, &result);
    (fixture, mock, result)
}

#[tokio::test]
async fn messages_basic() {
    replay_generate("messages-basic").await;
}

#[tokio::test]
async fn messages_provider_options_use_the_canonical_namespace() {
    let (_, _, result) = replay_generate("messages-provider-options").await;
    // The reasoning block carries its signature under the canonical key.
    let reasoning = result
        .content
        .iter()
        .find_map(|part| match part {
            GenerateContent::Reasoning(ReasoningOutput {
                text,
                provider_metadata,
            }) => Some((text.clone(), provider_metadata.clone())),
            _ => None,
        })
        .expect("a reasoning part");
    assert_eq!(reasoning.0, "17 * 23 = 391.");
    assert_eq!(
        reasoning.1,
        Some(provider_namespace("anthropic", json!({ "signature": "sig_fixture" })).unwrap())
    );
}

#[tokio::test]
async fn messages_system_and_tools() {
    let (fixture, _, result) = replay_generate("messages-system-tools").await;
    let recorded = &fixture.json["result"]["content"][0];
    let call = result
        .content
        .iter()
        .find_map(|part| match part {
            GenerateContent::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        })
        .expect("a tool call");
    assert_eq!(call.0, recorded["toolCallId"].as_str().unwrap());
    assert_eq!(call.1, recorded["toolName"].as_str().unwrap());
    assert_eq!(
        serde_json::from_str::<Value>(&call.2).unwrap(),
        recorded["input"]
    );
}

#[tokio::test]
async fn messages_custom_name_reads_both_namespaces_and_the_custom_one_wins() {
    let fixture = Fixture::load("messages-custom-name");
    let mock = fixture.mock();
    let provider = create_anthropic(AnthropicProviderSettings {
        name: Some("myproxy".to_string()),
        ..settings(&mock)
    })
    .unwrap();
    let model = provider.messages(fixture.model_id());
    // The name is the provider string verbatim, with no `.messages` added.
    assert_eq!(model.provider(), "myproxy");
    assert_eq!(model.provider(), fixture.provider_string());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    // The name changes identity and the options namespace, never the URL; the
    // recorded body carries the `myproxy` user id, not the `anthropic` one.
    fixture.assert_request(&mock.seen()[0]);
    assert_generate_result(&fixture, &result);
    assert_eq!(
        fixture.json["observations"]["bodyMetadataUserId"],
        json!("from-myproxy-key")
    );
    assert_eq!(
        fixture.json["observations"]["modelProvider"],
        json!("myproxy")
    );
}

#[tokio::test]
async fn messages_stream_basic() {
    let fixture = Fixture::load("messages-stream-basic");
    let mock = fixture.mock();
    let model = create_anthropic(settings(&mock))
        .unwrap()
        .messages(fixture.model_id());
    assert_eq!(model.provider(), fixture.provider_string());

    let result = model.do_stream(&fixture.call_options()).await.unwrap();
    let parts: Vec<StreamPart> = result
        .stream
        .map(|part| part.expect("no stream error"))
        .collect()
        .await;

    // The recording's user-agent names the provider package; the stream case
    // is checked like the others.
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    fixture.assert_request(&seen[0]);

    let text: String = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello world");
    let finish = parts.iter().find_map(|part| match part {
        StreamPart::Finish {
            finish_reason,
            usage,
            provider_metadata,
        } => Some((finish_reason, usage, provider_metadata)),
        _ => None,
    });
    let (reason, usage, metadata) = finish.expect("a finish part");
    // `message_start`'s usage updated by `message_delta`'s.
    let finish_step = fixture.json["result"]["parts"]
        .as_array()
        .and_then(|parts| parts.iter().find(|p| p["type"] == "finish-step"))
        .expect("a recorded finish-step");
    assert_eq!(
        serde_json::to_value(metadata).unwrap(),
        finish_step["providerMetadata"]
    );
    assert_eq!(usage.raw.as_ref(), finish_step["usage"]["raw"].as_object());
    assert_eq!(reason.unified, FinishReasonUnified::Stop);
    assert_eq!(reason.raw.as_deref(), Some("end_turn"));
    assert_eq!(usage.input_tokens.total, Some(12));
    assert_eq!(usage.output_tokens.total, Some(2));
}
