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
//! The recorded `authorization` is redacted, so the key the case used is
//! substituted back. The recorded SDK and runtime user-agent identifiers are
//! replaced with the pinned provider package's user-agent suffix.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

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
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt,
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
        api_key: Some(KEY.to_string()),
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
    /// (names case-insensitively, with the pinned user-agent) and body.
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
        expected.insert(
            "user-agent".to_string(),
            "ai-sdk-openai-compatible/3.0.59 ai-sdk-provider-utils/5.0.51".to_string(),
        );
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
        None => options
            .tools
            .as_ref()
            .filter(|tools| !tools.is_empty())
            .map(|_| ToolChoice::Auto),
        Some(other) => panic!("unmapped toolChoice {other}"),
    };
    options.provider_options = input
        .get("providerOptions")
        .map(|o| serde_json::from_value(o.clone()).unwrap());
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
fn opt_map(entries: &[(&str, Option<&str>)]) -> HeaderMapOpt {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.map(str::to_string)))
        .collect()
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
        serde_json::to_value(result.provider_metadata.as_ref().unwrap()).unwrap(),
        json!({ key: {} }),
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
    assert_eq!(
        serde_json::to_value(metadata.as_ref().unwrap()).unwrap(),
        json!({"groq": {}})
    );
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

fn _assert_provider_is_shareable() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<OpenAICompatibleProvider>();
}
