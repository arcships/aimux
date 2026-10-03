//! `create_google` against the recorded AI SDK behavior.
//!
//! `fixtures/aisdk/google/*.json` hold what `@ai-sdk/google`'s `createGoogle`
//! sent over a mocked fetch for a fixed input: URL, headers, body and the
//! model's `provider` string. Each fixture is replayed here through an
//! injected [`Fetch`] (the factory's `fetch` setting), so the test also proves
//! the transport is the one the settings name. The expected request comes from
//! the fixture; the inputs are rebuilt from the fixture's recorded `sdk.input`.
//!
//! Two differences from the recording are deliberate and asserted around: the
//! `user-agent` header is the SDK's own identifier and aimux sends none, and
//! the recorded `x-goog-api-key` is redacted, so the key the case used is
//! substituted back.
//!
//! The rest covers what a fixture cannot: the key is evaluated per request
//! (`Resolvable::Future` once, `AsyncFn` every time), provider and per-call
//! headers layer with `None` removing, the provider name drives every
//! modality's `provider()` string, files and discovery go through the same
//! transport, and the supported-URL patterns follow upstream.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::StreamExt;
use serde_json::{Value, json};
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::image_model::ImageModel;
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
use aimux_core::video_model::VideoModel;
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use aimux_providers::google::{GoogleProviderSettings, create_google, google};

use mock_fetch::{Canned, EnvVar, MockFetch, Seen};

const KEY: &str = "sk-test-fixture";
const ENV_KEY: &str = "sk-env-should-not-be-used";

