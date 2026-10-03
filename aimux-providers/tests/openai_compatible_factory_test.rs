//! `create_openai_compatible` against the recorded AI SDK behavior.
//!
//! `fixtures/aisdk/openai-compatible/*.json` hold what
//! `@ai-sdk/openai-compatible`'s `createOpenAICompatible` sent over a mocked
//! fetch for a fixed input: URL, headers, body and the model's `provider`
//! string. Each fixture is replayed here through an injected
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
//! (`Resolvable::Future` once, `AsyncFn` every time), no key means no
//! `Authorization` header, `Some("")` is sent as given, headers layer
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
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::result::{GenerateContent, GenerateResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool, ToolChoice};
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt, Resolvable,
};
use aimux_providers::openai_compatible::{
    OpenAICompatibleProvider, OpenAICompatibleProviderSettings, create_openai_compatible,
};

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

/// The settings every Groq-named fixture was recorded with
/// (`scripts/aisdk-fixtures/cases/openai-compatible.mjs`).
fn groq_settings(mock: &Arc<MockFetch>) -> OpenAICompatibleProviderSettings {
    OpenAICompatibleProviderSettings {
        name: "groq".to_string(),
        base_url: "https://api.groq.com/openai/v1".to_string(),
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
const FIXTURES: [&str; 7] = [
    "chat-basic",
    "chat-no-api-key",
    "chat-provider-options-generic-key-unknown",
    "chat-provider-options-namespace",
    "chat-query-params-and-headers",
    "chat-stream-basic",
    "embedding-basic",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/aisdk/openai-compatible")
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
    /// `authorization` is the value the redacted recording stands for, or
    /// `None` when the recording carries no `authorization` header at all.
    fn assert_request(&self, seen: &Seen, authorization: Option<&str>) {
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
        match authorization {
            Some(value) => {
                assert_eq!(
                    expected.get("authorization").map(String::as_str),
                    Some("<redacted>"),
                    "{}: the recording redacts the key",
                    self.name
                );
                expected.insert("authorization".to_string(), value.to_string());
            }
            None => assert!(
                !expected.contains_key("authorization"),
                "{}: the recording carries no authorization",
                self.name
            ),
        }
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
fn opt_map(entries: &[(&str, Option<&str>)]) -> HeaderMapOpt {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.map(str::to_string)))
        .collect()
}

fn chat_ok() -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    }))
}

fn hello() -> CallOptions {
    CallOptions::new(vec![message(Role::User, "hi")])
}

// ── the fixtures ─────────────────────────────────────────────────────────────

#[test]
fn every_fixture_is_replayed() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures/aisdk/openai-compatible exists")
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

/// Metadata is reported under the provider name, with the namespace entry
/// present even when empty.
fn assert_metadata_key(fixture: &Fixture, result: &GenerateResult, key: &str) {
    assert_eq!(
        fixture.json["result"]["providerMetadata"],
        json!({ key: {} }),
        "{}: recorded",
        fixture.name
    );
    assert_eq!(
        result.provider_metadata.as_ref().unwrap(),
        &json!({ key: {} }),
        "{}: metadata key",
        fixture.name
    );
}

/// Replay a `generateText` fixture recorded with the Groq-named provider.
async fn replay_groq_chat(name: &'static str) -> GenerateResult {
    let fixture = Fixture::load(name);
    let mock = fixture.mock();
    let provider = create_openai_compatible(groq_settings(&mock)).unwrap();
    let model = provider.chat(fixture.model_id());
    assert_eq!(model.provider(), "groq.chat");
    assert_eq!(
        model.provider(),
        fixture.provider_string(),
        "{name}: provider()"
    );
    assert_eq!(model.model_id(), fixture.model_id());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen.len(), 1, "{name}: exactly one request");
    fixture.assert_request(&seen[0], Some(&bearer(KEY)));
    assert_generate_result(&fixture, &result);
    assert_metadata_key(&fixture, &result, "groq");
    result
}

#[tokio::test]
async fn chat_basic() {
    replay_groq_chat("chat-basic").await;
}

#[tokio::test]
async fn chat_provider_options_namespace() {
    // `groq.foo` passes through at the top level; `openaiCompatible.user` is a
    // schema field.
    replay_groq_chat("chat-provider-options-namespace").await;
    let fixture = Fixture::load("chat-provider-options-namespace");
    assert_eq!(
        fixture.json["observations"],
        json!({"bodyHasFooAtTopLevel": true, "bodyUser": "u1", "providerIsGroqChat": true})
    );
}

