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
//!
//! The rest covers what a fixture cannot: the key is evaluated per request
//! (`Resolvable::Future` once, `AsyncFn` every time), headers layer
//! provider -> call with `None` removing, every modality goes through the
//! same transport, and discovery is a single exchange.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::result::{GenerateContent, GenerateResult};
use aimux_core::shared::FileBytes;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool, ToolChoice};
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt, Resolvable,
};
use aimux_providers::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai, openai};

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
        assert_eq!(seen.headers, expected, "{}: headers", self.name);

        assert_eq!(seen.json_body(), recorded["body"], "{}: body", self.name);
    }

    /// The call options the recording was made with.
    fn call_options(&self) -> CallOptions {
        call_options_from(self.sdk_input())
    }
}

fn message(role: Role, text: &str) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content: vec![ContentPart::text(text)],
        ..Default::default()
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
        Some("required") => ToolChoice::Required,
        Some("none") => ToolChoice::None,
        Some("auto") | None => ToolChoice::Auto,
        Some(other) => panic!("unmapped toolChoice {other}"),
    };
    options.provider_options = input
        .get("providerOptions")
        .and_then(Value::as_object)
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
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
        result.response.id.as_deref(),
        recorded["response"]["id"].as_str(),
        "{}: response id",
        fixture.name
    );
    assert_eq!(
        result.response.model_id.as_deref(),
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
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
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

// ── key evaluation: once or every request ────────────────────────────────────

fn chat_ok() -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    }))
}

fn hello() -> CallOptions {
    CallOptions::new(vec![message(Role::User, "hi")])
}

async fn authorizations_over_three_calls(api_key: Resolvable<String>) -> Vec<String> {
    let mock = MockFetch::new(vec![chat_ok(), chat_ok(), chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some(api_key),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.chat("gpt-4o");
    for _ in 0..3 {
        model.do_generate(&hello()).await.unwrap();
    }
    mock.seen()
        .iter()
        .map(|s| s.headers["authorization"].clone())
        .collect()
}

#[tokio::test]
async fn a_future_key_is_awaited_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let key = Resolvable::from_future(async move {
        Ok(format!("key-{}", counter.fetch_add(1, Ordering::SeqCst)))
    });
    let sent = authorizations_over_three_calls(key).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1, "evaluated once");
    assert_eq!(
        sent,
        vec![bearer("key-0"); 3],
        "every request shares the outcome"
    );
}

#[tokio::test]
async fn an_async_fn_key_is_awaited_on_every_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let key = Resolvable::from_async_fn(move || {
        let counter = counter.clone();
        async move { Ok(format!("key-{}", counter.fetch_add(1, Ordering::SeqCst))) }
    });
    let sent = authorizations_over_three_calls(key).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3, "evaluated per request");
    assert_eq!(
        sent,
        vec![bearer("key-0"), bearer("key-1"), bearer("key-2")]
    );
}

#[tokio::test]
async fn a_sync_fn_key_is_called_on_every_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let key =
        Resolvable::from_fn(move || Ok(format!("key-{}", counter.fetch_add(1, Ordering::SeqCst))));
    let sent = authorizations_over_three_calls(key).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        sent,
        vec![bearer("key-0"), bearer("key-1"), bearer("key-2")]
    );
}

#[tokio::test]
async fn a_failing_key_producer_fails_the_call_and_sends_nothing() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::from_fn(|| {
            Err(AiMuxError::Other("vault is sealed".into()))
        })),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("the producer is not run at creation");
    let error = provider
        .chat("gpt-4o")
        .do_generate(&hello())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("vault is sealed"), "{error}");
    assert!(mock.seen().is_empty());
}

