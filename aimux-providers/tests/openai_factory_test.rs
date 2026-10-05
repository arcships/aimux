//! `create_openai` against the recorded AI SDK behavior.
//!
//! `fixtures/aisdk/openai/*.json` hold what `@ai-sdk/openai`'s `createOpenAI`
//! sent over a mocked fetch for a fixed input: URL, headers, body and the
//! model's `provider` string. Each fixture is replayed here through an injected
//! [`Fetch`] (the factory's `fetch` setting), so the test also proves the
//! transport is the one the settings name. The expected request comes from the
//! fixture; the inputs are rebuilt from the fixture's recorded `sdk.input`.
//!
//! Two differences from the recording are deliberate and asserted around:
//! the `user-agent` header is the SDK's own identifier and aimux sends none,
//! and the recorded `authorization` is redacted, so the key the case used is
//! substituted back.

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
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, TextPart,
};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool, ToolChoice};
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt, Resolvable,
};
use aimux_providers::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};

// ── injected transport ───────────────────────────────────────────────────────

const KEY: &str = "sk-test-fixture";

const ENV_KEY: &str = "sk-env-should-not-be-used";

#[derive(Clone)]
struct Canned {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Canned {
    fn json(body: &Value) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(body).unwrap(),
        }
    }
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

fn settings(mock: &Arc<MockFetch>) -> OpenAIProviderSettings {
    OpenAIProviderSettings {
        api_key: Some(Resolvable::Value(KEY.to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    }
}

/// Sets (or removes) an environment variable and restores it on drop. Tests
/// that use it are `#[serial]`.
struct EnvVar {
    name: &'static str,
    saved: Option<String>,
}

impl EnvVar {
    fn set(name: &'static str, value: Option<&str>) -> Self {
        let saved = std::env::var(name).ok();
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        Self { name, saved }
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        unsafe {
            match &self.saved {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

// ── fixtures ─────────────────────────────────────────────────────────────────

/// Every fixture under `fixtures/aisdk/openai/`; the directory listing is
/// compared to this so a new fixture cannot go unreplayed.
const FIXTURES: [&str; 11] = [
    "chat-basic",
    "chat-custom-name",
    "chat-headers-merge",
    "chat-provider-options",
    "chat-stream-basic",
    "chat-system-and-settings",
    "chat-tools",
    "embedding-basic",
    "empty-api-key",
    "missing-api-key",
    "responses-basic",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/aisdk/openai")
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

    /// The transport answering with the recorded response (or a plain chat
    /// completion for the case that never reached the wire).
    fn mock(&self) -> Arc<MockFetch> {
        let response = &self.json["response"];
        if response.is_null() {
            return MockFetch::new(vec![Canned::json(&json!({
                "id": "chatcmpl-unused", "object": "chat.completion", "created": 1,
                "model": "gpt-4o",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "x"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            }))]);
        }
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
    /// (names case-insensitively, `user-agent` aside) and body.
    fn assert_request(&self, seen: &Seen, authorization: &str) {
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
        expected.remove("user-agent");
        assert_eq!(
            expected.get("authorization").map(String::as_str),
            Some("<redacted>"),
            "{}: the recording redacts the key",
            self.name
        );
        expected.insert("authorization".to_string(), authorization.to_string());
        let mut sent = seen.headers.clone();
        sent.remove("user-agent");
        assert_eq!(sent, expected, "{}: headers", self.name);

        assert_eq!(seen.json_body(), recorded["body"], "{}: body", self.name);
    }

    /// The call options the recording was made with.
    fn call_options(&self) -> CallOptions {
        call_options_from(self.sdk_input())
    }
}

fn message(role: Role, text: &str) -> LanguageModelMessage {
    match role {
        Role::System => LanguageModelMessage::System {
            content: text.to_string(),
            provider_options: None,
        },
        Role::User => LanguageModelMessage::user_text(text),
        Role::Assistant => LanguageModelMessage::Assistant {
            content: vec![AssistantPart::Text(TextPart {
                text: text.to_string(),
                provider_options: None,
            })],
            provider_options: None,
        },
        Role::Tool => panic!("tool messages require tool results"),
    }
}

/// Rebuild aimux call options from a recorded `sdk.input` (the subset the
/// OpenAI fixtures use).
fn call_options_from(input: &Value) -> CallOptions {
    let mut prompt: LanguageModelPrompt = Vec::new();
    if let Some(system) = input.get("system").and_then(Value::as_str) {
        prompt.push(message(Role::System, system));
    }
    if let Some(text) = input.get("prompt").and_then(Value::as_str) {
        prompt.push(message(Role::User, text));
    }
    for entry in input
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let role = match entry["role"].as_str().unwrap() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            other => panic!("unmapped role {other}"),
        };
        prompt.push(message(role, entry["content"].as_str().unwrap()));
    }
    let mut options = CallOptions::new(prompt);
    options.temperature = input.get("temperature").and_then(Value::as_f64);
    options.top_p = input.get("topP").and_then(Value::as_f64);
    options.max_output_tokens = input
        .get("maxOutputTokens")
        .and_then(Value::as_u64)
        .map(|v| v as u32);
    options.seed = input.get("seed").and_then(Value::as_u64);
    options.stop_sequences = input
        .get("stopSequences")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect());
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
        // `generateText` prepares its default tool choice before calling the provider.
        None if options.tools.is_some() => Some(ToolChoice::Auto),
        None => None,
        Some(other) => panic!("unmapped toolChoice {other}"),
    };
    options.provider_options = input
        .get("providerOptions")
        .map(|value| serde_json::from_value(value.clone()).unwrap());
    options.headers = input.get("headers").and_then(Value::as_object).map(|h| {
        h.iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect()
    });
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
    if let Some(raw) = recorded["rawFinishReason"].as_str() {
        assert_eq!(
            result.finish_reason.raw.as_deref(),
            Some(raw),
            "{}: raw finish",
            fixture.name
        );
    }
}

fn bearer(key: &str) -> String {
    format!("Bearer {key}")
}

// ── the fixtures ─────────────────────────────────────────────────────────────

#[test]
fn every_fixture_is_replayed() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures/aisdk/openai exists")
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

/// Replay a chat fixture whose request is a plain `generateText`.
async fn replay_chat(name: &'static str) {
    let fixture = Fixture::load(name);
    let mock = fixture.mock();
    let provider = create_openai(settings(&mock)).unwrap();
    let model = provider.chat(fixture.model_id());
    assert_eq!(
        model.provider(),
        fixture.provider_string(),
        "{name}: provider()"
    );
    assert_eq!(model.model_id(), fixture.model_id());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen.len(), 1, "{name}: exactly one request");
    fixture.assert_request(&seen[0], &bearer(KEY));
    assert_generate_result(&fixture, &result);
}

#[tokio::test]
async fn chat_basic() {
    replay_chat("chat-basic").await;
}

#[tokio::test]
async fn chat_system_and_settings() {
    replay_chat("chat-system-and-settings").await;
}

#[tokio::test]
async fn chat_provider_options_use_the_fixed_openai_namespace() {
    replay_chat("chat-provider-options").await;
}

#[tokio::test]
async fn chat_tools() {
    let fixture = Fixture::load("chat-tools");
    let mock = fixture.mock();
    let model = create_openai(settings(&mock)).unwrap().chat("gpt-4o");
    assert_eq!(model.provider(), fixture.provider_string());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    fixture.assert_request(&mock.seen()[0], &bearer(KEY));
    assert_generate_result(&fixture, &result);
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
async fn chat_custom_name() {
    let fixture = Fixture::load("chat-custom-name");
    let mock = fixture.mock();
    let provider = create_openai(OpenAIProviderSettings {
        name: Some("proxy".to_string()),
        ..settings(&mock)
    })
    .unwrap();
    let model = provider.chat(fixture.model_id());
    assert_eq!(model.provider(), "proxy.chat");
    assert_eq!(model.provider(), fixture.provider_string());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    // The name changes identity only, never the URL.
    fixture.assert_request(&mock.seen()[0], &bearer(KEY));
    assert_generate_result(&fixture, &result);
    assert_eq!(
        fixture.json["observations"]["providerIsProxyChat"],
        json!(true)
    );
}

#[tokio::test]
async fn chat_headers_merge() {
    let fixture = Fixture::load("chat-headers-merge");
    let mock = fixture.mock();
    // The recording was made with provider headers { "x-a": "1", "X-B": "2" }
    // (scripts/aisdk-fixtures/cases/openai.mjs).
    let headers: HeaderMapOpt = [("x-a", "1"), ("X-B", "2")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), Some(v.to_string())))
        .collect();
    let provider = create_openai(OpenAIProviderSettings {
        headers: Some(headers),
        ..settings(&mock)
    })
    .unwrap();
    let model = provider.chat(fixture.model_id());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    let seen = mock.seen();
    fixture.assert_request(&seen[0], &bearer(KEY));
    assert_generate_result(&fixture, &result);
    // Case-insensitive override: the call's `x-b` replaced the provider's
    // `X-B`, and no second `x-b` went out.
    assert_eq!(seen[0].headers["x-b"], "3");
    let sent: Vec<&String> = seen[0]
        .headers
        .keys()
        .filter(|n| n.starts_with("x-"))
        .collect();
    assert_eq!(sent, ["x-a", "x-b", "x-c"]);
    assert_eq!(
        fixture.json["observations"]["headerNamesAsSent"],
        json!(["x-a", "x-b", "x-c"])
    );
}

#[tokio::test]
async fn chat_stream_basic() {
    let fixture = Fixture::load("chat-stream-basic");
    let mock = fixture.mock();
    let model = create_openai(settings(&mock)).unwrap().chat("gpt-4o");
    assert_eq!(model.provider(), fixture.provider_string());

    let result = model.do_stream(&fixture.call_options()).await.unwrap();
    let parts: Vec<StreamPart> = result
        .stream
        .map(|part| part.expect("no stream error"))
        .collect()
        .await;

    // The recording's user-agent names the provider package; the stream case
    // is checked like the others.
    let mut seen = mock.seen();
    assert_eq!(seen.len(), 1);
    let sent = seen.remove(0);
    fixture.assert_request(&sent, &bearer(KEY));

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
            ..
        } => Some((finish_reason, usage)),
        _ => None,
    });
    let (reason, usage) = finish.expect("a finish part");
    assert_eq!(reason.unified, FinishReasonUnified::Stop);
    assert_eq!(usage.input_tokens.total, Some(12));
    assert_eq!(usage.output_tokens.total, Some(2));
}