#[tokio::test]
async fn unknown_fields_of_the_generic_namespace_are_not_forwarded() {
    replay_groq_chat("chat-provider-options-generic-key-unknown").await;
    let fixture = Fixture::load("chat-provider-options-generic-key-unknown");
    assert_eq!(
        fixture.json["observations"]["bodyHasExtraField"],
        json!(false)
    );
}

#[tokio::test]
async fn chat_query_params_and_headers() {
    let fixture = Fixture::load("chat-query-params-and-headers");
    let mock = fixture.mock();
    // The recording was made with headers { "x-a": "1", "X-B": "2" } and
    // queryParams { "api-version": "1" }.
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        headers: Some(opt_map(&[("x-a", Some("1")), ("X-B", Some("2"))])),
        query_params: Some(HashMap::from([(
            "api-version".to_string(),
            "1".to_string(),
        )])),
        ..groq_settings(&mock)
    })
    .unwrap();
    let model = provider.chat(fixture.model_id());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    let seen = mock.seen();
    fixture.assert_request(&seen[0], Some(&bearer(KEY)));
    assert_generate_result(&fixture, &result);
    // Case-insensitive override: the call's `x-b` replaced the provider's `X-B`.
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
async fn chat_without_an_api_key_sends_no_authorization() {
    let fixture = Fixture::load("chat-no-api-key");
    let mock = fixture.mock();
    // A key in the environment must not leak in either.
    let _env = EnvVar::set("OPENAI_API_KEY", Some(ENV_KEY));
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "local".to_string(),
        base_url: "http://localhost:1234/v1".to_string(),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("no key is needed to create the provider");
    let model = provider.chat(fixture.model_id());
    assert_eq!(model.provider(), "local.chat");
    assert_eq!(model.provider(), fixture.provider_string());

    let result = model.do_generate(&fixture.call_options()).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    fixture.assert_request(&seen[0], None);
    assert!(!seen[0].headers.contains_key("authorization"));
    assert_eq!(
        fixture.json["observations"]["hasAuthorizationHeader"],
        json!(false)
    );
    assert_generate_result(&fixture, &result);
    assert_metadata_key(&fixture, &result, "local");
}

#[tokio::test]
async fn chat_stream_basic() {
    let fixture = Fixture::load("chat-stream-basic");
    let mock = fixture.mock();
    let model = create_openai_compatible(groq_settings(&mock))
        .unwrap()
        .chat(fixture.model_id());
    assert_eq!(model.provider(), fixture.provider_string());

    let result = model.do_stream(&fixture.call_options()).await.unwrap();
    let parts: Vec<StreamPart> = result
        .stream
        .map(|part| part.expect("no stream error"))
        .collect()
        .await;

    // Unlike the native OpenAI package, the baseline asks for no
    // `stream_options` unless `include_usage` is set: the recorded body has
    // none. (The usage chunk is read anyway when the server sends it.)
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    fixture.assert_request(&seen[0], Some(&bearer(KEY)));
    assert!(seen[0].json_body().get("stream_options").is_none());

    let text: String = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello world");
    let (reason, usage, metadata) = parts
        .iter()
        .find_map(|part| match part {
            StreamPart::Finish {
                finish_reason,
                usage,
                provider_metadata,
            } => Some((finish_reason, usage, provider_metadata)),
            _ => None,
        })
        .expect("a finish part");
    assert_eq!(reason.unified, FinishReasonUnified::Stop);
    assert_eq!(usage.input_tokens.total, Some(12));
    assert_eq!(usage.output_tokens.total, Some(2));
    assert_eq!(metadata.as_ref().unwrap(), &json!({"groq": {}}));
}