#[serial]
#[tokio::test]
async fn an_unset_key_reads_the_environment_on_every_request() {
    let mock = MockFetch::new(vec![chat_ok(), chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.chat("gpt-4o");
    {
        let _env = EnvVar::set("OPENAI_API_KEY", Some("env-one"));
        model.do_generate(&hello()).await.unwrap();
    }
    {
        let _env = EnvVar::set("OPENAI_API_KEY", Some("env-two"));
        model.do_generate(&hello()).await.unwrap();
    }
    let sent: Vec<String> = mock
        .seen()
        .iter()
        .map(|s| s.headers["authorization"].clone())
        .collect();
    assert_eq!(sent, [bearer("env-one"), bearer("env-two")]);
}

#[tokio::test]
async fn an_unusable_key_is_rejected_without_echoing_it() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::Value("sk-line\nbreak-secret".to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let error = provider
        .chat("gpt-4o")
        .do_generate(&hello())
        .await
        .unwrap_err();
    assert!(matches!(error, AiMuxError::InvalidArgument(_)), "{error:?}");
    assert!(!error.to_string().contains("secret"), "{error}");
    assert!(mock.seen().is_empty());
}

// ── headers ──────────────────────────────────────────────────────────────────

fn opt_map(entries: &[(&str, Option<&str>)]) -> HeaderMapOpt {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.map(str::to_string)))
        .collect()
}

#[tokio::test]
async fn organization_and_project_are_fixed_headers() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        organization: Some("org-1".into()),
        project: Some("proj-1".into()),
        ..settings(&mock)
    })
    .unwrap();
    provider.chat("gpt-4o").do_generate(&hello()).await.unwrap();
    let headers = &mock.seen()[0].headers;
    assert_eq!(headers["openai-organization"], "org-1");
    assert_eq!(headers["openai-project"], "proj-1");
    let names: Vec<&str> = headers.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "authorization",
            "content-type",
            "openai-organization",
            "openai-project"
        ],
        "no other header is added"
    );
}

#[tokio::test]
async fn a_none_header_removes_one_the_provider_would_send() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        organization: Some("org-1".into()),
        headers: Some(opt_map(&[
            ("openai-organization", None),
            ("X-Extra", Some("1")),
        ])),
        ..settings(&mock)
    })
    .unwrap();
    provider.chat("gpt-4o").do_generate(&hello()).await.unwrap();
    let headers = &mock.seen()[0].headers;
    assert!(!headers.contains_key("openai-organization"));
    assert_eq!(headers["x-extra"], "1");
}

#[tokio::test]
async fn user_headers_win_over_the_fixed_ones_and_call_headers_win_over_both() {
    let mock = MockFetch::new(vec![chat_ok(), chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        headers: Some(opt_map(&[("AUTHORIZATION", Some("Bearer from-headers"))])),
        ..settings(&mock)
    })
    .unwrap();
    let model = provider.chat("gpt-4o");
    model.do_generate(&hello()).await.unwrap();
    let mut options = hello();
    options.headers = Some(HashMap::from([(
        "authorization".to_string(),
        "Bearer from-call".to_string(),
    )]));
    model.do_generate(&options).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["authorization"], "Bearer from-headers");
    assert_eq!(seen[1].headers["authorization"], "Bearer from-call");
    assert_eq!(
        seen[1]
            .headers
            .keys()
            .filter(|n| *n == "authorization")
            .count(),
        1
    );
}

// ── URL, namespace, transform ────────────────────────────────────────────────

#[tokio::test]
async fn base_url_is_normalized_and_prefixes_every_endpoint() {
    let mock = MockFetch::new(vec![chat_ok(), chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        base_url: Some("https://gateway.example/openai/v1/".into()),
        ..settings(&mock)
    })
    .unwrap();
    provider.chat("gpt-4o").do_generate(&hello()).await.unwrap();
    provider
        .responses("gpt-4o")
        .do_generate(&hello())
        .await
        .ok();
    let urls: Vec<String> = mock.seen().into_iter().map(|s| s.url).collect();
    assert_eq!(
        urls,
        [
            "https://gateway.example/openai/v1/chat/completions",
            "https://gateway.example/openai/v1/responses"
        ]
    );
}

