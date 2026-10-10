//! `create_google` against the recorded AI SDK behavior.
//!
//! `fixtures/aisdk/google/*.json` hold what `@ai-sdk/google`'s `createGoogle`
//! sent over a mocked fetch for a fixed input: URL, headers, body and the
//! model's `provider` string. Each fixture is replayed here through an
//! injected [`Fetch`] (the factory's `fetch` setting), so the test also proves
//! the transport is the one the settings name. The expected request comes from
//! the fixture; the inputs are rebuilt from the fixture's recorded `sdk.input`.
//!
//! The recorded `x-goog-api-key` is redacted, so the key the case used is
//! substituted back. The recorded SDK and runtime user-agent identifiers are
//! replaced with the pinned provider package's user-agent suffix.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::tool::RawToolCall;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool, ToolChoice};
use aimux_core::types::FinishReasonUnified;
use aimux_providers::google::{GoogleProviderSettings, create_google};

use mock_fetch::{Canned, MockFetch, Seen};

const KEY: &str = "sk-test-fixture";

fn settings(mock: &Arc<MockFetch>) -> GoogleProviderSettings {
    GoogleProviderSettings {
        api_key: Some(KEY.to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    }
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
            "ai-sdk-google/4.0.85 ai-sdk-provider-utils/5.0.51".to_string(),
        );
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

/// Rebuild aimux call options from a recorded `sdk.input` (the subset the
/// Google fixtures use).
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
        .map(|value| serde_json::from_value(value.clone()).unwrap());
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
            GenerateContent::ToolCall(RawToolCall {
                tool_name, input, ..
            }) => Some((tool_name.clone(), input.clone())),
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