#[tokio::test]
async fn embedding_basic() {
    let fixture = Fixture::load("embedding-basic");
    let mock = fixture.mock();
    let model = create_openai_compatible(groq_settings(&mock))
        .unwrap()
        .embedding(fixture.model_id());
    assert_eq!(model.provider(), "groq.embedding");
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

    fixture.assert_request(&mock.seen()[0], Some(&bearer(KEY)));
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

// ── identity and namespaces ──────────────────────────────────────────────────

#[test]
fn every_model_reports_name_dot_method() {
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "acme".to_string(),
        base_url: "https://api.acme.example/v1".to_string(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(provider.chat("m").provider(), "acme.chat");
    assert_eq!(provider.embedding("m").provider(), "acme.embedding");
    assert_eq!(provider.image("m").provider(), "acme.image");
    assert_eq!(provider.call("m").provider(), "acme.chat");
    assert_eq!(
        provider.language_model("m").unwrap().provider(),
        "acme.chat"
    );
    assert_eq!(
        provider.embedding_model("m").unwrap().provider(),
        "acme.embedding"
    );
    assert_eq!(provider.image_model("m").unwrap().provider(), "acme.image");
    assert!(provider.transcription_model("m").is_none());
    assert!(provider.speech_model("m").is_none());
}

fn options_with(provider_options: Value) -> CallOptions {
    let mut options = hello();
    options.provider_options = Some(
        provider_options
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    options
}

fn body_for(name: &str, options: &CallOptions) -> Value {
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        name: name.to_string(),
        base_url: "https://api.acme.example/v1".to_string(),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("m")
        .request_body(options, false)
        .unwrap()
        .body
}

#[test]
fn options_are_read_generic_then_name_then_camel_case_and_later_wins() {
    let body = body_for(
        "my_vendor",
        &options_with(json!({
            "openaiCompatible": {"user": "generic", "reasoningEffort": "low"},
            "my_vendor": {"user": "raw", "textVerbosity": "high"},
            "myVendor": {"user": "camel"},
        })),
    );
    assert_eq!(body["user"], "camel");
    assert_eq!(body["reasoning_effort"], "low");
    assert_eq!(body["verbosity"], "high");
}

#[test]
fn the_openai_namespace_is_not_read() {
    let body = body_for(
        "acme",
        &options_with(json!({"openai": {"user": "native", "foo": "bar"}})),
    );
    assert!(body.get("user").is_none());
    assert!(body.get("foo").is_none());
}

#[test]
fn own_namespace_fields_pass_through_after_sampling_and_before_messages() {
    let mut options = options_with(json!({"acme": {"top_secret_flag": true, "user": "u"}}));
    options.temperature = Some(0.5);
    let body = body_for("acme", &options);
    assert_eq!(body["top_secret_flag"], json!(true));
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "model",
            "user",
            "temperature",
            "top_secret_flag",
            "messages"
        ]
    );
}

#[tokio::test]
async fn metadata_is_reported_under_the_provider_name() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "acme".to_string(),
        base_url: "https://api.acme.example/v1".to_string(),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let result = provider.chat("m").do_generate(&hello()).await.unwrap();
    assert_eq!(result.provider_metadata.unwrap(), json!({"acme": {}}));
}

// ── tool calls, tool results, thought signatures ─────────────────────────────

fn sse_canned(events: &[&str]) -> Canned {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!("data: {event}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }
}

#[tokio::test]
async fn a_tool_call_without_an_id_gets_one_and_keeps_its_thought_signature() {
    let mock = MockFetch::new(vec![Canned::json(&json!({
        "id": "c", "model": "m",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": null,
            "tool_calls": [{
                "type": "function",
                "function": {"name": "weather", "arguments": "{\"city\":\"SF\"}"},
                "extra_content": {"google": {"thought_signature": "sig-1"}}
            }]
        }}]
    }))]);
    let provider = create_openai_compatible(groq_settings(&mock)).unwrap();
    let result = provider.chat("m").do_generate(&hello()).await.unwrap();
    // Usage is absent from the reply: the model still answers.
    assert_eq!(result.usage.input_tokens.total, None);
    let call = result
        .content
        .iter()
        .find_map(|part| match part {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                thought_signature,
                provider_metadata,
                ..
            } => Some((
                tool_call_id.clone(),
                tool_name.clone(),
                input.clone(),
                thought_signature.clone(),
                provider_metadata.clone(),
            )),
            _ => None,
        })
        .expect("a tool call");
    assert!(!call.0.is_empty(), "a generated id");
    assert_eq!(call.1, "weather");
    assert_eq!(call.2, "{\"city\":\"SF\"}");
    assert_eq!(call.3.as_deref(), Some("sig-1"));
    assert_eq!(
        call.4.unwrap(),
        json!({"groq": {"thoughtSignature": "sig-1"}})
    );
}