#[tokio::test]
async fn provider_options_are_read_from_openai_whatever_the_name() {
    let mock = MockFetch::new(vec![chat_ok(), chat_ok()]);
    let provider = create_openai(OpenAIProviderSettings {
        name: Some("proxy".into()),
        ..settings(&mock)
    })
    .unwrap();
    let model = provider.chat("gpt-4o");
    let mut options = hello();
    options.provider_options = Some(HashMap::from([
        ("openai".to_string(), json!({ "user": "from-openai" })),
        ("proxy".to_string(), json!({ "user": "from-proxy" })),
    ]));
    model.do_generate(&options).await.unwrap();
    assert_eq!(mock.seen()[0].json_body()["user"], "from-openai");
}

#[tokio::test]
async fn transform_request_body_runs_once_per_json_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let mock = MockFetch::new(vec![
        chat_ok(),
        Canned::json(
            &json!({"object": "list", "data": [{"object": "embedding", "index": 0, "embedding": [0.5]}], "model": "m", "usage": {"prompt_tokens": 1, "total_tokens": 1}}),
        ),
    ]);
    let provider = create_openai(OpenAIProviderSettings {
        transform_request_body: Some(Arc::new(move |mut body: Value| {
            counter.fetch_add(1, Ordering::SeqCst);
            body["x_tag"] = json!("t");
            body
        })),
        ..settings(&mock)
    })
    .unwrap();
    provider.chat("gpt-4o").do_generate(&hello()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    provider
        .embedding("text-embedding-3-small")
        .do_embed(&EmbeddingCallOptions::new("a"))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    for seen in mock.seen() {
        assert_eq!(seen.json_body()["x_tag"], "t");
    }
}

// ── every modality goes through the one transport ────────────────────────────

#[tokio::test]
async fn image_speech_transcription_and_files_use_the_injected_transport() {
    let mock = MockFetch::new(vec![
        Canned::json(&json!({"created": 1, "data": [{"b64_json": "aGk="}]})),
        Canned {
            status: 200,
            headers: vec![("content-type".into(), "audio/mpeg".into())],
            body: vec![1, 2, 3],
        },
        Canned::json(&json!({"text": "hello"})),
        Canned::json(
            &json!({"id": "file-1", "object": "file", "bytes": 3, "created_at": 1, "filename": "blob", "purpose": "assistants", "status": "processed"}),
        ),
    ]);
    let provider = create_openai(OpenAIProviderSettings {
        name: Some("proxy".into()),
        ..settings(&mock)
    })
    .unwrap();

    let image = provider.image("dall-e-3");
    assert_eq!(image.provider(), "proxy.image");
    image
        .do_generate(&ImageCallOptions::new("a cat".to_string()))
        .await
        .unwrap();

    let speech = provider.speech("tts-1");
    assert_eq!(speech.provider(), "proxy.speech");
    speech
        .do_generate(&SpeechCallOptions::new("hi".to_string()))
        .await
        .unwrap();

    let transcription = provider.transcription("whisper-1");
    assert_eq!(transcription.provider(), "proxy.transcription");
    transcription
        .do_generate(&TranscriptionCallOptions::new(
            AudioInput::Binary(vec![0; 8]),
            "audio/wav",
        ))
        .await
        .unwrap();

    let files = provider.files();
    assert_eq!(files.provider(), "proxy.files");
    files
        .upload_file(&UploadFileCallOptions {
            data: UploadFileData::Data {
                data: FileBytes::Binary(vec![1, 2, 3]),
            },
            media_type: "application/octet-stream".into(),
            filename: None,
            provider_options: None,
            abort_signal: None,
        })
        .await
        .unwrap();

    let seen = mock.seen();
    let urls: Vec<&str> = seen.iter().map(|s| s.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://api.openai.com/v1/images/generations",
            "https://api.openai.com/v1/audio/speech",
            "https://api.openai.com/v1/audio/transcriptions",
            "https://api.openai.com/v1/files",
        ]
    );
    for request in &seen {
        assert_eq!(request.method, "POST");
        assert_eq!(request.headers["authorization"], bearer(KEY));
    }
    assert!(seen[2].headers["content-type"].starts_with("multipart/form-data"));
    assert!(seen[3].headers["content-type"].starts_with("multipart/form-data"));
}

