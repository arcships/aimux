use crate::anthropic::{AnthropicProviderSettings, create_anthropic};

mod anthropic_provider_custom_provider_name {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - custom provider name › should use custom provider name when specified
    #[test]
    fn should_use_custom_provider_name_when_specified() {
        let provider = create_anthropic(AnthropicProviderSettings {
            base_url: Some("https://api.anthropic.com/v1".into()),
            name: Some("my-claude-proxy".into()),
            api_key: Some("test-api-key".into()),
            ..Default::default()
        })
        .unwrap();

        let model = provider.call("claude-3-haiku-20240307");
        assert_eq!(model.provider(), "my-claude-proxy");
    }
}

// ── Older ports (previously `tests/anthropic_factory_test.rs`) ──
//
// `create_anthropic` against the recorded AI SDK behavior.
//
// `fixtures/aisdk/anthropic/*.json` hold what `@ai-sdk/anthropic`'s
// `createAnthropic` sent over a mocked fetch for a fixed input: URL, headers,
// body and the model's `provider` string. Each fixture is replayed here
// through an injected [`Fetch`] (the factory's `fetch` setting), so the test
// also proves the transport is the one the settings name. The expected
// request comes from the fixture; the inputs are rebuilt from the fixture's
// recorded `sdk.input`.
//
// The recorded `x-api-key` is redacted, so the key the case used is
// substituted back. The user-agent aimux sends is checked against the
// recorded one (package and provider-utils parts).

mod older_factory {