#[tokio::test]
async fn a_streamed_tool_call_whose_first_delta_has_no_name_is_buffered_until_it_has_one() {
    let mock = MockFetch::new(vec![sse_canned(&[
        r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"arguments":"{\"a\""}}]}}]}"#,
        r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"lookup","arguments":":1}"}}]}}]}"#,
        r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    ])]);
    let provider = create_openai_compatible(groq_settings(&mock)).unwrap();
    let result = provider.chat("m").do_stream(&hello()).await.unwrap();
    let parts: Vec<StreamPart> = result.stream.map(|p| p.unwrap()).collect().await;
    let call = parts
        .iter()
        .find_map(|part| match part {
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        })
        .expect("a finished tool call");
    assert_eq!(call.0, "call_1");
    assert_eq!(call.1, "lookup");
    assert_eq!(call.2, "{\"a\":1}");
}

fn tool_exchange(result: Value) -> CallOptions {
    use aimux_core::content::ContentPart;
    CallOptions::new(vec![
        message(Role::User, "weather?"),
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![
                ContentPart::reasoning("thinking"),
                ContentPart::tool_call("call_1", "weather", json!({"city": "SF"})),
            ],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::tool_result("call_1", result)],
            ..Default::default()
        },
    ])
}

fn compat(multi_part: bool) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "acme".to_string(),
        base_url: "https://api.acme.example/v1".to_string(),
        supports_multi_part_tool_content: Some(multi_part),
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn assistant_turns_replay_reasoning_and_tool_calls() {
    let body = compat(false)
        .chat("m")
        .request_body(&tool_exchange(json!("sunny")), false)
        .unwrap()
        .body;
    assert_eq!(
        body["messages"][1],
        json!({
            "role": "assistant",
            "content": null,
            "reasoning_content": "thinking",
            "tool_calls": [{
                "id": "call_1", "type": "function",
                "function": {"name": "weather", "arguments": "{\"city\":\"SF\"}"}
            }]
        })
    );
    assert_eq!(
        body["messages"][2],
        json!({"role": "tool", "tool_call_id": "call_1", "content": "sunny"})
    );
}

#[test]
fn a_thought_signature_is_echoed_on_the_tool_call() {
    use aimux_core::content::ContentPart;
    let mut options = tool_exchange(json!("sunny"));
    if let ContentPart::ToolCall {
        thought_signature, ..
    } = &mut options.prompt[1].content[1]
    {
        *thought_signature = Some("sig-1".to_string());
    }
    let body = compat(false)
        .chat("m")
        .request_body(&options, false)
        .unwrap()
        .body;
    assert_eq!(
        body["messages"][1]["tool_calls"][0]["extra_content"],
        json!({"google": {"thought_signature": "sig-1"}})
    );
}

#[test]
fn structured_tool_results_need_supports_multi_part_tool_content() {
    let parts = json!([{"type": "text", "text": "sunny"}]);
    let text = compat(false)
        .chat("m")
        .request_body(&tool_exchange(parts.clone()), false)
        .unwrap()
        .body;
    // Without the flag the result is serialized to text.
    let sent = text["messages"][2]["content"].as_str().expect("a string");
    assert_eq!(serde_json::from_str::<Value>(sent).unwrap(), parts);

    let structured = compat(true)
        .chat("m")
        .request_body(&tool_exchange(parts.clone()), false)
        .unwrap()
        .body;
    assert_eq!(structured["messages"][2]["content"], parts);
}

#[test]
fn message_level_provider_options_ride_the_wire_object() {
    use aimux_core::content::ContentPart;
    let mut message = LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("hi")],
        ..Default::default()
    };
    message.provider_options = Some(json!({
        "openaiCompatible": {"cache_control": {"type": "ephemeral"}},
        "acme": {"x_flag": true},
        "openai": {"ignored": 1}
    }));
    let body = compat(false)
        .chat("m")
        .request_body(&CallOptions::new(vec![message]), false)
        .unwrap()
        .body;
    assert_eq!(
        body["messages"][0],
        json!({"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}, "x_flag": true})
    );
}

#[test]
fn a_file_reference_is_resolved_under_the_provider_name() {
    use aimux_core::content::ContentPart;
    let body = compat(false)
        .chat("m")
        .request_body(
            &CallOptions::new(vec![LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::file_reference(
                    "application/pdf",
                    json!({"acme": "file-1"}),
                )],
                ..Default::default()
            }]),
            false,
        )
        .unwrap()
        .body;
    assert_eq!(
        body["messages"][0]["content"][0],
        json!({"type": "file", "file": {"file_id": "file-1"}})
    );
}

