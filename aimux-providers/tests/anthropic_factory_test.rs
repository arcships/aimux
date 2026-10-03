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
//! Two differences from the recording are deliberate and asserted around: the
//! `user-agent` header is the SDK's own identifier and aimux sends none, and
//! the recorded `x-api-key` is redacted, so the key the case used is
//! substituted back.
//!
//! The rest covers what a fixture cannot: credentials are evaluated per
//! request (`Resolvable::Future` once, `AsyncFn` every time), `auth_token`
//! replaces `x-api-key`, headers layer provider -> betas -> call with `None`
//! removing, providerOptions are read from `anthropic` and the custom name
//! (the custom one winning) and written back under the custom name, files and
//! discovery go through the same transport, and Claude Platform on AWS and
//! Anthropic on Vertex are the same model with different configuration.

use std::collections::{BTreeMap, VecDeque};
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
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::result::{GenerateContent, GenerateResult};
use aimux_core::shared::FileBytes;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool, ToolChoice};
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::{
    AwsCredentials, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt,
    Resolvable,
};
use aimux_providers::anthropic::{AnthropicProviderSettings, anthropic, create_anthropic};
use aimux_providers::anthropic_aws::{
    AnthropicAwsAuth, AnthropicAwsProviderSettings, anthropic_aws, create_anthropic_aws,
};
use aimux_providers::vertex::{VertexAuth, VertexProvider, VertexProviderConfig};

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

fn settings(mock: &Arc<MockFetch>) -> AnthropicProviderSettings {
    AnthropicProviderSettings {
        api_key: Some(Resolvable::Value(KEY.to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    }
}

/// A plain text Messages response, for the cases that do not replay a fixture.
fn text_response() -> Canned {
    Canned::json(&json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))
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
    /// (names case-insensitively, `user-agent` aside) and body.
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
        expected.remove("user-agent");
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

fn message(role: Role, text: &str) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content: vec![ContentPart::text(text)],
        ..Default::default()
    }
}

fn user_prompt(text: &str) -> CallOptions {
    CallOptions::new(vec![message(Role::User, text)])
}

/// Rebuild aimux call options from a recorded `sdk.input` (the subset the
/// Anthropic fixtures use).
fn call_options_from(input: &Value) -> CallOptions {
    let mut prompt: LanguageModelPrompt = Vec::new();
    if let Some(system) = input.get("system").and_then(Value::as_str) {
        prompt.push(message(Role::System, system));
    }
    if let Some(text) = input.get("prompt").and_then(Value::as_str) {
        prompt.push(message(Role::User, text));
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
        Some("required") => ToolChoice::Required,
        Some("none") => ToolChoice::None,
        Some("auto") | None => ToolChoice::Auto,
        Some(other) => panic!("unmapped toolChoice {other}"),
    };
    options.provider_options = input
        .get("providerOptions")
        .and_then(Value::as_object)
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
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
    assert_eq!(
        result.finish_reason.raw.as_deref(),
        recorded["rawFinishReason"].as_str(),
        "{}: raw finish",
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
            GenerateContent::Reasoning {
                text,
                provider_metadata,
            } => Some((text.clone(), provider_metadata.clone())),
            _ => None,
        })
        .expect("a reasoning part");
    assert_eq!(reasoning.0, "17 * 23 = 391.");
    assert_eq!(
        reasoning.1,
        Some(json!({ "anthropic": { "signature": "sig_fixture" } }))
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
            ..
        } => Some((finish_reason, usage)),
        _ => None,
    });
    let (reason, usage) = finish.expect("a finish part");
    assert_eq!(reason.unified, FinishReasonUnified::Stop);
    assert_eq!(reason.raw.as_deref(), Some("end_turn"));
    assert_eq!(usage.input_tokens.total, Some(12));
    assert_eq!(usage.output_tokens.total, Some(2));
}

// ── identity ─────────────────────────────────────────────────────────────────