fn settings(mock: &Arc<MockFetch>) -> GoogleProviderSettings {
    GoogleProviderSettings {
        api_key: Some(Resolvable::Value(KEY.to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    }
}

/// A plain text `generateContent` response, for the cases that do not replay a
/// fixture.
fn text_response() -> Canned {
    Canned::json(&json!({
        "candidates": [{
            "content": { "parts": [{ "text": "ok" }], "role": "model" },
            "finishReason": "STOP", "index": 0
        }],
        "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2 }
    }))
}

// ── fixtures ─────────────────────────────────────────────────────────────────

/// Every fixture under `fixtures/aisdk/google/`; the directory listing is
/// compared to this so a new fixture cannot go unreplayed.
const FIXTURES: [&str; 5] = [
    "embedding-basic",
    "generate-basic",
    "generate-provider-options",
    "generate-stream-basic",
    "generate-tools",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/aisdk/google")
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
            expected.get("x-goog-api-key").map(String::as_str),
            Some("<redacted>"),
            "{}: the recording redacts the key",
            self.name
        );
        expected.insert("x-goog-api-key".to_string(), KEY.to_string());
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
/// Google fixtures use).
fn call_options_from(input: &Value) -> CallOptions {
    let mut prompt: LanguageModelPrompt = Vec::new();
    if let Some(system) = input.get("system").and_then(Value::as_str) {
        prompt.push(message(Role::System, system));
    }
    if let Some(text) = input.get("prompt").and_then(Value::as_str) {
        prompt.push(message(Role::User, text));
    }
    let mut options = CallOptions::new(prompt);
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

fn unified_finish(recorded: &str) -> FinishReasonUnified {
    match recorded {
        "stop" => FinishReasonUnified::Stop,
        "tool-calls" => FinishReasonUnified::ToolCalls,
        other => panic!("unmapped finish reason {other}"),
    }
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
    assert_eq!(
        result.finish_reason.unified,
        unified_finish(recorded["finishReason"].as_str().unwrap()),
        "{}: finish",
        fixture.name
    );
    assert_eq!(
        result.finish_reason.raw.as_deref(),
        recorded["rawFinishReason"].as_str(),
        "{}: raw finish",
        fixture.name
    );
    // The usage metadata comes back under the Gemini namespace.
    assert_eq!(
        result.provider_metadata.as_ref().unwrap()["google"]["usageMetadata"],
        recorded["providerMetadata"]["google"]["usageMetadata"],
        "{}: usage metadata",
        fixture.name
    );
}

// ── the fixtures ─────────────────────────────────────────────────────────────

#[test]
fn every_fixture_is_replayed() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures/aisdk/google exists")
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
    let provider = create_google(settings(&mock)).unwrap();
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
    fixture.assert_request(&seen[0]);
    assert_generate_result(&fixture, &result);
    (fixture, mock, result)
}

#[tokio::test]
async fn generate_basic() {
    replay_generate("generate-basic").await;
}

#[tokio::test]
async fn generate_provider_options_are_read_from_the_google_namespace() {
    let (fixture, _, _) = replay_generate("generate-provider-options").await;
    // The recorded body carries both option groups the case set.
    let body = &fixture.json["request"]["body"];
    assert_eq!(
        body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        json!(1024)
    );
    assert_eq!(
        body["safetySettings"][0]["threshold"],
        json!("BLOCK_ONLY_HIGH")
    );
}

#[tokio::test]
async fn generate_tools() {
    let (fixture, _, result) = replay_generate("generate-tools").await;
    let recorded = &fixture.json["result"]["content"][0];
    let call = result
        .content
        .iter()
        .find_map(|part| match part {
            GenerateContent::ToolCall {
                tool_name, input, ..
            } => Some((tool_name.clone(), input.clone())),
            _ => None,
        })
        .expect("a tool call");
    assert_eq!(call.0, recorded["toolName"].as_str().unwrap());
    assert_eq!(
        serde_json::from_str::<Value>(&call.1).unwrap(),
        recorded["input"]
    );
}

#[tokio::test]
async fn generate_stream_basic() {
    let fixture = Fixture::load("generate-stream-basic");
    let mock = fixture.mock();
    let model = create_google(settings(&mock))
        .unwrap()
        .chat(fixture.model_id());
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
    let recorded = &fixture.json["result"];
    let finish = parts.iter().find_map(|part| match part {
        StreamPart::Finish {
            finish_reason,
            usage,
            provider_metadata,
        } => Some((finish_reason, usage, provider_metadata)),
        _ => None,
    });
    let (reason, usage, metadata) = finish.expect("a finish part");
    assert_eq!(
        reason.unified,
        unified_finish(recorded["finishReason"].as_str().unwrap())
    );
    assert_eq!(reason.raw.as_deref(), Some("STOP"));
    assert_eq!(
        usage.input_tokens.total,
        recorded["usage"]["inputTokens"].as_u64().map(|v| v as _)
    );
    assert_eq!(
        usage.output_tokens.total,
        recorded["usage"]["outputTokens"].as_u64().map(|v| v as _)
    );
    assert_eq!(
        metadata.as_ref().unwrap()["google"]["usageMetadata"],
        recorded["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["type"] == "finish-step")
            .unwrap()["providerMetadata"]["google"]["usageMetadata"]
    );
}

#[tokio::test]
async fn embedding_basic() {
    let fixture = Fixture::load("embedding-basic");
    let mock = fixture.mock();
    let model = create_google(settings(&mock))
        .unwrap()
        .embedding(fixture.model_id());
    assert_eq!(model.provider(), fixture.provider_string());

    let values: Vec<String> = fixture.sdk_input()["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let result = model
        .do_embed(&EmbeddingCallOptions {
            values,
            abort_signal: None,
            max_retries: None,
            timeout: None,
            provider_options: None,
            headers: None,
        })
        .await
        .unwrap();

    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    fixture.assert_request(&seen[0]);
    let recorded: Vec<Vec<f32>> = fixture.json["result"]["embeddings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap() as f32)
                .collect()
        })
        .collect();
    assert_eq!(result.embeddings, recorded);
}

// ── identity ─────────────────────────────────────────────────────────────────

#[test]
fn provider_strings_follow_the_name() {
    let default = create_google(GoogleProviderSettings::default()).unwrap();
    assert_eq!(default.chat("x").provider(), "google.generative-ai");
    assert_eq!(default.call("x").provider(), "google.generative-ai");
    assert_eq!(
        default.language_model("x").unwrap().provider(),
        "google.generative-ai"
    );
    assert_eq!(default.embedding("x").provider(), "google.generative-ai");
    assert_eq!(default.image("x").provider(), "google.generative-ai");
    assert_eq!(default.video("x").provider(), "google.generative-ai.video");
    assert_eq!(default.files().provider(), "google.generative-ai.files");

    let proxy = create_google(GoogleProviderSettings {
        name: Some("proxy".to_string()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(proxy.chat("x").provider(), "proxy");
    assert_eq!(proxy.embedding("x").provider(), "proxy");
    assert_eq!(proxy.image("x").provider(), "proxy");
    assert_eq!(proxy.video("x").provider(), "proxy.video");
    assert_eq!(proxy.files().provider(), "proxy.files");
}

#[test]
fn the_default_instance_is_one_provider_and_creation_reads_nothing() {
    assert!(std::ptr::eq(google(), google()));
    assert_eq!(google().chat("x").provider(), "google.generative-ai");
}

#[test]
fn the_provider_offers_language_embedding_image_video_and_files() {
    let provider = create_google(GoogleProviderSettings::default()).unwrap();
    assert!(provider.language_model("m").is_ok());
    assert!(provider.embedding_model("m").is_ok());
    assert!(provider.image_model("m").is_ok());
    assert!(provider.video_model("m").unwrap().is_ok());
    assert!(Provider::files(&provider).is_some());
    assert!(provider.speech_model("s").is_none());
    assert!(provider.reranking_model("r").is_none());
}

#[test]
fn a_bad_base_url_fails_the_factory() {
    let result = create_google(GoogleProviderSettings {
        base_url: Some("not a url".to_string()),
        ..Default::default()
    });
    assert!(matches!(result, Err(AiMuxError::InvalidArgument(_))));
}

// ── namespaces ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn only_the_google_key_is_read_and_written() {
    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let model = create_google(settings(&mock))
        .unwrap()
        .chat("gemini-2.5-flash");

    // Options under `googleVertex` / `vertex` belong to other packages.
    let mut foreign = user_prompt("hi");
    foreign.provider_options = Some(
        [
            ("googleVertex".to_string(), json!({ "cachedContent": "v" })),
            ("vertex".to_string(), json!({ "labels": { "a": "b" } })),
        ]
        .into_iter()
        .collect(),
    );
    let result = model.do_generate(&foreign).await.unwrap();
    let body = mock.seen()[0].json_body();
    assert!(body.get("cachedContent").is_none() && body.get("labels").is_none());
    let metadata = result.provider_metadata.unwrap();
    assert!(metadata.get("google").is_some());
    assert!(metadata.get("googleVertex").is_none() && metadata.get("vertex").is_none());

    let mut own = user_prompt("hi");
    own.provider_options = Some(
        [("google".to_string(), json!({ "cachedContent": "g" }))]
            .into_iter()
            .collect(),
    );
    model.do_generate(&own).await.unwrap();
    assert_eq!(mock.seen()[1].json_body()["cachedContent"], "g");
}

// ── credentials ──────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn missing_api_key_fails_the_call_not_the_factory() {
    let _env = EnvVar::set("GOOGLE_GENERATIVE_AI_API_KEY", None);
    let mock = MockFetch::new(vec![text_response()]);

    // Creating the provider and a model reads no key.
    let provider = create_google(GoogleProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("no key is read at creation");
    let model = provider.chat("gemini-2.5-flash");

    let error = model
        .do_generate(&user_prompt("hi"))
        .await
        .expect_err("the call has no key");
    match &error {
        AiMuxError::LoadApiKey {
            env_var,
            description,
        } => {
            assert_eq!(env_var, "GOOGLE_GENERATIVE_AI_API_KEY");
            assert_eq!(description, "Google Generative AI");
            assert!(!error.to_string().contains(ENV_KEY));
        }
        other => panic!("expected LoadApiKey, got {other:?}"),
    }
    assert!(mock.seen().is_empty(), "nothing was sent");
}

#[serial]
#[tokio::test]
async fn an_unset_key_is_read_from_the_environment_on_each_call() {
    let _env = EnvVar::set("GOOGLE_GENERATIVE_AI_API_KEY", Some(ENV_KEY));
    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let model = create_google(GoogleProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash");

    model.do_generate(&user_prompt("one")).await.unwrap();
    unsafe { std::env::set_var("GOOGLE_GENERATIVE_AI_API_KEY", "rotated") };
    model.do_generate(&user_prompt("two")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen[0].headers["x-goog-api-key"], ENV_KEY);
    assert_eq!(seen[1].headers["x-goog-api-key"], "rotated");
}

#[serial]
#[tokio::test]
async fn an_explicit_empty_key_never_falls_back_to_the_environment() {
    let _env = EnvVar::set("GOOGLE_GENERATIVE_AI_API_KEY", Some(ENV_KEY));
    let mock = MockFetch::new(vec![text_response()]);
    let model = create_google(GoogleProviderSettings {
        api_key: Some(Resolvable::Value(String::new())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash");

    model.do_generate(&user_prompt("hi")).await.unwrap();
    assert_eq!(mock.seen()[0].headers["x-goog-api-key"], "");
}

#[tokio::test]
async fn async_keys_are_awaited_per_call_and_futures_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let async_key = Resolvable::from_async_fn(move || {
        let counted = counted.clone();
        async move { Ok(format!("key-{}", counted.fetch_add(1, Ordering::SeqCst))) }
    });
    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let model = create_google(GoogleProviderSettings {
        api_key: Some(async_key),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "creation evaluates nothing"
    );
    model.do_generate(&user_prompt("one")).await.unwrap();
    model.do_generate(&user_prompt("two")).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["x-goog-api-key"], "key-0");
    assert_eq!(seen[1].headers["x-goog-api-key"], "key-1");

    let futures_calls = Arc::new(AtomicUsize::new(0));
    let counted = futures_calls.clone();
    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let model = create_google(GoogleProviderSettings {
        api_key: Some(Resolvable::from_future(async move {
            Ok(format!("once-{}", counted.fetch_add(1, Ordering::SeqCst)))
        })),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash");
    model.do_generate(&user_prompt("one")).await.unwrap();
    model.do_generate(&user_prompt("two")).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["x-goog-api-key"], "once-0");
    assert_eq!(seen[1].headers["x-goog-api-key"], "once-0");
    assert_eq!(futures_calls.load(Ordering::SeqCst), 1);
}

// ── headers ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn headers_layer_provider_then_call_and_none_removes() {
    let mock = MockFetch::new(vec![text_response(), text_response()]);
    let mut provider_headers = HeaderMapOpt::new();
    provider_headers.insert("X-Team".to_string(), Some("blue".to_string()));
    provider_headers.insert("X-Trace".to_string(), Some("provider".to_string()));
    let model = create_google(GoogleProviderSettings {
        headers: Some(provider_headers),
        ..settings(&mock)
    })
    .unwrap()
    .chat("gemini-2.5-flash");

    let mut options = user_prompt("hi");
    options.headers = Some(
        [("x-trace".to_string(), "call".to_string())]
            .into_iter()
            .collect(),
    );
    model.do_generate(&options).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["x-team"], "blue");
    assert_eq!(seen[0].headers["x-trace"], "call", "per-call wins");
    assert_eq!(seen[0].headers["x-goog-api-key"], KEY);

    // A `None` provider header removes even the key header.
    let mock = MockFetch::new(vec![text_response()]);
    let mut remove = HeaderMapOpt::new();
    remove.insert("X-Goog-Api-Key".to_string(), None);
    create_google(GoogleProviderSettings {
        headers: Some(remove),
        ..settings(&mock)
    })
    .unwrap()
    .chat("gemini-2.5-flash")
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    assert!(!mock.seen()[0].headers.contains_key("x-goog-api-key"));
}

// ── base URL, transform, supported URLs ──────────────────────────────────────

#[tokio::test]
async fn base_url_replaces_the_default_and_loses_its_trailing_slash() {
    let mock = MockFetch::new(vec![text_response()]);
    create_google(GoogleProviderSettings {
        base_url: Some("https://proxy.example/v1beta/".to_string()),
        ..settings(&mock)
    })
    .unwrap()
    .chat("gemini-2.5-flash")
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    assert_eq!(
        mock.seen()[0].url,
        "https://proxy.example/v1beta/models/gemini-2.5-flash:generateContent"
    );
}

#[tokio::test]
async fn transform_request_body_rewrites_every_json_body() {
    let mock = MockFetch::new(vec![text_response()]);
    create_google(GoogleProviderSettings {
        transform_request_body: Some(Arc::new(|mut body| {
            body["labels"] = json!({ "team": "blue" });
            body
        })),
        ..settings(&mock)
    })
    .unwrap()
    .chat("gemini-2.5-flash")
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    assert_eq!(
        mock.seen()[0].json_body()["labels"],
        json!({ "team": "blue" })
    );
}

#[test]
fn supported_urls_follow_upstream() {
    let provider = create_google(GoogleProviderSettings {
        base_url: Some("https://proxy.example/v1beta".to_string()),
        ..Default::default()
    })
    .unwrap();

    let gemini = provider.chat("gemini-2.5-flash").supported_urls().0;
    let any = &gemini["*"];
    for url in [
        "https://generativelanguage.googleapis.com/v1beta/files/abc",
        "https://proxy.example/v1beta/files/abc",
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "https://youtu.be/dQw4w9WgXcQ",
    ] {
        assert!(any.iter().any(|re| re.is_match(url)), "{url}");
    }
    assert!(
        !any.iter()
            .any(|re| re.is_match("https://example.com/a.png"))
    );
    // External https media URLs are fetched by Gemini models after 2.0.
    assert!(gemini["image/png"][0].is_match("https://example.com/a.png"));
    assert!(!gemini["image/png"][0].is_match("http://example.com/a.png"));
    assert!(gemini.contains_key("application/pdf"));

    let old = provider.chat("gemini-2.0-flash").supported_urls().0;
    assert!(old.contains_key("*") && !old.contains_key("image/png"));
}

// ── files and discovery share the transport ──────────────────────────────────

#[tokio::test]
async fn list_models_is_one_exchange_through_the_same_transport() {
    let mock = MockFetch::new(vec![Canned::json(&json!({
        "models": [
            { "name": "models/gemini-2.5-flash", "displayName": "Gemini 2.5 Flash" },
            { "name": "models/gemini-2.5-pro" }
        ]
    }))]);
    let provider = create_google(settings(&mock)).unwrap();
    let models = provider.list_models().await.unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["gemini-2.5-flash", "gemini-2.5-pro"]
    );
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(
        seen[0].url,
        "https://generativelanguage.googleapis.com/v1beta/models"
    );
    assert_eq!(seen[0].headers["x-goog-api-key"], KEY);
}

#[tokio::test]
async fn a_failing_list_models_is_not_retried() {
    let mock = MockFetch::new(vec![Canned {
        status: 503,
        headers: vec![("content-type".into(), "application/json".into())],
        body: br#"{"error":{"message":"busy","status":"UNAVAILABLE"}}"#.to_vec(),
    }]);
    let provider = create_google(settings(&mock)).unwrap();
    assert!(provider.list_models().await.is_err());
    assert_eq!(mock.seen().len(), 1, "discovery runs once");
}

#[tokio::test]
async fn a_file_upload_is_one_attempt_per_stage() {
    // The init exchange fails with a retryable status: it must not be replayed.
    let mock = MockFetch::new(vec![
        Canned {
            status: 503,
            headers: vec![("content-type".into(), "application/json".into())],
            body: br#"{"error":{"message":"busy","status":"UNAVAILABLE"}}"#.to_vec(),
        },
        text_response(),
    ]);
    let files = create_google(settings(&mock)).unwrap().files();
    let error = files
        .upload_file(&UploadFileCallOptions {
            data: UploadFileData::Data {
                data: FileBytes::Binary(vec![1, 2, 3]),
            },
            media_type: "application/octet-stream".to_string(),
            filename: None,
            provider_options: None,
            abort_signal: None,
        })
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Failed to initiate resumable upload"),
        "{error}"
    );
    let seen = mock.seen();
    assert_eq!(seen.len(), 1, "no retry");
    assert_eq!(
        seen[0].url,
        "https://generativelanguage.googleapis.com/upload/v1beta/files"
    );
    assert_eq!(seen[0].headers["x-goog-api-key"], KEY);
}

// The image request shapes are covered by `google_image_test.rs`; this only
// pins the trait route and the provider string.
#[test]
fn image_models_are_offered_through_the_trait() {
    let provider = create_google(GoogleProviderSettings::default()).unwrap();
    let model = provider.image_model("imagen-3.0-generate-002").unwrap();
    assert_eq!(model.provider(), "google.generative-ai");
}