// ── key evaluation ───────────────────────────────────────────────────────────

fn keyed(mock: &Arc<MockFetch>, key: Option<Resolvable<String>>) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        api_key: key,
        ..groq_settings(mock)
    })
    .unwrap()
}

async fn authorizations_over_three_calls(api_key: Resolvable<String>) -> Vec<String> {
    let mock = MockFetch::new(vec![chat_ok(), chat_ok(), chat_ok()]);
    let model = keyed(&mock, Some(api_key)).chat("m");
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
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(sent, vec![bearer("key-0"); 3]);
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
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        sent,
        vec![bearer("key-0"), bearer("key-1"), bearer("key-2")]
    );
}

#[tokio::test]
async fn creating_the_provider_does_not_run_the_key_producer() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = keyed(
        &mock,
        Some(Resolvable::from_fn(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok("k".to_string())
        })),
    );
    let _model = provider.chat("m");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failing_key_producer_fails_the_call_and_sends_nothing() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = keyed(
        &mock,
        Some(Resolvable::from_fn(|| {
            Err(AiMuxError::Other("vault is sealed".into()))
        })),
    );
    let error = provider.chat("m").do_generate(&hello()).await.unwrap_err();
    assert!(error.to_string().contains("vault is sealed"), "{error}");
    assert!(mock.seen().is_empty());
}

#[serial]
#[tokio::test]
async fn an_explicit_empty_key_is_sent_as_given() {
    let _env = EnvVar::set("OPENAI_API_KEY", Some(ENV_KEY));
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = keyed(&mock, Some(Resolvable::Value(String::new())));
    provider.chat("m").do_generate(&hello()).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["authorization"].trim(), "Bearer");
    assert!(!seen[0].headers["authorization"].contains(ENV_KEY));
}

#[tokio::test]
async fn a_none_header_removes_the_authorization_header() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        headers: Some(opt_map(&[("Authorization", None)])),
        ..groq_settings(&mock)
    })
    .unwrap();
    provider.chat("m").do_generate(&hello()).await.unwrap();
    assert!(!mock.seen()[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn call_headers_win_over_provider_headers() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        headers: Some(opt_map(&[("AUTHORIZATION", Some("Bearer from-headers"))])),
        ..groq_settings(&mock)
    })
    .unwrap();
    let mut options = hello();
    options.headers = Some(HashMap::from([(
        "authorization".to_string(),
        "Bearer from-call".to_string(),
    )]));
    provider.chat("m").do_generate(&options).await.unwrap();
    assert_eq!(mock.seen()[0].headers["authorization"], "Bearer from-call");
}

// ── settings ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn include_usage_adds_stream_options_to_streaming_requests_only() {
    let sse = Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: b"data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".to_vec(),
    };
    let mock = MockFetch::new(vec![chat_ok(), sse]);
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        include_usage: Some(true),
        ..groq_settings(&mock)
    })
    .unwrap();
    let model = provider.chat("m");
    model.do_generate(&hello()).await.unwrap();
    let streamed = model.do_stream(&hello()).await.unwrap();
    let _: Vec<_> = streamed.stream.collect().await;
    let seen = mock.seen();
    assert!(seen[0].json_body().get("stream_options").is_none());
    assert_eq!(
        seen[1].json_body()["stream_options"],
        json!({"include_usage": true})
    );
}

fn schema_options() -> CallOptions {
    let mut options = hello();
    options.response_format = Some(aimux_core::options::ResponseFormat::Json {
        schema: Some(json!({"type": "object"})),
        name: Some("answer".to_string()),
        description: None,
    });
    options
}

#[test]
fn a_schema_needs_supports_structured_outputs_or_degrades_with_a_warning() {
    let provider = |flag: Option<bool>| {
        create_openai_compatible(OpenAICompatibleProviderSettings {
            name: "acme".to_string(),
            base_url: "https://api.acme.example/v1".to_string(),
            supports_structured_outputs: flag,
            ..Default::default()
        })
        .unwrap()
    };
    let off = provider(None)
        .chat("m")
        .request_body(&schema_options(), false)
        .unwrap();
    assert_eq!(off.body["response_format"], json!({"type": "json_object"}));
    assert_eq!(off.warnings.len(), 1);

    let on = provider(Some(true))
        .chat("m")
        .request_body(&schema_options(), false)
        .unwrap();
    assert_eq!(
        on.body["response_format"],
        json!({"type": "json_schema", "json_schema": {
            "schema": {"type": "object"}, "strict": true, "name": "answer"
        }})
    );
    assert!(on.warnings.is_empty());
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
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        transform_request_body: Some(Arc::new(move |mut body: Value| {
            counter.fetch_add(1, Ordering::SeqCst);
            body["x_tag"] = json!("t");
            body
        })),
        ..groq_settings(&mock)
    })
    .unwrap();
    provider.chat("m").do_generate(&hello()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    provider
        .embedding("e")
        .do_embed(&EmbeddingCallOptions::new("a"))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    for seen in mock.seen() {
        assert_eq!(seen.json_body()["x_tag"], "t");
    }
}