    use crate::anthropic::{AnthropicProviderSettings, create_anthropic};
    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
    use aimux_core::options::CallOptions;
    use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput};
    use aimux_core::shared::provider_namespace;
    use aimux_core::stream_part::StreamPart;
    use aimux_core::tool::RawToolCall;
    use aimux_core::tool::{FunctionTool, LanguageModelV4ToolChoice, Tool};
    use aimux_core::types::{
        FinishReasonUnified, LanguageModelUsage, add_language_model_usage, as_language_model_usage,
    };
    use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};
    use async_trait::async_trait;
    use bytes::Bytes;
    use futures::StreamExt;
    use reqwest::StatusCode;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, VecDeque};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

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
        /// (names case-insensitively; user-agent checked against the recording) and body.
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
            // The user-agent is checked against the recording, not overwritten:
            // aimux sends `ai-sdk-anthropic/<v> ai-sdk-provider-utils/<v>`; every
            // recording carries the same provider-utils part, and a provider-layer
            // recording (not made through `ai`) starts with the same package part.
            let recorded_ua = expected.remove("user-agent").expect("recorded user-agent");
            let mut sent_headers = seen.headers.clone();
            let sent_ua = sent_headers.remove("user-agent").expect("sent user-agent");
            let (package_ua, utils_ua) = sent_ua
                .split_once(' ')
                .expect("package and provider-utils user-agent parts");
            assert!(
                package_ua.starts_with("ai-sdk-anthropic/")
                    && utils_ua.starts_with("ai-sdk-provider-utils/"),
                "{}: sent user-agent {sent_ua:?}",
                self.name
            );
            assert!(
                recorded_ua.contains(&format!(" {utils_ua} ")),
                "{}: recorded user-agent {recorded_ua:?} lacks {utils_ua:?}",
                self.name
            );
            if recorded_ua.starts_with("ai-sdk-anthropic/") {
                assert!(
                    recorded_ua.starts_with(&format!("{package_ua} ")),
                    "{}: recorded user-agent {recorded_ua:?} does not start with {package_ua:?}",
                    self.name
                );
            }
            assert_eq!(
                expected.get("x-api-key").map(String::as_str),
                Some("<redacted>"),
                "{}: the recording redacts the key",
                self.name
            );
            expected.insert("x-api-key".to_string(), KEY.to_string());
            assert_eq!(sent_headers, expected, "{}: headers", self.name);

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
                        let mut function =
                            FunctionTool::new(name.clone(), tool["inputSchema"].clone());
                        if let Some(description) = tool["description"].as_str() {
                            function = function.with_description(description);
                        }
                        Tool::Function(function)
                    })
                    .collect(),
            );
        }
        options.tool_choice = match input.get("toolChoice").and_then(Value::as_str) {
            Some("required") => Some(LanguageModelV4ToolChoice::Required),
            Some("none") => Some(LanguageModelV4ToolChoice::None),
            Some("auto") => Some(LanguageModelV4ToolChoice::Auto),
            None => options
                .tools
                .as_ref()
                .filter(|tools| !tools.is_empty())
                .map(|_| LanguageModelV4ToolChoice::Auto),
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
        // The recorded `result.usage` is ai@7.0.127's totalUsage, the sum over
        // the steps (generate-text.ts:1797-1799, types/usage.ts:122-155): both
        // detail objects present, `{}` when empty, and no raw. aimux's sum over
        // the one step writes the same JSON.
        let total = add_language_model_usage(
            &LanguageModelUsage::default(),
            &as_language_model_usage(&result.usage),
        );
        assert_eq!(
            serde_json::to_value(&total).unwrap(),
            recorded["usage"],
            "{}: usage JSON",
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
    #[serial_test::serial]
    async fn messages_basic() {
        replay_generate("messages-basic").await;
    }

    #[tokio::test]
    #[serial_test::serial]
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
    #[serial_test::serial]
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

    /// partial: `should accept providerOptions with custom provider name key`; full port in src/anthropic/tests/language_model.rs.
    #[tokio::test]
    #[serial_test::serial]
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

    /// `includeRawChunks`: every SSE chunk is forwarded as a `Raw` part before
    /// the parts made from it, a chunk that is not JSON included (it has no raw
    /// value), as anthropic@4.0.71 `anthropic-language-model.ts:1956-1963` does;
    /// that chunk is then reported as an error and the stream goes on. Without
    /// the option no `Raw` part is sent.
    /// partial: `should include raw chunks when includeRawChunks is enabled`; full port in src/anthropic/tests/language_model.rs.
    #[tokio::test]
    #[serial_test::serial]
    async fn messages_stream_emits_raw_chunks_when_asked() {
        let fixture = Fixture::load("messages-stream-basic");
        let mock = fixture.mock();
        let body = {
            let mut canned = mock.canned.lock().unwrap();
            let body = String::from_utf8(canned[0].body.clone()).unwrap();
            let (first, rest) = body.split_once("\n\n").unwrap();
            let body = format!("{first}\n\ndata: {{not json\n\n{rest}");
            canned[0].body = body.clone().into_bytes();
            body
        };
        let expected: Vec<Value> = body
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str(data).unwrap_or(Value::Null))
            .collect();
        let model = create_anthropic(settings(&mock))
            .unwrap()
            .messages(fixture.model_id());
        let options = CallOptions {
            include_raw_chunks: Some(true),
            ..fixture.call_options()
        };
        let items: Vec<_> = model
            .do_stream(&options)
            .await
            .unwrap()
            .stream
            .collect()
            .await;

        let raw: Vec<Value> = items
            .iter()
            .filter_map(|item| match item {
                Ok(StreamPart::Raw { raw_value }) => Some(raw_value.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(raw, expected);
        let not_json = items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    Ok(StreamPart::Raw {
                        raw_value: Value::Null
                    })
                )
            })
            .unwrap();
        // The chunk that does not parse is an error part, and the stream goes on
        // (anthropic-language-model.ts:1960-1963).
        assert!(
            matches!(
                &items[not_json + 1],
                Ok(StreamPart::Error { error }) if error.is_recoverable_stream_error()
            ),
            "{:?}",
            items[not_json + 1]
        );
        let first_delta = items
            .iter()
            .position(|item| matches!(item, Ok(StreamPart::TextDelta { .. })))
            .unwrap();
        // The chunk a delta was made from is the last raw part before it.
        let source = items[..first_delta]
            .iter()
            .rev()
            .find_map(|item| match item {
                Ok(StreamPart::Raw { raw_value }) => Some(raw_value),
                _ => None,
            });
        assert_eq!(source.unwrap()["delta"]["text"], "Hello");
        let text: String = items
            .iter()
            .filter_map(|item| match item {
                Ok(StreamPart::TextDelta { delta, .. }) => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello world");

        let model = create_anthropic(settings(&fixture.mock()))
            .unwrap()
            .messages(fixture.model_id());
        let result = model.do_stream(&fixture.call_options()).await.unwrap();
        let parts: Vec<_> = result.stream.collect().await;
        assert!(
            !parts
                .iter()
                .any(|item| matches!(item, Ok(StreamPart::Raw { .. })))
        );
    }

    /// partial: `should stream text deltas`; full port in src/anthropic/tests/language_model.rs.
    #[tokio::test]
    #[serial_test::serial]
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
        assert!(
            fixture.json["request"]["headers"]["user-agent"]
                .as_str()
                .unwrap()
                .starts_with("ai-sdk-anthropic/"),
            "the stream case is a provider-layer recording"
        );
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
}

// ── Older ports (previously `tests/anthropic_remaining_test.rs`) ──
//
// Remaining Anthropic provider tests — ported from the TS SDK suite.
//
// Sources:
//
// - `anthropic-provider.test.ts` — provider configuration (baseURL, auth,
//   custom provider name, `supportedUrls`).
// - `anthropic-unknown-model-max-output-tokens.test.ts` — unknown-model
//   `max_tokens` defaulting + compatibility warning.
// - `sanitize-json-schema.test.ts` — JSON Schema sanitization.
// - `anthropic-language-model.test.ts` → `mid-conversation tool changes`
//   describe block.

mod older_remaining {
    use super::super::remaining_support::*;
    use crate::anthropic::{AnthropicProviderSettings, create_anthropic};
    use crate::test_support::mock_fetch::EnvVar;
    use aimux_core::error::AiMuxError;
    use aimux_core::language_model::LanguageModel;
    use serial_test::serial;

    mod create_anthropic {
        use super::*;

        mod base_url_configuration {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › uses the default Anthropic base URL when not provided
            #[tokio::test]
            #[serial]
            async fn default_base_url_when_not_provided() {
                let _env = EnvVar::set("ANTHROPIC_BASE_URL", None);
                let fetch = mock();
                assert_eq!(
                    request_url(settings(&fetch), &fetch).await,
                    "https://api.anthropic.com/v1/messages"
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › uses ANTHROPIC_BASE_URL when set
            #[tokio::test]
            #[serial]
            async fn uses_anthropic_base_url_env_when_set() {
                let _env = EnvVar::set(
                    "ANTHROPIC_BASE_URL",
                    Some("https://proxy.anthropic.example/v1/"),
                );
                let fetch = mock();
                assert_eq!(
                    request_url(settings(&fetch), &fetch).await,
                    "https://proxy.anthropic.example/v1/messages"
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › normalizes a bare Anthropic API URL from ANTHROPIC_BASE_URL
            #[tokio::test]
            #[serial]
            async fn normalizes_bare_anthropic_url_from_env() {
                let _env = EnvVar::set("ANTHROPIC_BASE_URL", Some("https://api.anthropic.com/"));
                let fetch = mock();
                assert_eq!(
                    request_url(settings(&fetch), &fetch).await,
                    "https://api.anthropic.com/v1/messages"
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › normalizes a bare Anthropic API URL from the baseURL option
            #[tokio::test]
            async fn normalizes_bare_anthropic_url_from_base_url_option() {
                let fetch = mock();
                let settings = AnthropicProviderSettings {
                    base_url: Some("https://api.anthropic.com/".into()),
                    ..settings(&fetch)
                };
                assert_eq!(
                    request_url(settings, &fetch).await,
                    "https://api.anthropic.com/v1/messages"
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › prefers the baseURL option over ANTHROPIC_BASE_URL
            #[tokio::test]
            #[serial]
            async fn prefers_base_url_option_over_env() {
                let _env = EnvVar::set(
                    "ANTHROPIC_BASE_URL",
                    Some("https://env.anthropic.example/v1"),
                );
                let fetch = mock();
                let settings = AnthropicProviderSettings {
                    base_url: Some("https://option.anthropic.example/v1/".into()),
                    ..settings(&fetch)
                };
                assert_eq!(
                    request_url(settings, &fetch).await,
                    "https://option.anthropic.example/v1/messages"
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › rejects an empty baseURL option during provider creation
            #[test]
            fn rejects_empty_base_url_option() {
                let result = create_anthropic(AnthropicProviderSettings {
                    api_key: Some("test-api-key".into()),
                    base_url: Some(String::new()),
                    ..Default::default()
                });
                assert!(
                    matches!(result, Err(AiMuxError::InvalidArgument(msg)) if msg == "baseURL must be a non-empty string.")
                );
            }
        }
    }

    mod anthropic_provider_authentication {
        use super::*;

        mod auth_token_option {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - authentication › authToken option › sends Authorization Bearer header when authToken is provided
            #[tokio::test]
            #[serial_test::serial]
            async fn sends_authorization_bearer_when_auth_token_provided() {
                let fetch = mock();
                let settings = AnthropicProviderSettings {
                    auth_token: Some("test-auth-token".into()),
                    api_key: None,
                    ..settings(&fetch)
                };
                request_url(settings, &fetch).await;
                let requests = fetch.seen();
                assert_eq!(
                    requests[0].headers.get("authorization").map(String::as_str),
                    Some("Bearer test-auth-token")
                );
                assert!(!requests[0].headers.contains_key("x-api-key"));
            }
        }

        mod api_key_and_auth_token_conflict {
            use super::*;

            /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - authentication › apiKey and authToken conflict › throws error when both apiKey and authToken options are provided
            #[test]
            fn throws_when_both_api_key_and_auth_token_provided() {
                let result = create_anthropic(AnthropicProviderSettings {
                    api_key: Some("test-api-key".into()),
                    auth_token: Some("test-auth-token".into()),
                    ..Default::default()
                });
                assert!(
                    matches!(result, Err(AiMuxError::InvalidArgument(msg)) if msg == "Both apiKey and authToken were provided. Please use only one authentication method.")
                );
            }
        }
    }

    mod anthropic_provider_custom_provider_name {
        use super::*;

        /// partial: `should use custom provider name when specified`; full port in src/anthropic/tests/provider.rs.
        #[test]
        fn uses_custom_provider_name_when_specified() {
            let model = make_model_with_config(AnthropicProviderSettings {
                name: Some("my-proxy".into()),
                api_key: Some("test-api-key".into()),
                ..Default::default()
            });
            assert_eq!(model.provider(), "my-proxy");
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - custom provider name › should default to anthropic.messages when name not specified
        #[test]
        fn defaults_to_anthropic_messages_when_not_specified() {
            let model = make_model_with_config(AnthropicProviderSettings {
                api_key: Some("test-api-key".into()),
                ..Default::default()
            });
            assert_eq!(model.provider(), "anthropic.messages");
        }
    }

    mod anthropic_provider_supported_urls {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - supportedUrls › should support image/* URLs
        #[tokio::test]
        async fn supports_image_urls() {
            let model = make_model_with_config(AnthropicProviderSettings::default());
            assert!(
                model.supported_urls().await.0["image/*"][0]
                    .is_match("https://example.com/image.png")
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - supportedUrls › should support application/pdf URLs
        #[tokio::test]
        async fn supports_pdf_urls() {
            let model = make_model_with_config(AnthropicProviderSettings::default());
            assert!(
                model.supported_urls().await.0["application/pdf"][0]
                    .is_match("https://arxiv.org/pdf/2401.00001")
            );
        }
    }
}