#[test]
fn provider_strings_follow_the_name() {
    let default = create_anthropic(AnthropicProviderSettings::default()).unwrap();
    assert_eq!(default.messages("x").provider(), "anthropic.messages");
    assert_eq!(default.call("x").provider(), "anthropic.messages");
    assert_eq!(
        default.language_model("x").unwrap().provider(),
        "anthropic.messages"
    );
    assert_eq!(default.files().provider(), "anthropic.files");

    for (name, files) in [("proxy", "proxy.files"), ("proxy.messages", "proxy.files")] {
        let provider = create_anthropic(AnthropicProviderSettings {
            name: Some(name.to_string()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(provider.messages("x").provider(), name);
        assert_eq!(provider.files().provider(), files, "{name}");
    }
}

#[test]
fn the_default_instance_is_one_provider_and_creation_reads_nothing() {
    assert!(std::ptr::eq(anthropic(), anthropic()));
    assert_eq!(anthropic().messages("x").provider(), "anthropic.messages");
}

#[test]
fn other_modalities_are_not_offered() {
    let provider = create_anthropic(AnthropicProviderSettings::default()).unwrap();
    for (error, model_type) in [
        (
            provider.embedding_model("e").err().expect("no embeddings"),
            "embeddingModel",
        ),
        (
            provider.image_model("i").err().expect("no images"),
            "imageModel",
        ),
    ] {
        match error {
            AiMuxError::NoSuchModel { model_type: t, .. } => assert_eq!(t, model_type),
            other => panic!("expected NoSuchModel, got {other:?}"),
        }
    }
    assert!(Provider::files(&provider).is_some());
    assert!(provider.transcription_model("t").is_none());
}

// ── credentials ──────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn missing_api_key_fails_the_call_not_the_factory() {
    let _env = EnvVar::set("ANTHROPIC_API_KEY", None);
    let mock = MockFetch::new(vec![text_response()]);

    // Creating the provider and a model reads no key.
    let provider = create_anthropic(AnthropicProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("no key is read at creation");
    let model = provider.messages("claude-sonnet-4-5");

    let error = model
        .do_generate(&user_prompt("hi"))
        .await
        .expect_err("the call has no key");
    match &error {
        AiMuxError::LoadApiKey {
            env_var,
            description,
        } => {
            assert_eq!(env_var, "ANTHROPIC_API_KEY");
            assert_eq!(description, "Anthropic");
        }
        other => panic!("expected LoadApiKey, got {other:?}"),
    }
    assert!(mock.seen().is_empty(), "nothing was sent");
}

#[serial]
#[tokio::test]
async fn an_unset_key_is_read_from_the_environment_on_each_call() {
    let _env = EnvVar::set("ANTHROPIC_API_KEY", Some(ENV_KEY));
    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let model = create_anthropic(AnthropicProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .messages("claude-sonnet-4-5");

    model.do_generate(&user_prompt("one")).await.unwrap();
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "rotated") };
    model.do_generate(&user_prompt("two")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen[0].headers["x-api-key"], ENV_KEY);
    assert_eq!(seen[1].headers["x-api-key"], "rotated");
}

#[serial]
#[tokio::test]
async fn an_explicit_empty_key_is_sent_as_given() {
    // The environment holds a key; an explicit "" must not fall back to it.
    let _env = EnvVar::set("ANTHROPIC_API_KEY", Some(ENV_KEY));
    let mock = MockFetch::new(vec![text_response()]);
    let model = create_anthropic(AnthropicProviderSettings {
        api_key: Some(Resolvable::Value(String::new())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .messages("claude-sonnet-4-5");

    model.do_generate(&user_prompt("hi")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen[0].headers["x-api-key"], "");
    assert!(!seen[0].headers.values().any(|v| v.contains(ENV_KEY)));
}

#[serial]
#[tokio::test]
async fn an_auth_token_replaces_the_api_key_header() {
    let _env = EnvVar::set("ANTHROPIC_API_KEY", Some(ENV_KEY));
    let mock = MockFetch::new(vec![text_response()]);
    let model = create_anthropic(AnthropicProviderSettings {
        auth_token: Some(Resolvable::Value("tok".to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .messages("claude-sonnet-4-5");

    model.do_generate(&user_prompt("hi")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen[0].headers["authorization"], "Bearer tok");
    assert!(!seen[0].headers.contains_key("x-api-key"));
    assert_eq!(seen[0].headers["anthropic-version"], "2023-06-01");
}

#[test]
fn an_api_key_together_with_an_auth_token_is_rejected_at_creation() {
    let result = create_anthropic(AnthropicProviderSettings {
        api_key: Some(Resolvable::Value("k".to_string())),
        auth_token: Some(Resolvable::Value("t".to_string())),
        ..Default::default()
    });
    match result {
        Err(AiMuxError::InvalidArgument(message)) => assert_eq!(
            message,
            "Both apiKey and authToken were provided. Please use only one authentication method."
        ),
        Err(other) => panic!("expected InvalidArgument, got {other:?}"),
        Ok(_) => panic!("both credentials must be rejected"),
    }
}

#[tokio::test]
async fn a_future_key_is_awaited_once_and_an_async_fn_key_on_every_request() {
    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let once = Arc::new(AtomicUsize::new(0));
    let counter = once.clone();
    let model = create_anthropic(AnthropicProviderSettings {
        api_key: Some(Resolvable::from_future(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok("from-future".to_string())
        })),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .messages("claude-sonnet-4-5");
    model.do_generate(&user_prompt("a")).await.unwrap();
    model.do_generate(&user_prompt("b")).await.unwrap();
    assert_eq!(once.load(Ordering::SeqCst), 1);
    assert_eq!(mock.seen()[1].headers["x-api-key"], "from-future");

    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let model = create_anthropic(AnthropicProviderSettings {
        api_key: Some(Resolvable::from_async_fn(move || {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            async move { Ok(format!("key-{n}")) }
        })),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .messages("claude-sonnet-4-5");
    model.do_generate(&user_prompt("a")).await.unwrap();
    model.do_generate(&user_prompt("b")).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["x-api-key"], "key-0");
    assert_eq!(seen[1].headers["x-api-key"], "key-1");
}

// ── headers ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn headers_layer_provider_then_betas_then_call_and_none_removes() {
    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let provider_headers: HeaderMapOpt = [
        ("X-Org", Some("provider")),
        ("x-keep", Some("1")),
        ("Anthropic-Version", None),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
    .collect();
    let model = create_anthropic(AnthropicProviderSettings {
        headers: Some(provider_headers),
        ..settings(&mock)
    })
    .unwrap()
    .messages("claude-sonnet-4-5");

    let mut options = user_prompt("hi");
    options.headers = Some(
        [("x-org".to_string(), "call".to_string())]
            .into_iter()
            .collect(),
    );
    model.do_generate(&options).await.unwrap();
    let sent = &mock.seen()[0].headers;
    assert_eq!(
        sent["x-org"], "call",
        "call headers win, case-insensitively"
    );
    assert_eq!(sent["x-keep"], "1");
    assert!(
        !sent.contains_key("anthropic-version"),
        "a None value removes a fixed header"
    );
    assert_eq!(sent.keys().filter(|n| n.starts_with("x-org")).count(), 1);

    // Betas come from the request; a call header may still override them.
    let mut tools = user_prompt("hi");
    tools.tools = Some(vec![Tool::Function(FunctionTool::new(
        "t",
        json!({"type": "object", "properties": {}}),
    ))]);
    model.do_generate(&tools).await.unwrap();
    assert_eq!(
        mock.seen()[1].headers["anthropic-beta"],
        "structured-outputs-2025-11-13"
    );
}

// ── providerOptions namespace ────────────────────────────────────────────────

#[tokio::test]
async fn response_metadata_is_written_under_the_custom_name() {
    let thinking = json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "thinking", "thinking": "hm", "signature": "sig-1"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    });
    for (name, key) in [(None, "anthropic"), (Some("proxy"), "proxy")] {
        let mock = MockFetch::new(vec![Canned::json(&thinking)]);
        let model = create_anthropic(AnthropicProviderSettings {
            name: name.map(str::to_string),
            ..settings(&mock)
        })
        .unwrap()
        .messages("claude-sonnet-4-5");
        let result = model.do_generate(&user_prompt("hi")).await.unwrap();
        let metadata = result.content.iter().find_map(|part| match part {
            GenerateContent::Reasoning {
                provider_metadata, ..
            } => provider_metadata.clone(),
            _ => None,
        });
        assert_eq!(
            metadata,
            Some(json!({ key: { "signature": "sig-1" } })),
            "{name:?}"
        );
    }
}

#[tokio::test]
async fn a_custom_namespace_is_read_back_from_the_prompt() {
    let mock = MockFetch::new(vec![text_response()]);
    let model = create_anthropic(AnthropicProviderSettings {
        name: Some("proxy".to_string()),
        ..settings(&mock)
    })
    .unwrap()
    .messages("claude-sonnet-4-5");
    let mut options = CallOptions::new(vec![
        message(Role::User, "hi"),
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::Reasoning {
                text: "hm".to_string(),
                signature: None,
                provider_options: Some(json!({ "proxy": { "signature": "sig-1" } })),
            }],
            ..Default::default()
        },
        message(Role::User, "more"),
    ]);
    options.provider_options = Some(
        [(
            "proxy".to_string(),
            json!({ "thinking": { "type": "enabled", "budgetTokens": 2000 } }),
        )]
        .into_iter()
        .collect(),
    );

    model.do_generate(&options).await.unwrap();

    let body = mock.seen()[0].json_body();
    assert_eq!(body["thinking"]["budget_tokens"], json!(2000));
    assert_eq!(
        body["messages"][1]["content"][0],
        json!({ "type": "thinking", "thinking": "hm", "signature": "sig-1" })
    );
}

// ── base URL and body hook ───────────────────────────────────────────────────

#[tokio::test]
async fn the_base_url_is_used_as_given_and_the_bare_origin_means_v1() {
    for (base, url) in [
        (
            "https://api.anthropic.com",
            "https://api.anthropic.com/v1/messages",
        ),
        (
            "https://proxy.example/anthropic/v1/",
            "https://proxy.example/anthropic/v1/messages",
        ),
        ("https://proxy.example", "https://proxy.example/messages"),
    ] {
        let mock = MockFetch::new(vec![text_response()]);
        let model = create_anthropic(AnthropicProviderSettings {
            base_url: Some(base.to_string()),
            ..settings(&mock)
        })
        .unwrap()
        .messages("claude-sonnet-4-5");
        model.do_generate(&user_prompt("hi")).await.unwrap();
        assert_eq!(mock.seen()[0].url, url, "{base}");
    }
}

#[test]
fn only_an_invalid_base_url_fails_creation() {
    for bad in [
        "",
        "gateway.example/v1",
        "ftp://gateway.example",
        "https://",
    ] {
        let result = create_anthropic(AnthropicProviderSettings {
            base_url: Some(bad.to_string()),
            ..Default::default()
        });
        assert!(
            matches!(result, Err(AiMuxError::InvalidArgument(_))),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn transform_request_body_rewrites_what_is_sent_and_reported() {
    let mock = MockFetch::new(vec![text_response()]);
    let model = create_anthropic(AnthropicProviderSettings {
        transform_request_body: Some(Arc::new(|mut body| {
            body["metadata"] = json!({ "user_id": "rewritten" });
            body
        })),
        ..settings(&mock)
    })
    .unwrap()
    .messages("claude-sonnet-4-5");

    let result = model.do_generate(&user_prompt("hi")).await.unwrap();

    assert_eq!(
        mock.seen()[0].json_body()["metadata"],
        json!({ "user_id": "rewritten" })
    );
    assert_eq!(
        result.request_body.unwrap()["metadata"],
        json!({ "user_id": "rewritten" })
    );
}

// ── files and discovery ──────────────────────────────────────────────────────

fn upload() -> UploadFileCallOptions {
    UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Binary(vec![1, 2, 3]),
        },
        media_type: "application/pdf".into(),
        filename: Some("a.pdf".into()),
        provider_options: None,
        abort_signal: None,
    }
}

#[tokio::test]
async fn files_use_the_transport_the_beta_header_and_the_provider_name() {
    for (name, key, provider) in [
        (None, "anthropic", "anthropic.files"),
        (Some("proxy"), "proxy", "proxy.files"),
    ] {
        let mock = MockFetch::new(vec![Canned::json(&json!({
            "id": "file_1", "filename": "a.pdf", "mime_type": "application/pdf",
            "size_bytes": 3, "downloadable": false
        }))]);
        let files = create_anthropic(AnthropicProviderSettings {
            name: name.map(str::to_string),
            ..settings(&mock)
        })
        .unwrap()
        .files();
        assert_eq!(files.provider(), provider);

        let result = files.upload_file(&upload()).await.unwrap();

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, "https://api.anthropic.com/v1/files");
        assert_eq!(seen[0].headers["x-api-key"], KEY);
        assert_eq!(seen[0].headers["anthropic-beta"], "files-api-2025-04-14");
        // The reference is always keyed by the canonical name; metadata by the
        // provider's own.
        assert_eq!(result.provider_reference["anthropic"], "file_1");
        let metadata = result.provider_metadata.expect("metadata");
        assert!(metadata.contains_key(key), "{metadata:?}");
        assert_eq!(metadata.len(), 1);
    }
}

#[tokio::test]
async fn list_models_is_one_authenticated_get() {
    let mock = MockFetch::new(vec![Canned::json(
        &json!({"data": [{"id": "claude-sonnet-4-5", "display_name": "Claude Sonnet 4.5"}]}),
    )]);
    let provider = create_anthropic(settings(&mock)).unwrap();
    let models = provider.list_models().await.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "claude-sonnet-4-5");
    assert_eq!(models[0].owned_by.as_deref(), Some("Claude Sonnet 4.5"));
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, "https://api.anthropic.com/v1/models");
    assert_eq!(seen[0].headers["x-api-key"], KEY);
    assert_eq!(seen[0].headers["anthropic-version"], "2023-06-01");
}

#[tokio::test]
async fn list_models_does_not_retry() {
    let failing = Canned {
        status: 503,
        headers: vec![("retry-after-ms".into(), "0".into())],
        body: br#"{"type":"error","error":{"type":"api_error","message":"busy"}}"#.to_vec(),
    };
    let mock = MockFetch::new(vec![failing.clone(), failing]);
    let provider = create_anthropic(settings(&mock)).unwrap();
    let error = provider.list_models().await.unwrap_err();
    assert!(matches!(error, AiMuxError::ApiCall(_)), "{error:?}");
    assert_eq!(mock.seen().len(), 1, "one exchange, no retry");
}

#[serial]
#[tokio::test]
async fn list_models_without_a_key_fails_before_sending() {
    let _env = EnvVar::set("ANTHROPIC_API_KEY", None);
    let mock = MockFetch::new(vec![]);
    let provider = create_anthropic(AnthropicProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let error = provider.list_models().await.unwrap_err();
    assert!(matches!(error, AiMuxError::LoadApiKey { .. }), "{error:?}");
    assert!(mock.seen().is_empty());
}

// ── Claude Platform on AWS ───────────────────────────────────────────────────

fn credentials() -> AwsCredentials {
    AwsCredentials {
        access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
        secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        session_token: Some("session".to_string()),
        region: "eu-west-1".to_string(),
    }
}

#[tokio::test]
async fn aws_with_an_api_key_is_the_same_model_on_another_endpoint() {
    let mock = MockFetch::new(vec![text_response()]);
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
        region: Some("us-west-2".to_string()),
        auth: Some(AnthropicAwsAuth::ApiKey(Resolvable::Value(KEY.to_string()))),
        workspace_id: Some("ws_1".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.messages("claude-sonnet-4-5");
    assert_eq!(model.provider(), "anthropic-aws");

    model.do_generate(&user_prompt("hi")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(
        seen[0].url,
        "https://aws-external-anthropic.us-west-2.api.aws/v1/messages"
    );
    assert_eq!(seen[0].headers["x-api-key"], KEY);
    assert_eq!(seen[0].headers["anthropic-workspace-id"], "ws_1");
    assert_eq!(seen[0].headers["anthropic-version"], "2023-06-01");
    assert!(!seen[0].headers.contains_key("authorization"));
    assert_eq!(seen[0].json_body()["model"], json!("claude-sonnet-4-5"));
}

#[tokio::test]
async fn aws_sigv4_is_applied_by_the_transport_to_the_final_request() {
    let mock = MockFetch::new(vec![text_response()]);
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
        auth: Some(AnthropicAwsAuth::SigV4(Resolvable::Value(credentials()))),
        fetch: Some(mock.transport()),
        // The rewrite runs before the signature is computed.
        transform_request_body: Some(Arc::new(|mut body| {
            body["metadata"] = json!({ "user_id": "signed" });
            body
        })),
        ..Default::default()
    })
    .unwrap();
    provider
        .messages("claude-sonnet-4-5")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap();

    let seen = mock.seen();
    // The region comes from the credentials when the settings name none.
    assert_eq!(
        seen[0].url,
        "https://aws-external-anthropic.eu-west-1.api.aws/v1/messages"
    );
    assert!(!seen[0].headers.contains_key("x-api-key"));
    let authorization = &seen[0].headers["authorization"];
    assert!(
        authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"),
        "{authorization}"
    );
    assert!(authorization.contains("/eu-west-1/aws-external-anthropic/aws4_request"));
    assert_eq!(seen[0].headers["x-amz-security-token"], "session");
    // The signed payload hash is that of the bytes that were sent.
    use sha2::{Digest, Sha256};
    assert_eq!(
        seen[0].headers["x-amz-content-sha256"],
        hex::encode(Sha256::digest(&seen[0].body))
    );
    assert_eq!(seen[0].json_body()["metadata"]["user_id"], json!("signed"));
}

#[tokio::test]
async fn aws_sigv4_signs_discovery_too() {
    let mock = MockFetch::new(vec![Canned::json(&json!({"data": []}))]);
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
        auth: Some(AnthropicAwsAuth::SigV4(Resolvable::Value(credentials()))),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    provider.list_models().await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].method, "GET");
    assert!(seen[0].headers["authorization"].starts_with("AWS4-HMAC-SHA256"));
}

#[serial]
#[tokio::test]
async fn aws_without_credentials_reads_its_key_from_the_environment_per_call() {
    let _env = EnvVar::set("ANTHROPIC_AWS_API_KEY", None);
    let mock = MockFetch::new(vec![text_response()]);
    let provider = create_anthropic_aws(AnthropicAwsProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("creation reads nothing");
    let error = provider
        .messages("claude-sonnet-4-5")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap_err();
    match error {
        AiMuxError::LoadApiKey { env_var, .. } => assert_eq!(env_var, "ANTHROPIC_AWS_API_KEY"),
        other => panic!("expected LoadApiKey, got {other:?}"),
    }
    assert!(mock.seen().is_empty());
    assert!(std::ptr::eq(anthropic_aws(), anthropic_aws()));
}

#[test]
fn aws_rejects_a_region_that_would_change_the_host() {
    let result = create_anthropic_aws(AnthropicAwsProviderSettings {
        region: Some("evil.example/".to_string()),
        ..Default::default()
    });
    assert!(matches!(result, Err(AiMuxError::InvalidArgument(_))));
}

// ── Anthropic on Vertex ──────────────────────────────────────────────────────

#[tokio::test]
async fn vertex_is_the_same_model_with_a_different_envelope() {
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(
            "/projects/p/locations/l/publishers/anthropic/models/claude-sonnet-4-5:rawPredict",
        ))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_response_value()))
        .mount(&mock_server)
        .await;
    let model = VertexProvider::new(VertexProviderConfig {
        base_url: format!(
            "{}/projects/p/locations/l/publishers/google",
            mock_server.uri()
        ),
        project: Some("p".to_string()),
        location: Some("l".to_string()),
        auth: VertexAuth::BearerToken("vertex-token".to_string()),
        api_key_source: None,
    })
    .anthropic_model("claude-sonnet-4-5")
    .unwrap();
    assert_eq!(model.provider(), "googleVertex.anthropic.messages");

    let mut options = user_prompt("hi");
    options.tools = Some(vec![Tool::Function(
        FunctionTool::new("t", json!({"type": "object", "properties": {}})).with_strict(true),
    )]);
    let result = model.do_generate(&options).await.unwrap();

    let requests = mock_server.received_requests().await.unwrap();
    let request = &requests[0];
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert!(body.get("model").is_none(), "the URL carries the model");
    assert_eq!(body["anthropic_version"], json!("vertex-2023-10-16"));
    assert!(
        body["tools"][0].get("strict").is_none(),
        "Vertex rejects strict tool definitions"
    );
    let header = |name: &str| {
        request
            .headers
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    };
    assert_eq!(
        header("authorization").as_deref(),
        Some("Bearer vertex-token")
    );
    assert_eq!(
        header("anthropic-version"),
        None,
        "the version is in the body"
    );
    assert_eq!(
        header("anthropic-beta"),
        None,
        "no structured-outputs beta on Vertex"
    );
    assert_eq!(result.request_body.unwrap(), body);
}

fn text_response_value() -> Value {
    serde_json::from_slice(&text_response().body).unwrap()
}