#[tokio::test]
async fn base_url_is_normalized_and_the_image_endpoint_uses_the_same_transport() {
    let mock = MockFetch::new(vec![
        chat_ok(),
        Canned::json(&json!({"created": 1, "data": [{"b64_json": "aGk="}]})),
    ]);
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        base_url: "https://gateway.example/openai/v1/".to_string(),
        ..groq_settings(&mock)
    })
    .unwrap();
    provider.chat("m").do_generate(&hello()).await.unwrap();
    let image = provider
        .image("img")
        .do_generate(&ImageCallOptions::new("a cat".to_string()))
        .await
        .unwrap();
    assert!(matches!(
        image.images,
        aimux_core::image_model::ImageOutputs::Base64(ref images) if images == &["aGk=".to_string()]
    ));
    let seen = mock.seen();
    assert_eq!(
        seen[0].url,
        "https://gateway.example/openai/v1/chat/completions"
    );
    assert_eq!(
        seen[1].url,
        "https://gateway.example/openai/v1/images/generations"
    );
    assert_eq!(seen[1].json_body()["response_format"], "b64_json");
    assert_eq!(seen[1].headers["authorization"], bearer(KEY));
}

// ── discovery ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_models_is_one_get_and_sends_no_authorization_without_a_key() {
    let mock = MockFetch::new(vec![Canned::json(
        &json!({"object": "list", "data": [{"id": "local-model", "object": "model", "created": 1, "owned_by": "me"}]}),
    )]);
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "local".to_string(),
        base_url: "http://localhost:1234/v1".to_string(),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let models = provider.list_models().await.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "local-model");
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, "http://localhost:1234/v1/models");
    assert!(!seen[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn list_models_does_not_retry() {
    let failing = Canned {
        status: 503,
        headers: vec![("retry-after-ms".into(), "0".into())],
        body: br#"{"error":{"message":"busy"}}"#.to_vec(),
    };
    let mock = MockFetch::new(vec![failing.clone(), failing]);
    let provider = keyed(&mock, Some(Resolvable::Value(KEY.to_string())));
    let error = provider.list_models().await.unwrap_err();
    assert!(matches!(error, AiMuxError::ApiCall(_)), "{error:?}");
    assert_eq!(mock.seen().len(), 1, "one exchange, no retry");
}

// ── factory ──────────────────────────────────────────────────────────────────

#[test]
fn the_factory_validates_name_and_base_url() {
    let make = |name: &str, base_url: &str| {
        create_openai_compatible(OpenAICompatibleProviderSettings {
            name: name.to_string(),
            base_url: base_url.to_string(),
            ..Default::default()
        })
    };
    for (name, base_url) in [
        ("", "https://a.example/v1"),
        ("  ", "https://a.example/v1"),
        ("a.b", "https://a.example/v1"),
        ("a", ""),
        ("a", "gateway.example/v1"),
        ("a", "ftp://gateway.example"),
        ("a", "https://"),
    ] {
        assert!(
            matches!(make(name, base_url), Err(AiMuxError::InvalidArgument(_))),
            "({name:?}, {base_url:?}) must be rejected"
        );
    }
    assert!(make("local", "http://127.0.0.1:8080/v1/").is_ok());
}

#[test]
fn settings_debug_never_prints_secrets() {
    let printed = format!(
        "{:?}",
        OpenAICompatibleProviderSettings {
            name: "x".into(),
            base_url: "https://x.example".into(),
            api_key: Some(Resolvable::Value("sk-very-secret".into())),
            headers: Some(opt_map(&[("x-token", Some("tok-very-secret"))])),
            ..Default::default()
        }
    );
    assert!(!printed.contains("secret"), "{printed}");
}

fn _assert_provider_is_shareable() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<OpenAICompatibleProvider>();
}