#[tokio::test]
async fn the_provider_trait_serves_the_native_models() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = create_openai(settings(&mock)).unwrap();
    let model = provider.language_model("gpt-4o").unwrap();
    assert_eq!(model.provider(), "openai.chat");
    assert_eq!(provider.call("gpt-4o").provider(), "openai.chat");
    assert_eq!(
        provider.embedding_model("e").unwrap().provider(),
        "openai.embedding"
    );
    assert_eq!(
        provider.image_model("i").unwrap().provider(),
        "openai.image"
    );
    assert!(provider.transcription_model("t").is_some());
    assert!(provider.speech_model("s").is_some());
    assert!(Provider::files(&provider).is_some());
    model.do_generate(&hello()).await.unwrap();
    assert_eq!(mock.seen().len(), 1);
}

// ── discovery ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_models_is_one_authenticated_get() {
    let mock = MockFetch::new(vec![Canned::json(
        &json!({"object": "list", "data": [{"id": "gpt-4o", "object": "model", "created": 1, "owned_by": "openai"}]}),
    )]);
    let provider = create_openai(settings(&mock)).unwrap();
    let models = provider.list_models().await.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "gpt-4o");
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, "https://api.openai.com/v1/models");
    assert_eq!(seen[0].headers["authorization"], bearer(KEY));
}

#[tokio::test]
async fn list_models_does_not_retry() {
    let failing = Canned {
        status: 503,
        headers: vec![("retry-after-ms".into(), "0".into())],
        body: br#"{"error":{"message":"busy"}}"#.to_vec(),
    };
    let mock = MockFetch::new(vec![failing.clone(), failing]);
    let provider = create_openai(settings(&mock)).unwrap();
    let error = provider.list_models().await.unwrap_err();
    assert!(matches!(error, AiMuxError::ApiCall(_)), "{error:?}");
    assert_eq!(mock.seen().len(), 1, "one exchange, no retry");
}

#[serial]
#[tokio::test]
async fn list_models_without_a_key_fails_before_sending() {
    let _env = EnvVar::set("OPENAI_API_KEY", None);
    let mock = MockFetch::new(vec![]);
    let provider = create_openai(OpenAIProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let error = provider.list_models().await.unwrap_err();
    assert!(matches!(error, AiMuxError::LoadApiKey { .. }), "{error:?}");
    assert!(mock.seen().is_empty());
}

// ── factory and default instance ─────────────────────────────────────────────

#[test]
fn the_default_instance_is_one_provider_and_creation_reads_nothing() {
    assert!(std::ptr::eq(openai(), openai()));
    assert_eq!(openai().chat("gpt-4o").provider(), "openai.chat");
    assert_eq!(openai().call("gpt-4o").provider(), "openai.chat");
}

#[test]
fn only_an_invalid_base_url_fails_creation() {
    for bad in [
        "",
        "gateway.example/v1",
        "ftp://gateway.example",
        "https://",
    ] {
        let result = create_openai(OpenAIProviderSettings {
            base_url: Some(bad.to_string()),
            ..Default::default()
        });
        assert!(
            matches!(result, Err(AiMuxError::InvalidArgument(_))),
            "{bad:?} must be rejected"
        );
    }
    assert!(
        create_openai(OpenAIProviderSettings {
            base_url: Some("http://127.0.0.1:8080/v1/".into()),
            name: Some("local".into()),
            organization: Some("o".into()),
            project: Some("p".into()),
            headers: Some(HeaderMapOpt::new()),
            ..Default::default()
        })
        .is_ok()
    );
}

#[test]
fn settings_debug_never_prints_secrets() {
    let printed = format!(
        "{:?}",
        OpenAIProviderSettings {
            api_key: Some(Resolvable::Value("sk-very-secret".into())),
            headers: Some(opt_map(&[("x-token", Some("tok-very-secret"))])),
            ..Default::default()
        }
    );
    assert!(!printed.contains("secret"), "{printed}");
}

fn _assert_provider_is_shareable() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<OpenAIProvider>();
}