#[tokio::test]
async fn responses_basic() {
    let fixture = Fixture::load("responses-basic");
    let mock = fixture.mock();
    let model = create_openai(settings(&mock)).unwrap().responses("gpt-4o");
    assert_eq!(model.provider(), fixture.provider_string());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    fixture.assert_request(&mock.seen()[0], &bearer(KEY));
    assert_generate_result(&fixture, &result);
}

#[tokio::test]
async fn embedding_basic() {
    let fixture = Fixture::load("embedding-basic");
    let mock = fixture.mock();
    let model = create_openai(settings(&mock))
        .unwrap()
        .embedding("text-embedding-3-small");
    assert_eq!(model.provider(), fixture.provider_string());

    let values: Vec<String> = fixture.sdk_input()["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let options = EmbeddingCallOptions {
        values,
        ..EmbeddingCallOptions::new("")
    };
    let result = model.do_embed(&options).await.unwrap();

    fixture.assert_request(&mock.seen()[0], &bearer(KEY));
    let recorded: Vec<Vec<f32>> =
        serde_json::from_value(fixture.json["result"]["embeddings"].clone()).unwrap();
    assert_eq!(result.embeddings, recorded);
    assert_eq!(
        result.usage.map(|u| u.tokens),
        fixture.json["result"]["usage"]["tokens"]
            .as_u64()
            .map(|v| v as u32)
    );
}

#[serial]
#[tokio::test]
async fn missing_api_key_fails_the_call_not_the_factory() {
    let fixture = Fixture::load("missing-api-key");
    let _env = EnvVar::set("OPENAI_API_KEY", None);
    let mock = fixture.mock();

    // Creating the provider and a model reads no key.
    let provider = create_openai(OpenAIProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("no key is read at creation");
    let model = provider.chat(fixture.model_id());
    assert_eq!(model.provider(), fixture.provider_string());

    let error = model
        .do_generate(&fixture.call_options())
        .await
        .expect_err("the call has no key");
    match &error {
        AiMuxError::LoadApiKey {
            env_var,
            description,
        } => {
            assert_eq!(env_var, "OPENAI_API_KEY");
            assert_eq!(description, "OpenAI");
        }
        other => panic!("expected LoadApiKey, got {other:?}"),
    }
    // Same text as the SDK's: names the setting and the variable, no value.
    let message = error.to_string();
    assert!(
        message.contains("OpenAI") && message.contains("OPENAI_API_KEY"),
        "{message}"
    );
    assert_eq!(
        fixture.json["error"]["name"].as_str(),
        Some("AI_LoadAPIKeyError")
    );
    assert!(mock.seen().is_empty(), "nothing was sent");
    assert_eq!(fixture.json["observations"]["requestsSent"], json!(0));
}

#[serial]
#[tokio::test]
async fn an_explicit_empty_key_is_sent_as_given() {
    let fixture = Fixture::load("empty-api-key");
    // The environment holds a key; an explicit "" must not fall back to it.
    let _env = EnvVar::set("OPENAI_API_KEY", Some(ENV_KEY));
    let mock = fixture.mock();
    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::Value(String::new())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.chat(fixture.model_id());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    // The wire value is exactly `Bearer ` (the SDK's headers object trims the
    // trailing space, which is what its observation records).
    fixture.assert_request(&seen[0], "Bearer ");
    assert_eq!(seen[0].headers["authorization"].trim(), "Bearer");
    assert!(!seen[0].headers["authorization"].contains(ENV_KEY));
    assert_eq!(
        fixture.json["observations"]["authorizationIsEmptyBearer"],
        json!(true)
    );
    assert_eq!(fixture.json["observations"]["usedEnvKey"], json!(false));
    assert_generate_result(&fixture, &result);
}

fn _assert_provider_is_shareable() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<OpenAIProvider>();
}
