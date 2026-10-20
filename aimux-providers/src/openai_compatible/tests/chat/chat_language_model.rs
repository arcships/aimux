use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::shared::{FileBytes, FileData, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::ProviderMetadata;

use crate::openai_compatible::{BaseUrl, ChatSettings, CompatModelConfig};
use crate::openai_compatible::{
    MetadataExtractor, OpenAICompatibleChatModel, OpenAICompatibleProvider,
    OpenAICompatibleProviderSettings, StreamMetadataExtractor, create_openai_compatible,
};
use crate::shared::{AuthScheme, Credential, ProviderHeaders};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

/// The user agent aimux pins (upstream's test server sees `0.0.0-test`
/// versions and its header snapshots leave the header out).
const USER_AGENT: &str = "ai-sdk-openai-compatible/3.0.62 ai-sdk-provider-utils/5.0.53";

const TEST_SCHEMA_VALUE: &str = r#"{"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false,"$schema":"http://json-schema.org/draft-07/schema#"}"#;

/// `TEST_PROMPT`.
fn test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

/// `LanguageModelV4CallOptions` from their JSON form; a missing `prompt` is
/// `TEST_PROMPT`.
fn call_options(mut options: Value) -> CallOptions {
    options
        .as_object_mut()
        .expect("call options are an object")
        .entry("prompt")
        .or_insert_with(test_prompt);
    serde_json::from_value(options).expect("valid call options")
}

/// `{ type: 'function', name, inputSchema }`.
fn function_tool(name: &str, input_schema: Value) -> Value {
    json!({ "type": "function", "name": name, "inputSchema": input_schema })
}

/// The `{ value: string }` input schema most tool tests use.
fn value_schema() -> Value {
    serde_json::from_str(TEST_SCHEMA_VALUE).unwrap()
}

/// `createOpenAICompatible({ baseURL: 'https://my.api.com/v1/', name:
/// 'test-provider' })` on the mock transport; the struct-update base of
/// every provider and model below.
fn settings(fetch: &Arc<MockFetch>) -> OpenAICompatibleProviderSettings {
    OpenAICompatibleProviderSettings {
        name: "test-provider".into(),
        base_url: "https://my.api.com/v1/".into(),
        fetch: Some(fetch.transport()),
        ..Default::default()
    }
}

fn headers(pairs: &[(&str, &str)]) -> HashMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
        .collect()
}

/// The file-level `provider` (with `Authorization: Bearer test-api-key`).
fn provider(fetch: &Arc<MockFetch>) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        headers: Some(headers(&[("Authorization", "Bearer test-api-key")])),
        ..settings(fetch)
    })
    .unwrap()
}

/// The file-level `model` (`provider('grok-3')`).
fn model(fetch: &Arc<MockFetch>) -> OpenAICompatibleChatModel {
    provider(fetch).chat("grok-3")
}

/// `new OpenAICompatibleChatLanguageModel(modelId, { provider:
/// 'test-provider', url: () => 'https://my.api.com/v1/chat/completions',
/// headers: () => ({}), ...config })`: the same config through the factory.
fn direct_model(
    model_id: &str,
    settings: OpenAICompatibleProviderSettings,
) -> OpenAICompatibleChatModel {
    create_openai_compatible(settings).unwrap().chat(model_id)
}

/// A `json-value` response of the test server: `content-type` and
/// `content-length` of the serialized body, then `headers`.
fn json_value(body: &Value, extra_headers: &[(&str, &str)]) -> Canned {
    let body = serde_json::to_vec(body).unwrap();
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), body.len().to_string()),
    ];
    headers.extend(
        extra_headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    Canned {
        status: 200,
        headers,
        body,
    }
}

/// A `stream-chunks` response of the test server.
fn stream_chunks<S: AsRef<str>>(chunks: &[S], extra_headers: &[(&str, &str)]) -> Canned {
    let mut headers = vec![
        ("content-type".to_string(), "text/event-stream".to_string()),
        ("cache-control".to_string(), "no-cache".to_string()),
        ("connection".to_string(), "keep-alive".to_string()),
    ];
    headers.extend(
        extra_headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    Canned {
        status: 200,
        headers,
        body: chunks
            .iter()
            .map(AsRef::as_ref)
            .collect::<String>()
            .into_bytes(),
    }
}

fn fixture(filename: &str) -> &'static str {
    match filename {
        "xai-text.json" => include_str!("fixtures/xai-text.json"),
        "xai-tool-call.json" => include_str!("fixtures/xai-tool-call.json"),
        "xai-text.chunks.txt" => include_str!("fixtures/xai-text.chunks.txt"),
        "xai-tool-call.chunks.txt" => include_str!("fixtures/xai-tool-call.chunks.txt"),
        "placeholder-first-chunk.chunks.txt" => {
            include_str!("fixtures/placeholder-first-chunk.chunks.txt")
        }
        "anthropic-fallback-tool-call.sse" => {
            include_str!("fixtures/anthropic-fallback-tool-call.sse")
        }
        other => panic!("no fixture {other}"),
    }
}

/// `prepareJsonFixtureResponse`.
fn prepare_json_fixture_response(filename: &str, extra_headers: &[(&str, &str)]) -> Canned {
    let body: Value = serde_json::from_str(fixture(&format!("{filename}.json"))).unwrap();
    json_value(&body, extra_headers)
}

/// `prepareChunksFixtureResponse`.
fn prepare_chunks_fixture_response(filename: &str, extra_headers: &[(&str, &str)]) -> Canned {
    let mut chunks: Vec<String> = fixture(&format!("{filename}.chunks.txt"))
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("data: {line}\n\n"))
        .collect();
    chunks.push("data: [DONE]\n\n".to_string());
    stream_chunks(&chunks, extra_headers)
}

/// `prepareSseFixtureResponse`.
fn prepare_sse_fixture_response(filename: &str) -> Canned {
    stream_chunks(&[fixture(&format!("{filename}.sse"))], &[])
}

/// A `toMatchSnapshot()` value of a `doGenerate` result. Upstream's
/// `request.body` is the JSON string that was sent; aimux keeps the body as
/// the JSON value, so the snapshot's string is compared parsed.
fn generate_snapshot(text: &str) -> Value {
    let mut snapshot: Value = serde_json::from_str(text).unwrap();
    if let Some(body) = snapshot.pointer_mut("/request/body")
        && let Some(text) = body.as_str()
    {
        *body = serde_json::from_str(text).unwrap();
    }
    snapshot
}

fn result_json(result: &GenerateResult) -> Value {
    serde_json::to_value(result).unwrap()
}

fn request_body(fetch: &MockFetch) -> Value {
    fetch.seen()[0].json_body()
}

/// `server.calls[0].requestHeaders` without `user-agent`, which is checked
/// on its own against the pinned value.
fn request_headers(seen: &Seen) -> Value {
    let mut headers = seen.headers.clone();
    assert_eq!(headers.remove("user-agent").as_deref(), Some(USER_AGENT));
    serde_json::to_value(headers).unwrap()
}

/// The error of an error part as the snapshots show it: upstream passes an
/// error chunk's `error` object on as it is (aimux's `ApiCallError.data`);
/// thrown errors are `[AI_<Name>: <message>]`, here `{ name, message }`.
fn error_json(error: &AiMuxError) -> Value {
    match error {
        AiMuxError::ApiCall(detail) => detail.data.clone().unwrap_or(Value::Null),
        AiMuxError::InvalidResponseData(message) => {
            json!({ "name": "AI_InvalidResponseDataError", "message": message })
        }
        AiMuxError::JsonParse(message) => {
            json!({ "name": "AI_JSONParseError", "message": message })
        }
        other => json!({ "name": format!("{other:?}"), "message": other.to_string() }),
    }
}

fn part_json(part: StreamPart) -> Value {
    match part {
        StreamPart::Error { error } => json!({ "type": "error", "error": error_json(&error) }),
        part => serde_json::to_value(part).unwrap(),
    }
}

/// `convertReadableStreamToArray`: rejects with the error the stream ends
/// with.
async fn convert_stream_to_array(result: StreamResult) -> Result<Vec<Value>, AiMuxError> {
    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(item) = stream.next().await {
        parts.push(part_json(item?));
    }
    Ok(parts)
}

async fn stream_parts(result: StreamResult) -> Vec<Value> {
    convert_stream_to_array(result).await.unwrap()
}

fn of_type<'a>(parts: &'a [Value], kind: &str) -> Vec<&'a Value> {
    parts.iter().filter(|part| part["type"] == kind).collect()
}

fn find<'a>(parts: &'a [Value], kind: &str) -> &'a Value {
    parts
        .iter()
        .find(|part| part["type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} part"))
}

/// `toMatchObject`: every property of `expected` is in `actual` and matches
/// recursively; arrays match element by element and have the same length.
#[track_caller]
fn assert_match_object(actual: &Value, expected: &Value) {
    fn matches(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => expected
                .iter()
                .all(|(key, value)| actual.get(key).is_some_and(|a| matches(a, value))),
            (Value::Array(actual), Value::Array(expected)) => {
                actual.len() == expected.len()
                    && actual.iter().zip(expected).all(|(a, e)| matches(a, e))
            }
            (actual, expected) => actual == expected,
        }
    }
    assert!(
        matches(actual, expected),
        "toMatchObject failed\n  actual: {actual:#}\nexpected: {expected:#}"
    );
}

/// `prepareJsonResponse` of the `doGenerate` block: `overrides` holds the
/// named arguments that differ from upstream's defaults.
fn prepare_json_response(overrides: Value) -> Canned {
    let get = |key: &str, default: Value| overrides.get(key).cloned().unwrap_or(default);
    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    message.insert("content".into(), get("content", json!("")));
    message.insert(
        "reasoning_content".into(),
        get("reasoning_content", json!("")),
    );
    message.insert("reasoning".into(), get("reasoning", json!("")));
    for key in ["tool_calls", "function_call"] {
        if let Some(value) = overrides.get(key) {
            message.insert(key.into(), value.clone());
        }
    }
    json_value(
        &json!({
            "id": get("id", json!("chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd")),
            "object": "chat.completion",
            "created": get("created", json!(1711115037)),
            "model": get("model", json!("grok-3")),
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": get("finish_reason", json!("stop")),
            }],
            "usage": get("usage", json!({ "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 })),
            "system_fingerprint": "fp_3bc1b5746c",
        }),
        &[],
    )
}

/// `prepareStreamResponse` of the `doStream` block.
fn prepare_stream_response(content: &[&str], finish_reason: &str) -> Canned {
    const PREFIX: &str = r#"data: {"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1702657020,"model":"grok-3","#;
    let mut chunks = vec![
        format!(
            r#"{PREFIX}"system_fingerprint":null,"choices":[{{"index":0,"delta":{{"role":"assistant","content":""}},"finish_reason":null}}]}}"#
        ) + "\n\n",
    ];
    for text in content {
        chunks.push(
            format!(
                r#"{PREFIX}"system_fingerprint":null,"choices":[{{"index":1,"delta":{{"content":"{text}"}},"finish_reason":null}}]}}"#
            ) + "\n\n",
        );
    }
    chunks.push(
        format!(
            r#"{PREFIX}"system_fingerprint":null,"choices":[{{"index":0,"delta":{{}},"finish_reason":"{finish_reason}"}}]}}"#
        ) + "\n\n",
    );
    chunks.push(
        format!(
            concat!(
                r#"data: {{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","#,
                r#""system_fingerprint":"fp_10c08bf97d","choices":[{{"index":0,"delta":{{}},"finish_reason":"{finish_reason}"}}],"#,
                r#""usage":{{"queue_time":0.061348671,"prompt_tokens":18,"prompt_time":0.000211569,"#,
                r#""completion_tokens":439,"completion_time":0.798181818,"total_tokens":457,"total_time":0.798393387}}}}"#
            ),
            finish_reason = finish_reason
        ) + "\n\n",
    );
    chunks.push("data: [DONE]\n\n".to_string());
    stream_chunks(&chunks, &[])
}

/// `data: <json>\n\n` for every chunk, as the test bodies spell them.
fn data(chunks: &[&str]) -> Vec<String> {
    chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect()
}

/// `data(chunks)` followed by `data: [DONE]`.
fn data_done(chunks: &[&str]) -> Vec<String> {
    let mut all = data(chunks);
    all.push("data: [DONE]\n\n".to_string());
    all
}

fn json_mock(body: Value) -> Arc<MockFetch> {
    MockFetch::new(vec![json_value(&body, &[])])
}

fn stream_mock(chunks: &[&str]) -> Arc<MockFetch> {
    MockFetch::new(vec![stream_chunks(&data_done(chunks), &[])])
}

/// `new OpenAICompatibleChatLanguageModel('gpt-5', { provider, url: () => '',
/// headers: () => ({}) })`.
fn config_model(provider: &str) -> OpenAICompatibleChatModel {
    OpenAICompatibleChatModel::from_config(
        "gpt-5".to_string(),
        CompatModelConfig {
            provider: provider.to_string(),
            base_url: BaseUrl::Fixed(String::new()),
            query_params: None,
            headers: ProviderHeaders::new(Credential::None, AuthScheme::Bearer, Vec::new(), None),
            fetch_headers: None,
            fetch: None,
            transform_request_body: None,
            chat: ChatSettings::default(),
        },
    )
}

/// The `testMetadataExtractor` of the `metadata extraction` block.
struct TestMetadataExtractor;

impl MetadataExtractor for TestMetadataExtractor {
    fn extract_metadata(&self, parsed_body: &Value) -> Option<ProviderMetadata> {
        let value = parsed_body.as_object()?.get("test_field")?;
        Some(provider_namespace("test-provider", json!({ "value": value })).unwrap())
    }

    fn create_stream_extractor(&self) -> Box<dyn StreamMetadataExtractor> {
        Box::new(TestStreamMetadataExtractor {
            accumulated_value: None,
        })
    }
}

struct TestStreamMetadataExtractor {
    accumulated_value: Option<Value>,
}

impl StreamMetadataExtractor for TestStreamMetadataExtractor {
    fn process_chunk(&mut self, chunk: &Value) {
        let stops = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("finish_reason"))
            == Some(&json!("stop"));
        if stops && let Some(value) = chunk.get("test_field") {
            self.accumulated_value = Some(value.clone());
        }
    }

    fn build_metadata(&self) -> Option<ProviderMetadata> {
        // `accumulatedValue ? { ... } : undefined`
        let value = self
            .accumulated_value
            .as_ref()
            .filter(|value| value.as_str().is_none_or(|text| !text.is_empty()))?;
        Some(provider_namespace("test-provider", json!({ "value": value })).unwrap())
    }
}

mod config {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › config › should extract base name from provider string
    #[test]
    fn should_extract_base_name_from_provider_string() {
        let model = config_model("anthropic.beta");

        assert_eq!(model.config.provider_options_name(), "anthropic");
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › config › should handle provider without dot notation
    #[test]
    fn should_handle_provider_without_dot_notation() {
        let model = config_model("openai");

        assert_eq!(model.config.provider_options_name(), "openai");
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › config › should return empty for empty provider
    #[test]
    fn should_return_empty_for_empty_provider() {
        let model = config_model("");

        assert_eq!(model.config.provider_options_name(), "");
    }
}

mod do_generate {
    use super::*;

    mod text_fixture {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › text (fixture) › should extract text content
        #[tokio::test]
        async fn should_extract_text_content() {
            let fetch = MockFetch::new(vec![prepare_json_fixture_response("xai-text", &[])]);

            let result = model(&fetch)
                .do_generate(&call_options(json!({})))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result),
                generate_snapshot(include_str!("snapshots/do_generate_text_fixture.json"))
            );
        }
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should reject a response without choices
    #[tokio::test]
    async fn should_reject_a_response_without_choices() {
        let fetch = json_mock(json!({
            "id": "chatcmpl-empty",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "grok-3",
            "choices": [],
            "usage": { "prompt_tokens": 4, "total_tokens": 4, "completion_tokens": 0 },
        }));

        let error = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap_err();

        assert!(
            matches!(&error, AiMuxError::InvalidResponseData(message) if message == "Response did not contain any choices."),
            "{error:?}"
        );
    }

    mod tool_call_fixture {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › tool call (fixture) › should extract tool call content
        #[tokio::test]
        async fn should_extract_tool_call_content() {
            let fetch = MockFetch::new(vec![prepare_json_fixture_response("xai-tool-call", &[])]);

            let result = model(&fetch)
                .do_generate(&call_options(json!({})))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result),
                generate_snapshot(include_str!("snapshots/do_generate_tool_call_fixture.json"))
            );
        }
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should generate distinct IDs for parallel tool calls with empty IDs
    #[tokio::test]
    async fn should_generate_distinct_ids_for_parallel_tool_calls_with_empty_ids() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({
            "finish_reason": "tool_calls",
            "tool_calls": [
                { "id": "", "type": "function", "function": { "name": "first", "arguments": "{\"value\":1}" } },
                { "id": "", "type": "function", "function": { "name": "second", "arguments": "{\"value\":2}" } },
            ],
        }))]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();
        let content = result_json(&result)["content"].clone();
        let tool_call_ids: Vec<String> = content
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "tool-call")
            .map(|part| part["toolCallId"].as_str().unwrap().to_string())
            .collect();

        assert_eq!(tool_call_ids.len(), 2);
        assert!(tool_call_ids.iter().all(|id| !id.is_empty()));
        assert_eq!(
            tool_call_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            2
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should extract usage
    #[tokio::test]
    async fn should_extract_usage() {
        let fetch = MockFetch::new(vec![prepare_json_fixture_response("xai-text", &[])]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&result)["usage"],
            json!({
                "inputTokens": {
                    "cacheRead": 2,
                    "noCache": 10,
                    "total": 12
                },
                "outputTokens": {
                    "reasoning": 320,
                    "text": 0,
                    "total": 2
                },
                "raw": {
                    "completion_tokens": 2,
                    "completion_tokens_details": {
                        "accepted_prediction_tokens": 0,
                        "audio_tokens": 0,
                        "reasoning_tokens": 320,
                        "rejected_prediction_tokens": 0
                    },
                    "cost_in_usd_ticks": 1641500,
                    "num_sources_used": 0,
                    "prompt_tokens": 12,
                    "prompt_tokens_details": {
                        "audio_tokens": 0,
                        "cached_tokens": 2,
                        "image_tokens": 0,
                        "text_tokens": 12
                    },
                    "total_tokens": 334
                }
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should send additional response information
    #[tokio::test]
    async fn should_send_additional_response_information() {
        let fetch = MockFetch::new(vec![prepare_json_fixture_response("xai-text", &[])]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();
        let response = result.response.unwrap();

        assert_eq!(
            response.id.as_deref(),
            Some("edea4703-19aa-6d74-fedb-dc1c213543e0")
        );
        assert_eq!(
            response.timestamp.map(|timestamp| timestamp.to_string()),
            Some("2026-02-11T01:08:10.000Z".to_string())
        );
        assert_eq!(response.model_id.as_deref(), Some("grok-3-mini"));
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let fetch = MockFetch::new(vec![prepare_json_fixture_response(
            "xai-text",
            &[("test-header", "test-value")],
        )]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&result)["response"]["headers"],
            json!({
                "content-length": "2053",
                "content-type": "application/json",
                "test-header": "test-value"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should pass user setting to requests
    #[tokio::test]
    async fn should_pass_user_setting_to_requests() {
        let fetch = MockFetch::new(vec![prepare_json_response(
            json!({ "content": "Hello, World!" }),
        )]);
        let model_with_user = provider(&fetch).chat("grok-3");

        model_with_user
            .do_generate(&call_options(json!({
                "providerOptions": { "xai": { "user": "test-user-id" } },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should extract reasoning from reasoning field when reasoning_content is not provided
    #[tokio::test]
    async fn should_extract_reasoning_from_reasoning_field_when_reasoning_content_is_not_provided()
    {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({
            "content": "Hello, World!",
            "reasoning": "This is the reasoning from the reasoning field",
        }))]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&result)["content"],
            json!([
                {
                    "text": "Hello, World!",
                    "type": "text"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should prefer reasoning_content over reasoning field when both are provided
    #[tokio::test]
    async fn should_prefer_reasoning_content_over_reasoning_field_when_both_are_provided() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({
            "content": "Hello, World!",
            "reasoning_content": "This is from reasoning_content",
            "reasoning": "This is from reasoning field",
        }))]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&result)["content"],
            json!([
                {
                    "text": "Hello, World!",
                    "type": "text"
                },
                {
                    "text": "This is from reasoning_content",
                    "type": "reasoning"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should normalize text and thinking content parts
    #[tokio::test]
    async fn should_normalize_text_and_thinking_content_parts() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({
            "content": [
                {
                    "type": "thinking",
                    "thinking": [
                        { "type": "text", "text": "Let me think" },
                        { "type": "text", "text": " this through." },
                    ],
                },
                { "type": "text", "text": "The answer is 391." },
            ],
        }))]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&result)["content"],
            json!([
                { "type": "reasoning", "text": "Let me think this through." },
                { "type": "text", "text": "The answer is 391." },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should ignore unknown content parts
    #[tokio::test]
    async fn should_ignore_unknown_content_parts() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({
            "content": [
                {
                    "type": "future-part",
                    "text": { "nested": true },
                    "thinking": { "nested": true },
                },
                { "type": "text", "text": "The answer is 391." },
            ],
        }))]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&result)["content"],
            json!([{ "type": "text", "text": "The answer is 391." }])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should support partial usage
    #[tokio::test]
    async fn should_support_partial_usage() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({
            "usage": { "prompt_tokens": 20, "total_tokens": 20 },
        }))]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&result)["usage"],
            json!({
                "inputTokens": {
                    "cacheRead": 0,
                    "noCache": 20,
                    "total": 20
                },
                "outputTokens": {
                    "reasoning": 0,
                    "text": 0,
                    "total": 0
                },
                "raw": {
                    "prompt_tokens": 20,
                    "total_tokens": 20
                }
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should support unknown finish reason
    #[tokio::test]
    async fn should_support_unknown_finish_reason() {
        let fetch = MockFetch::new(vec![prepare_json_response(
            json!({ "finish_reason": "eos" }),
        )]);

        let response = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&response)["finishReason"],
            json!({
                "raw": "eos",
                "unified": "other"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should pass the model and the messages
    #[tokio::test]
    async fn should_pass_the_model_and_the_messages() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "" }))]);

        model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should pass video input as video_url in doGenerate
    #[tokio::test]
    async fn should_pass_video_input_as_video_url_in_do_generate() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "" }))]);
        let mut options = call_options(json!({
            "prompt": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "Describe this video." },
                    { "type": "file", "data": { "type": "data", "data": "" }, "mediaType": "video/mp4" },
                ],
            }],
        }));
        // `data: new Uint8Array([0, 1, 2, 3])`
        if let LanguageModelMessage::User { content, .. } = &mut options.prompt[0]
            && let UserPart::File(file) = &mut content[1]
        {
            file.data = FileData::Data {
                data: FileBytes::Binary(vec![0, 1, 2, 3]),
            };
        }

        model(&fetch).do_generate(&options).await.unwrap();

        assert_match_object(
            &request_body(&fetch),
            &json!({
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Describe this video." },
                        { "type": "video_url", "video_url": { "url": "data:video/mp4;base64,AAECAw==" } },
                    ],
                }],
            }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should pass multi-part tool content when supported
    #[tokio::test]
    async fn should_pass_multi_part_tool_content_when_supported() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "" }))]);
        let multi_part_provider = create_openai_compatible(OpenAICompatibleProviderSettings {
            supports_multi_part_tool_content: Some(true),
            ..settings(&fetch)
        })
        .unwrap();

        multi_part_provider
            .chat("grok-3")
            .do_generate(&call_options(json!({
                "prompt": [{
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolCallId": "call-1",
                        "toolName": "useImage",
                        "output": {
                            "type": "content",
                            "value": [
                                { "type": "text", "text": "image result" },
                                {
                                    "type": "file",
                                    "data": { "type": "data", "data": "iVBORw0KGgo=" },
                                    "mediaType": "image/png",
                                },
                            ],
                        },
                    }],
                }],
            })))
            .await
            .unwrap();

        assert_match_object(
            &request_body(&fetch),
            &json!({
                "messages": [{
                    "role": "tool",
                    "tool_call_id": "call-1",
                    "content": [
                        { "type": "text", "text": "image result" },
                        { "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBORw0KGgo=" } },
                    ],
                }],
            }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should pass settings
    #[tokio::test]
    async fn should_pass_settings() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({}))]);

        provider(&fetch)
            .chat("grok-3")
            .do_generate(&call_options(json!({
                "providerOptions": { "openaiCompatible": { "user": "test-user-id" } },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3",
                "user": "test-user-id"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should pass settings with deprecated openai-compatible key and emit warning
    #[tokio::test]
    async fn should_pass_settings_with_deprecated_openai_compatible_key_and_emit_warning() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({}))]);

        let result = provider(&fetch)
            .chat("grok-3")
            .do_generate(&call_options(json!({
                "providerOptions": { "openai-compatible": { "user": "test-user-id" } },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3",
                "user": "test-user-id"
            })
        );
        assert_eq!(
            result_json(&result)["warnings"],
            json!([
                {
                    "message": "Use 'openaiCompatible' instead.",
                    "setting": "providerOptions key 'openai-compatible'",
                    "type": "deprecated"
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should include provider-specific options
    #[tokio::test]
    async fn should_include_provider_specific_options() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({}))]);

        provider(&fetch)
            .chat("grok-3")
            .do_generate(&call_options(json!({
                "providerOptions": { "test-provider": { "someCustomOption": "test-value" } },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3",
                "someCustomOption": "test-value"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should serialize thought signatures using the custom provider metadata key
    #[tokio::test]
    async fn should_serialize_thought_signatures_using_the_custom_provider_metadata_key() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "" }))]);

        provider(&fetch)
            .chat("grok-3")
            .do_generate(&call_options(json!({
                "prompt": [{
                    "role": "assistant",
                    "content": [{
                        "type": "tool-call",
                        "toolCallId": "call-1",
                        "toolName": "test_tool",
                        "input": { "value": "test" },
                        "providerOptions": {
                            "test-provider": { "thoughtSignature": "<Signature A>" },
                        },
                    }],
                }],
            })))
            .await
            .unwrap();

        assert_match_object(
            &request_body(&fetch),
            &json!({
                "messages": [{
                    "role": "assistant",
                    "tool_calls": [{
                        "extra_content": { "google": { "thought_signature": "<Signature A>" } },
                    }],
                }],
            }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should not include provider-specific options for different provider
    #[tokio::test]
    async fn should_not_include_provider_specific_options_for_different_provider() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({}))]);

        provider(&fetch)
            .chat("grok-3")
            .do_generate(&call_options(json!({
                "providerOptions": { "notThisProviderName": { "someCustomOption": "test-value" } },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3"
            })
        );
    }

    mod camel_case_provider_options {
        use super::*;

        /// The usage of the two metadata-key tests.
        fn prediction_usage() -> Value {
            json!({
                "prompt_tokens": 20,
                "completion_tokens": 30,
                "total_tokens": 50,
                "completion_tokens_details": { "accepted_prediction_tokens": 15 },
            })
        }

        fn signature_tool_calls() -> Value {
            json!([{
                "id": "call-1",
                "type": "function",
                "function": { "name": "test_tool", "arguments": "{\"arg\":\"value\"}" },
                "extra_content": { "google": { "thought_signature": "<Sig>" } },
            }])
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should accept camelCase provider options key for hyphenated provider name
        #[tokio::test]
        async fn should_accept_camel_case_provider_options_key_for_hyphenated_provider_name() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "Hello!" }))]);

            provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({
                    "providerOptions": { "testProvider": { "someCustomOption": "test-value" } },
                })))
                .await
                .unwrap();

            assert_match_object(
                &request_body(&fetch),
                &json!({ "someCustomOption": "test-value" }),
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should prefer camelCase options over raw-name options
        #[tokio::test]
        async fn should_prefer_camel_case_options_over_raw_name_options() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "Hello!" }))]);

            provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({
                    "providerOptions": {
                        "test-provider": { "someCustomOption": "raw-value" },
                        "testProvider": { "someCustomOption": "camel-value" },
                    },
                })))
                .await
                .unwrap();

            assert_match_object(
                &request_body(&fetch),
                &json!({ "someCustomOption": "camel-value" }),
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should use camelCase metadata key when camelCase provider options are used
        #[tokio::test]
        async fn should_use_camel_case_metadata_key_when_camel_case_provider_options_are_used() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({
                "content": "Hello!",
                "usage": prediction_usage(),
            }))]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({
                    "providerOptions": { "testProvider": { "reasoningEffort": "high" } },
                })))
                .await
                .unwrap();
            let metadata = result.provider_metadata.unwrap();

            assert!(metadata.contains_key("testProvider"));
            assert!(!metadata.contains_key("test-provider"));
            assert_match_object(
                &Value::Object(metadata["testProvider"].clone()),
                &json!({ "acceptedPredictionTokens": 15 }),
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should use raw metadata key when raw provider options are used
        #[tokio::test]
        async fn should_use_raw_metadata_key_when_raw_provider_options_are_used() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({
                "content": "Hello!",
                "usage": prediction_usage(),
            }))]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({
                    "providerOptions": { "test-provider": { "reasoningEffort": "high" } },
                })))
                .await
                .unwrap();
            let metadata = result.provider_metadata.unwrap();

            assert!(metadata.contains_key("test-provider"));
            assert_match_object(
                &Value::Object(metadata["test-provider"].clone()),
                &json!({ "acceptedPredictionTokens": 15 }),
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should emit deprecated warning when raw provider options key is used
        #[tokio::test]
        async fn should_emit_deprecated_warning_when_raw_provider_options_key_is_used() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "Hello!" }))]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({
                    "providerOptions": { "test-provider": { "reasoningEffort": "high" } },
                })))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result)["warnings"],
                json!([
                    {
                        "message": "Use 'testProvider' instead.",
                        "setting": "providerOptions key 'test-provider'",
                        "type": "deprecated"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should not emit deprecated warning when camelCase provider options key is used
        #[tokio::test]
        async fn should_not_emit_deprecated_warning_when_camel_case_provider_options_key_is_used() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "Hello!" }))]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({
                    "providerOptions": { "testProvider": { "reasoningEffort": "high" } },
                })))
                .await
                .unwrap();

            assert_eq!(result_json(&result)["warnings"], json!([]));
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should use raw metadata key when no provider options are passed
        #[tokio::test]
        async fn should_use_raw_metadata_key_when_no_provider_options_are_passed() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "Hello!" }))]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({})))
                .await
                .unwrap();

            assert!(
                result
                    .provider_metadata
                    .unwrap()
                    .contains_key("test-provider")
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should include thought signature in providerMetadata with camelCase key
        #[tokio::test]
        async fn should_include_thought_signature_in_provider_metadata_with_camel_case_key() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "tool_calls": signature_tool_calls() }),
            )]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({
                    "providerOptions": { "testProvider": {} },
                })))
                .await
                .unwrap();

            assert_match_object(
                &result_json(&result)["content"],
                &json!([{
                    "type": "tool-call",
                    "providerMetadata": { "testProvider": { "thoughtSignature": "<Sig>" } },
                }]),
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › camelCase provider options › should include thought signature in providerMetadata with raw key
        #[tokio::test]
        async fn should_include_thought_signature_in_provider_metadata_with_raw_key() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "tool_calls": signature_tool_calls() }),
            )]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_generate(&call_options(json!({
                    "providerOptions": { "test-provider": {} },
                })))
                .await
                .unwrap();

            assert_match_object(
                &result_json(&result)["content"],
                &json!([{
                    "type": "tool-call",
                    "providerMetadata": { "test-provider": { "thoughtSignature": "<Sig>" } },
                }]),
            );
        }
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should pass tools and toolChoice
    #[tokio::test]
    async fn should_pass_tools_and_tool_choice() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "" }))]);

        model(&fetch)
            .do_generate(&call_options(json!({
                "tools": [function_tool("test-tool", value_schema())],
                "toolChoice": { "type": "tool", "toolName": "test-tool" },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3",
                "tool_choice": {
                    "function": {
                        "name": "test-tool"
                    },
                    "type": "function"
                },
                "tools": [
                    {
                        "function": {
                            "name": "test-tool",
                            "parameters": {
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
                            }
                        },
                        "type": "function"
                    }
                ]
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "" }))]);
        let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
            headers: Some(headers(&[
                ("Authorization", "Bearer test-api-key"),
                ("Custom-Provider-Header", "provider-header-value"),
            ])),
            ..settings(&fetch)
        })
        .unwrap();

        provider
            .chat("grok-3")
            .do_generate(&call_options(json!({
                "headers": { "Custom-Request-Header": "request-header-value" },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_headers(&fetch.seen()[0]),
            json!({
                "authorization": "Bearer test-api-key",
                "content-type": "application/json",
                "custom-provider-header": "provider-header-value",
                "custom-request-header": "request-header-value"
            })
        );
    }

    mod google_gemini_thought_signatures_openai_compatibility {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › Google Gemini thought signatures (OpenAI compatibility) › should parse thought signature from extra_content and include in providerMetadata
        #[tokio::test]
        async fn should_parse_thought_signature_from_extra_content_and_include_in_provider_metadata()
         {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({
                "tool_calls": [{
                    "id": "function-call-1",
                    "type": "function",
                    "function": { "name": "check_flight", "arguments": "{\"flight\":\"AA100\"}" },
                    "extra_content": { "google": { "thought_signature": "<Signature A>" } },
                }],
            }))]);

            let result = model(&fetch)
                .do_generate(&call_options(json!({
                    "tools": [function_tool("check_flight", json!({
                        "type": "object",
                        "properties": { "flight": { "type": "string" } },
                        "required": ["flight"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                    }))],
                })))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result)["content"],
                json!([
                    {
                        "input": "{\"flight\":\"AA100\"}",
                        "providerMetadata": {
                            "test-provider": {
                                "thoughtSignature": "<Signature A>"
                            }
                        },
                        "toolCallId": "function-call-1",
                        "toolName": "check_flight",
                        "type": "tool-call"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › Google Gemini thought signatures (OpenAI compatibility) › should handle parallel tool calls with signature only on first call
        #[tokio::test]
        async fn should_handle_parallel_tool_calls_with_signature_only_on_first_call() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({
                "tool_calls": [
                    {
                        "id": "function-call-paris",
                        "type": "function",
                        "function": { "name": "get_current_temperature", "arguments": "{\"location\":\"Paris\"}" },
                        "extra_content": { "google": { "thought_signature": "<Signature A>" } },
                    },
                    {
                        "id": "function-call-london",
                        "type": "function",
                        "function": { "name": "get_current_temperature", "arguments": "{\"location\":\"London\"}" },
                    },
                ],
            }))]);

            let result = model(&fetch)
                .do_generate(&call_options(json!({
                    "tools": [function_tool("get_current_temperature", json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } },
                        "required": ["location"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                    }))],
                })))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result)["content"],
                json!([
                    {
                        "input": "{\"location\":\"Paris\"}",
                        "providerMetadata": {
                            "test-provider": {
                                "thoughtSignature": "<Signature A>"
                            }
                        },
                        "toolCallId": "function-call-paris",
                        "toolName": "get_current_temperature",
                        "type": "tool-call"
                    },
                    {
                        "input": "{\"location\":\"London\"}",
                        "toolCallId": "function-call-london",
                        "toolName": "get_current_temperature",
                        "type": "tool-call"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › Google Gemini thought signatures (OpenAI compatibility) › should not include providerMetadata when no thought signature is present
        #[tokio::test]
        async fn should_not_include_provider_metadata_when_no_thought_signature_is_present() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": { "name": "some_tool", "arguments": "{\"param\":\"value\"}" },
                }],
            }))]);

            let result = model(&fetch)
                .do_generate(&call_options(json!({
                    "tools": [function_tool("some_tool", json!({
                        "type": "object",
                        "properties": { "param": { "type": "string" } },
                        "required": ["param"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                    }))],
                })))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result)["content"],
                json!([
                    {
                        "input": "{\"param\":\"value\"}",
                        "toolCallId": "call-1",
                        "toolName": "some_tool",
                        "type": "tool-call"
                    }
                ])
            );
        }
    }

    mod response_format {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should not send a response_format when response format is text
        #[tokio::test]
        async fn should_not_send_a_response_format_when_response_format_is_text() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"Spark\"}" }),
            )]);
            let model = direct_model(
                "gpt-4o-2024-08-06",
                OpenAICompatibleProviderSettings {
                    supports_structured_outputs: Some(false),
                    ..settings(&fetch)
                },
            );

            model
                .do_generate(&call_options(
                    json!({ "responseFormat": { "type": "text" } }),
                ))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-4o-2024-08-06"
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should forward json response format as "json_object" without schema
        #[tokio::test]
        async fn should_forward_json_response_format_as_json_object_without_schema() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"Spark\"}" }),
            )]);
            let model = provider(&fetch).chat("gpt-4o-2024-08-06");

            model
                .do_generate(&call_options(
                    json!({ "responseFormat": { "type": "json" } }),
                ))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-4o-2024-08-06",
                    "response_format": {
                        "type": "json_object"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should forward json response format as "json_object" and omit schema when structuredOutputs are disabled
        #[tokio::test]
        async fn should_forward_json_object_and_omit_schema_when_structured_outputs_are_disabled() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"Spark\"}" }),
            )]);
            let model = direct_model(
                "gpt-4o-2024-08-06",
                OpenAICompatibleProviderSettings {
                    supports_structured_outputs: Some(false),
                    ..settings(&fetch)
                },
            );

            let result = model
                .do_generate(&call_options(json!({
                    "responseFormat": { "type": "json", "schema": value_schema() },
                })))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-4o-2024-08-06",
                    "response_format": {
                        "type": "json_object"
                    }
                })
            );
            assert_eq!(
                result_json(&result)["warnings"],
                json!([
                    {
                        "details": "JSON response format schema is only supported with structuredOutputs",
                        "feature": "responseFormat",
                        "type": "unsupported"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should forward json response format as "json_object" and include schema when structuredOutputs are enabled
        #[tokio::test]
        async fn should_forward_json_object_and_include_schema_when_structured_outputs_are_enabled()
        {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"Spark\"}" }),
            )]);
            let model = direct_model(
                "gpt-4o-2024-08-06",
                OpenAICompatibleProviderSettings {
                    supports_structured_outputs: Some(true),
                    ..settings(&fetch)
                },
            );

            let result = model
                .do_generate(&call_options(json!({
                    "responseFormat": { "type": "json", "schema": value_schema() },
                })))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-4o-2024-08-06",
                    "response_format": {
                        "json_schema": {
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
                            "strict": true
                        },
                        "type": "json_schema"
                    }
                })
            );
            assert_eq!(result_json(&result)["warnings"], json!([]));
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should pass reasoningEffort setting from providerOptions
        #[tokio::test]
        async fn should_pass_reasoning_effort_setting_from_provider_options() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"test\"}" }),
            )]);
            let model = direct_model("gpt-5", settings(&fetch));

            model
                .do_generate(&call_options(json!({
                    "providerOptions": { "test-provider": { "reasoningEffort": "high" } },
                })))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-5",
                    "reasoning_effort": "high"
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should not duplicate reasoningEffort in request body
        #[tokio::test]
        async fn should_not_duplicate_reasoning_effort_in_request_body() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"test\"}" }),
            )]);
            let model = direct_model("gpt-5", settings(&fetch));

            model
                .do_generate(&call_options(json!({
                    "providerOptions": {
                        "test-provider": {
                            "reasoningEffort": "high",
                            "customOption": "should-be-included",
                        },
                    },
                })))
                .await
                .unwrap();
            let body = request_body(&fetch);

            assert_eq!(body["reasoning_effort"], "high");
            assert_eq!(body.get("reasoningEffort"), None);
            assert_eq!(body["customOption"], "should-be-included");
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should pass top-level reasoning as reasoning_effort
        #[tokio::test]
        async fn should_pass_top_level_reasoning_as_reasoning_effort() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "test" }))]);

            model(&fetch)
                .do_generate(&call_options(json!({ "reasoning": "medium" })))
                .await
                .unwrap();

            assert_eq!(request_body(&fetch)["reasoning_effort"], "medium");
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should pass top-level reasoning none as reasoning_effort
        #[tokio::test]
        async fn should_pass_top_level_reasoning_none_as_reasoning_effort() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "test" }))]);

            model(&fetch)
                .do_generate(&call_options(json!({ "reasoning": "none" })))
                .await
                .unwrap();

            assert_eq!(request_body(&fetch)["reasoning_effort"], "none");
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should prefer providerOptions reasoningEffort over top-level reasoning
        #[tokio::test]
        async fn should_prefer_provider_options_reasoning_effort_over_top_level_reasoning() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "test" }))]);
            let model = direct_model("gpt-5", settings(&fetch));

            model
                .do_generate(&call_options(json!({
                    "reasoning": "medium",
                    "providerOptions": { "test-provider": { "reasoningEffort": "high" } },
                })))
                .await
                .unwrap();

            assert_eq!(request_body(&fetch)["reasoning_effort"], "high");
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should pass textVerbosity setting from providerOptions
        #[tokio::test]
        async fn should_pass_text_verbosity_setting_from_provider_options() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"test\"}" }),
            )]);
            let model = direct_model("gpt-5", settings(&fetch));

            model
                .do_generate(&call_options(json!({
                    "providerOptions": { "test-provider": { "textVerbosity": "low" } },
                })))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-5",
                    "verbosity": "low"
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should not duplicate textVerbosity in request body
        #[tokio::test]
        async fn should_not_duplicate_text_verbosity_in_request_body() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"test\"}" }),
            )]);
            let model = direct_model("gpt-5", settings(&fetch));

            model
                .do_generate(&call_options(json!({
                    "providerOptions": {
                        "test-provider": {
                            "textVerbosity": "medium",
                            "customOption": "should-be-included",
                        },
                    },
                })))
                .await
                .unwrap();
            let body = request_body(&fetch);

            assert_eq!(body["verbosity"], "medium");
            assert_eq!(body.get("textVerbosity"), None);
            assert_eq!(body["customOption"], "should-be-included");
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should use json_schema & strict with responseFormat json when structuredOutputs are enabled
        #[tokio::test]
        async fn should_use_json_schema_and_strict_when_structured_outputs_are_enabled() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"Spark\"}" }),
            )]);
            let model = direct_model(
                "gpt-4o-2024-08-06",
                OpenAICompatibleProviderSettings {
                    supports_structured_outputs: Some(true),
                    ..settings(&fetch)
                },
            );

            model
                .do_generate(&call_options(json!({
                    "responseFormat": { "type": "json", "schema": value_schema() },
                })))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-4o-2024-08-06",
                    "response_format": {
                        "json_schema": {
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
                            "strict": true
                        },
                        "type": "json_schema"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should set name & description with responseFormat json when structuredOutputs are enabled
        #[tokio::test]
        async fn should_set_name_and_description_when_structured_outputs_are_enabled() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"Spark\"}" }),
            )]);
            let model = direct_model(
                "gpt-4o-2024-08-06",
                OpenAICompatibleProviderSettings {
                    supports_structured_outputs: Some(true),
                    ..settings(&fetch)
                },
            );

            model
                .do_generate(&call_options(json!({
                    "responseFormat": {
                        "type": "json",
                        "name": "test-name",
                        "description": "test description",
                        "schema": value_schema(),
                    },
                })))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-4o-2024-08-06",
                    "response_format": {
                        "json_schema": {
                            "description": "test description",
                            "name": "test-name",
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
                            "strict": true
                        },
                        "type": "json_schema"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should send strict: false when strictJsonSchema is explicitly disabled
        #[tokio::test]
        async fn should_send_strict_false_when_strict_json_schema_is_explicitly_disabled() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"Spark\"}" }),
            )]);
            let model = direct_model(
                "gpt-4o-2024-08-06",
                OpenAICompatibleProviderSettings {
                    supports_structured_outputs: Some(true),
                    ..settings(&fetch)
                },
            );

            model
                .do_generate(&call_options(json!({
                    "responseFormat": {
                        "type": "json",
                        "name": "test-name",
                        "description": "test description",
                        "schema": value_schema(),
                    },
                    "providerOptions": { "test-provider": { "strictJsonSchema": false } },
                })))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-4o-2024-08-06",
                    "response_format": {
                        "json_schema": {
                            "description": "test description",
                            "name": "test-name",
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
                            "strict": false
                        },
                        "type": "json_schema"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › response format › should allow for undefined schema with responseFormat json when structuredOutputs are enabled
        #[tokio::test]
        async fn should_allow_for_undefined_schema_when_structured_outputs_are_enabled() {
            let fetch = MockFetch::new(vec![prepare_json_response(
                json!({ "content": "{\"value\":\"Spark\"}" }),
            )]);
            let model = direct_model(
                "gpt-4o-2024-08-06",
                OpenAICompatibleProviderSettings {
                    supports_structured_outputs: Some(true),
                    ..settings(&fetch)
                },
            );

            model
                .do_generate(&call_options(json!({
                    "responseFormat": {
                        "type": "json",
                        "name": "test-name",
                        "description": "test description",
                    },
                })))
                .await
                .unwrap();

            assert_eq!(
                request_body(&fetch),
                json!({
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "gpt-4o-2024-08-06",
                    "response_format": {
                        "type": "json_object"
                    }
                })
            );
        }
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let fetch = MockFetch::new(vec![prepare_json_response(json!({ "content": "" }))]);

        let result = model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        // Upstream's `request.body` is the JSON string; aimux keeps the value.
        let mut expected = json!({
            "body": "{\"model\":\"grok-3\",\"messages\":[{\"role\":\"user\",\"content\":\"Hello\"}]}"
        });
        expected["body"] = serde_json::from_str(expected["body"].as_str().unwrap()).unwrap();
        assert_eq!(result_json(&result)["request"], expected);
    }

    mod usage_details {
        use super::*;

        fn usage_response(usage: Value) -> Value {
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hello!" },
                    "finish_reason": "stop",
                }],
                "usage": usage,
            })
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › usage details › should extract detailed token usage when available
        #[tokio::test]
        async fn should_extract_detailed_token_usage_when_available() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 30,
                    "total_tokens": 50,
                    "prompt_tokens_details": { "cached_tokens": 5 },
                    "completion_tokens_details": {
                        "reasoning_tokens": 10,
                        "accepted_prediction_tokens": 15,
                        "rejected_prediction_tokens": 5,
                    },
                },
            }))]);

            let result = model(&fetch)
                .do_generate(&call_options(json!({})))
                .await
                .unwrap();
            let result = result_json(&result);

            assert_eq!(
                result["usage"],
                json!({
                    "inputTokens": {
                        "cacheRead": 5,
                        "noCache": 15,
                        "total": 20
                    },
                    "outputTokens": {
                        "reasoning": 10,
                        "text": 20,
                        "total": 30
                    },
                    "raw": {
                        "completion_tokens": 30,
                        "completion_tokens_details": {
                            "accepted_prediction_tokens": 15,
                            "reasoning_tokens": 10,
                            "rejected_prediction_tokens": 5
                        },
                        "prompt_tokens": 20,
                        "prompt_tokens_details": {
                            "cached_tokens": 5
                        },
                        "total_tokens": 50
                    }
                })
            );
            assert_eq!(
                result["providerMetadata"],
                json!({
                    "test-provider": {
                        "acceptedPredictionTokens": 15,
                        "rejectedPredictionTokens": 5
                    }
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › usage details › should handle missing token details
        #[tokio::test]
        async fn should_handle_missing_token_details() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({
                "usage": { "prompt_tokens": 20, "completion_tokens": 30 },
            }))]);

            let result = model(&fetch)
                .do_generate(&call_options(json!({})))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result)["providerMetadata"]["test-provider"],
                json!({})
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › usage details › should handle partial token details
        #[tokio::test]
        async fn should_handle_partial_token_details() {
            let fetch = MockFetch::new(vec![prepare_json_response(json!({
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 30,
                    "total_tokens": 50,
                    "prompt_tokens_details": { "cached_tokens": 5 },
                    "completion_tokens_details": { "reasoning_tokens": 10 },
                },
            }))]);

            let result = model(&fetch)
                .do_generate(&call_options(json!({})))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result)["usage"],
                json!({
                    "inputTokens": {
                        "cacheRead": 5,
                        "noCache": 15,
                        "total": 20
                    },
                    "outputTokens": {
                        "reasoning": 10,
                        "text": 20,
                        "total": 30
                    },
                    "raw": {
                        "completion_tokens": 30,
                        "completion_tokens_details": {
                            "reasoning_tokens": 10
                        },
                        "prompt_tokens": 20,
                        "prompt_tokens_details": {
                            "cached_tokens": 5
                        },
                        "total_tokens": 50
                    }
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › usage details › should preserve extra usage fields from provider-specific responses
        #[tokio::test]
        async fn should_preserve_extra_usage_fields_from_provider_specific_responses() {
            let fetch = json_mock(usage_response(json!({
                "prompt_tokens": 18,
                "completion_tokens": 439,
                "total_tokens": 457,
                "queue_time": 0.061348671,
                "prompt_time": 0.000211569,
                "completion_time": 0.798181818,
                "total_time": 0.798393387,
            })));

            let result = model(&fetch)
                .do_generate(&call_options(json!({})))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result)["usage"]["raw"],
                json!({
                    "completion_time": 0.798181818,
                    "completion_tokens": 439,
                    "prompt_time": 0.000211569,
                    "prompt_tokens": 18,
                    "queue_time": 0.061348671,
                    "total_time": 0.798393387,
                    "total_tokens": 457
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doGenerate › usage details › should preserve extra usage fields nested inside token details
        #[tokio::test]
        async fn should_preserve_extra_usage_fields_nested_inside_token_details() {
            let fetch = json_mock(usage_response(json!({
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150,
                "prompt_tokens_details": { "cached_tokens": 80, "cache_type": "ephemeral" },
                "completion_tokens_details": { "reasoning_tokens": 10, "provider_specific_detail": 7 },
            })));

            let result = model(&fetch)
                .do_generate(&call_options(json!({})))
                .await
                .unwrap();

            assert_eq!(
                result_json(&result)["usage"]["raw"],
                json!({
                    "completion_tokens": 50,
                    "completion_tokens_details": {
                        "provider_specific_detail": 7,
                        "reasoning_tokens": 10
                    },
                    "prompt_tokens": 100,
                    "prompt_tokens_details": {
                        "cache_type": "ephemeral",
                        "cached_tokens": 80
                    },
                    "total_tokens": 150
                })
            );
        }
    }
}

mod do_stream {
    use super::*;

    fn no_raw_chunks() -> Value {
        json!({ "includeRawChunks": false })
    }

    /// `{ ...no_raw_chunks(), ...extra }`.
    fn stream_options(extra: Value) -> CallOptions {
        let mut options = no_raw_chunks();
        options
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        call_options(options)
    }

    /// `tools: [{ type: 'function', name: 'test-tool', inputSchema: { value } }]`.
    fn test_tool_options() -> CallOptions {
        stream_options(json!({ "tools": [function_tool("test-tool", value_schema())] }))
    }

    mod text_fixture {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › text (fixture) › should stream text content
        #[tokio::test]
        async fn should_stream_text_content() {
            let fetch = MockFetch::new(vec![prepare_chunks_fixture_response("xai-text", &[])]);

            let result = model(&fetch)
                .do_stream(&stream_options(json!({})))
                .await
                .unwrap();

            let expected: Value =
                serde_json::from_str(include_str!("snapshots/do_stream_text_fixture.json"))
                    .unwrap();
            assert_eq!(Value::Array(stream_parts(result).await), expected);
        }
    }

    mod tool_call_fixture {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › tool call (fixture) › should stream tool call content
        #[tokio::test]
        async fn should_stream_tool_call_content() {
            let fetch = MockFetch::new(vec![prepare_chunks_fixture_response("xai-tool-call", &[])]);

            let result = model(&fetch)
                .do_stream(&stream_options(json!({})))
                .await
                .unwrap();

            let expected: Value =
                serde_json::from_str(include_str!("snapshots/do_stream_tool_call_fixture.json"))
                    .unwrap();
            assert_eq!(Value::Array(stream_parts(result).await), expected);
        }
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let fetch = MockFetch::new(vec![prepare_chunks_fixture_response(
            "xai-text",
            &[("test-header", "test-value")],
        )]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(result.response.unwrap().headers).unwrap(),
            json!({
                "cache-control": "no-cache",
                "connection": "keep-alive",
                "content-type": "text/event-stream",
                "test-header": "test-value"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should report an error when $scenario without a finish reason
    #[tokio::test]
    async fn should_report_an_error_when_scenario_without_a_finish_reason() {
        let scenarios: [(&str, &[&str]); 2] = [
            ("the connection closes", &[]),
            ("[DONE] is received", &["data: [DONE]\n\n"]),
        ];
        for (scenario, final_chunks) in scenarios {
            let mut chunks = data(&[
                r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1702657020,"model":"grok-3","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
                r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1702657020,"model":"grok-3","choices":[{"index":0,"delta":{"content":" World"},"finish_reason":null}]}"#,
            ]);
            chunks.extend(final_chunks.iter().map(|chunk| (*chunk).to_string()));
            let fetch = MockFetch::new(vec![stream_chunks(&chunks, &[])]);

            let result = model(&fetch)
                .do_stream(&stream_options(json!({})))
                .await
                .unwrap();
            let events = stream_parts(result).await;

            assert_eq!(
                of_type(&events, "text-delta"),
                [
                    &json!({ "type": "text-delta", "delta": "Hello", "id": "txt-0" }),
                    &json!({ "type": "text-delta", "delta": " World", "id": "txt-0" }),
                ],
                "{scenario}"
            );
            let errors = of_type(&events, "error");
            assert_eq!(errors.len(), 1, "{scenario}");
            assert_eq!(
                errors[0]["error"]["name"], "AI_InvalidResponseDataError",
                "{scenario}"
            );
            assert_match_object(
                &errors[0]["error"],
                &json!({ "message": "Response stream ended without a finish reason." }),
            );
            let finishes = of_type(&events, "finish");
            assert_eq!(finishes.len(), 1, "{scenario}");
            assert_eq!(finishes[0]["type"], "finish", "{scenario}");
            assert_eq!(
                finishes[0]["finishReason"],
                json!({ "unified": "error" }),
                "{scenario}"
            );
        }
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should handle empty string role in delta chunks
    #[tokio::test]
    async fn should_handle_empty_string_role_in_delta_chunks() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1702657020,"model":"grok-3","choices":[{"index":0,"delta":{"role":"","content":"Hello"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1702657020,"model":"grok-3","choices":[{"index":0,"delta":{"role":"","content":" World"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1702657020,"model":"grok-3","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();
        let events = stream_parts(result).await;

        assert_eq!(
            of_type(&events, "text-delta"),
            [
                &json!({ "type": "text-delta", "delta": "Hello", "id": "txt-0" }),
                &json!({ "type": "text-delta", "delta": " World", "id": "txt-0" }),
            ]
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should respect the includeUsage option
    #[tokio::test]
    async fn should_respect_the_include_usage_option() {
        let fetch = MockFetch::new(vec![prepare_stream_response(
            &["Hello", ", ", "World!"],
            "stop",
        )]);
        let model = direct_model(
            "gpt-4o-2024-08-06",
            OpenAICompatibleProviderSettings {
                include_usage: Some(true),
                ..settings(&fetch)
            },
        );

        model.do_stream(&stream_options(json!({}))).await.unwrap();
        let body = request_body(&fetch);

        assert_eq!(body["stream"], true);
        assert_eq!(
            body["stream_options"],
            json!({
                "include_usage": true
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream reasoning content before text deltas
    #[tokio::test]
    async fn should_stream_reasoning_content_before_text_deltas() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":"", "reasoning_content":"Let me think"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"content":"", "reasoning_content":" about this"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"content":"Here's"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"content":" my response"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","system_fingerprint":"fp_10c08bf97d","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":439}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                    "modelId": "grok-3",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "reasoning-0",
                    "type": "reasoning-start"
                },
                {
                    "delta": "Let me think",
                    "id": "reasoning-0",
                    "type": "reasoning-delta"
                },
                {
                    "delta": " about this",
                    "id": "reasoning-0",
                    "type": "reasoning-delta"
                },
                {
                    "id": "reasoning-0",
                    "type": "reasoning-end"
                },
                {
                    "id": "txt-0",
                    "type": "text-start"
                },
                {
                    "delta": "Here's",
                    "id": "txt-0",
                    "type": "text-delta"
                },
                {
                    "delta": " my response",
                    "id": "txt-0",
                    "type": "text-delta"
                },
                {
                    "id": "txt-0",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "stop",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 18,
                            "total": 18
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 439,
                            "total": 439
                        },
                        "raw": {
                            "completion_tokens": 439,
                            "prompt_tokens": 18
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should keep reasoning active when deltas include empty tool calls
    #[tokio::test]
    async fn should_keep_reasoning_active_when_deltas_include_empty_tool_calls() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning_content":"Think ","tool_calls":[]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","model":"test-model","choices":[{"index":0,"delta":{"content":"","reasoning_content":"more...","tool_calls":[]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","model":"test-model","choices":[{"index":0,"delta":{"content":"Hello","reasoning_content":"","tool_calls":[]},"finish_reason":"stop"}]}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();
        let events = stream_parts(result).await;

        assert_eq!(
            events
                .iter()
                .filter(|event| event["type"]
                    .as_str()
                    .is_some_and(|kind| kind.starts_with("reasoning-")))
                .collect::<Vec<_>>(),
            [
                &json!({ "type": "reasoning-start", "id": "reasoning-0" }),
                &json!({ "type": "reasoning-delta", "id": "reasoning-0", "delta": "Think " }),
                &json!({ "type": "reasoning-delta", "id": "reasoning-0", "delta": "more..." }),
                &json!({ "type": "reasoning-end", "id": "reasoning-0" }),
            ]
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream reasoning from reasoning field when reasoning_content is not provided
    #[tokio::test]
    async fn should_stream_reasoning_from_reasoning_field_when_reasoning_content_is_not_provided() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":"", "reasoning":"Let me consider"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"content":"", "reasoning":" this carefully"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"content":"My answer is"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"content":" correct"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","system_fingerprint":"fp_10c08bf97d","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":439}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                    "modelId": "grok-3",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "reasoning-0",
                    "type": "reasoning-start"
                },
                {
                    "delta": "Let me consider",
                    "id": "reasoning-0",
                    "type": "reasoning-delta"
                },
                {
                    "delta": " this carefully",
                    "id": "reasoning-0",
                    "type": "reasoning-delta"
                },
                {
                    "id": "reasoning-0",
                    "type": "reasoning-end"
                },
                {
                    "id": "txt-0",
                    "type": "text-start"
                },
                {
                    "delta": "My answer is",
                    "id": "txt-0",
                    "type": "text-delta"
                },
                {
                    "delta": " correct",
                    "id": "txt-0",
                    "type": "text-delta"
                },
                {
                    "id": "txt-0",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "stop",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 18,
                            "total": 18
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 439,
                            "total": 439
                        },
                        "raw": {
                            "completion_tokens": 439,
                            "prompt_tokens": 18
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should prefer reasoning_content over reasoning field in streaming when both are provided
    #[tokio::test]
    async fn should_prefer_reasoning_content_over_reasoning_field_in_streaming() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":"", "reasoning_content":"From reasoning_content", "reasoning":"From reasoning"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"content":"Final response"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","system_fingerprint":"fp_10c08bf97d","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":439}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                    "modelId": "grok-3",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "reasoning-0",
                    "type": "reasoning-start"
                },
                {
                    "delta": "From reasoning_content",
                    "id": "reasoning-0",
                    "type": "reasoning-delta"
                },
                {
                    "id": "reasoning-0",
                    "type": "reasoning-end"
                },
                {
                    "id": "txt-0",
                    "type": "text-start"
                },
                {
                    "delta": "Final response",
                    "id": "txt-0",
                    "type": "text-delta"
                },
                {
                    "id": "txt-0",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "stop",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 18,
                            "total": 18
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 439,
                            "total": 439
                        },
                        "raw": {
                            "completion_tokens": 439,
                            "prompt_tokens": 18
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream tool deltas
    #[tokio::test]
    async fn should_stream_tool_deltas() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":""}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\""}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"value"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\""}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"Spark"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"le"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" Day"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","system_fingerprint":"fp_10c08bf97d","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"queue_time":0.061348671,"prompt_tokens":18,"prompt_time":0.000211569,"completion_tokens":439,"completion_time":0.798181818,"total_tokens":457,"total_time":0.798393387}}"#,
        ]);

        let result = model(&fetch).do_stream(&test_tool_options()).await.unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                    "modelId": "grok-3",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "toolName": "test-tool",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "value",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\":\"",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "Spark",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "le",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": " Day",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\"}",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"value\":\"Sparkle Day\"}",
                    "toolCallId": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "toolName": "test-tool",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_calls",
                        "unified": "tool-calls"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 18,
                            "total": 18
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 439,
                            "total": 439
                        },
                        "raw": {
                            "completion_time": 0.798181818,
                            "completion_tokens": 439,
                            "prompt_time": 0.000211569,
                            "prompt_tokens": 18,
                            "queue_time": 0.061348671,
                            "total_time": 0.798393387,
                            "total_tokens": 457
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream tool deltas when function.name arrives in a later chunk
    #[tokio::test]
    async fn should_stream_tool_deltas_when_function_name_arrives_in_a_later_chunk() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-late-name","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"role":"assistant","content":null},"finish_reason":null}]}"#,
            // First tool_calls delta carries id and (empty) arguments but no function.name
            r#"{"id":"chatcmpl-late-name","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_late","type":"function","function":{"arguments":""}}]},"finish_reason":null}]}"#,
            // function.name arrives in the next chunk together with the start of the arguments
            r#"{"id":"chatcmpl-late-name","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_late","type":"function","function":{"name":"test-tool","arguments":"{\""}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-late-name","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"value"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-late-name","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\"hi\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-late-name","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":18,"completion_tokens":10,"total_tokens":28}}"#,
        ]);

        let result = model(&fetch).do_stream(&test_tool_options()).await.unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chatcmpl-late-name",
                    "modelId": "grok-3",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "call_late",
                    "toolName": "test-tool",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"",
                    "id": "call_late",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "value",
                    "id": "call_late",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\":\"hi\"}",
                    "id": "call_late",
                    "type": "tool-input-delta"
                },
                {
                    "id": "call_late",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"value\":\"hi\"}",
                    "toolCallId": "call_late",
                    "toolName": "test-tool",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_calls",
                        "unified": "tool-calls"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 18,
                            "total": 18
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 10,
                            "total": 10
                        },
                        "raw": {
                            "completion_tokens": 10,
                            "prompt_tokens": 18,
                            "total_tokens": 28
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream a tool call whose index starts at one
    #[tokio::test]
    async fn should_stream_a_tool_call_whose_index_starts_at_one() {
        let fetch = MockFetch::new(vec![prepare_sse_fixture_response(
            "anthropic-fallback-tool-call",
        )]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({
                "tools": [function_tool("read_file", json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                }))],
            })))
            .await
            .unwrap();
        let parts = stream_parts(result).await;

        assert_eq!(
            of_type(&parts, "text-delta")
                .iter()
                .map(|part| part["delta"].as_str().unwrap())
                .collect::<String>(),
            "Reading it."
        );
        assert_eq!(
            of_type(&parts, "tool-call"),
            [&json!({
                "type": "tool-call",
                "toolCallId": "toolu_sanitized",
                "toolName": "read_file",
                "input": "{\"path\": \"a.txt\"}",
            })]
        );
        assert_eq!(parts.last().unwrap()["type"], "finish");
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should keep id-less tool calls distinct when the index is reused
    #[tokio::test]
    async fn should_keep_id_less_tool_calls_distinct_when_the_index_is_reused() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-reused-index","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"type":"function","function":{"name":"read_file","arguments":"{\"path\":\"p0\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-reused-index","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"write_file","arguments":"{\"path\":\"p1\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-reused-index","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"read_file","arguments":"{\"path\":\"p2\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-reused-index","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":18,"completion_tokens":10,"total_tokens":28}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();
        let parts = stream_parts(result).await;
        let tool_calls = of_type(&parts, "tool-call");

        assert_eq!(
            tool_calls
                .iter()
                .map(|call| json!({ "toolName": call["toolName"], "input": call["input"] }))
                .collect::<Vec<_>>(),
            [
                json!({ "toolName": "read_file", "input": "{\"path\":\"p0\"}" }),
                json!({ "toolName": "write_file", "input": "{\"path\":\"p1\"}" }),
                json!({ "toolName": "read_file", "input": "{\"path\":\"p2\"}" }),
            ]
        );
        assert!(
            tool_calls
                .iter()
                .all(|call| !call["toolCallId"].as_str().unwrap().trim().is_empty())
        );
        assert_eq!(
            tool_calls
                .iter()
                .map(|call| call["toolCallId"].as_str().unwrap())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should error when streamed tool call never receives a function.name
    #[tokio::test]
    async fn should_error_when_streamed_tool_call_never_receives_a_function_name() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-no-name","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"role":"assistant","content":null},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-no-name","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_missing","type":"function","function":{"arguments":"{}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-no-name","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":18,"completion_tokens":10,"total_tokens":28}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({
                "tools": [function_tool("test-tool", json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#",
                }))],
            })))
            .await
            .unwrap();
        let error = convert_stream_to_array(result).await.unwrap_err();

        // `[AI_InvalidResponseDataError: Expected 'function.name' to be a string.]`
        assert_eq!(
            error_json(&error),
            json!({
                "name": "AI_InvalidResponseDataError",
                "message": "Expected 'function.name' to be a string.",
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream tool call with thought signature from extra_content
    #[tokio::test]
    async fn should_stream_tool_call_with_thought_signature_from_extra_content() {
        let fetch = stream_mock(&[
            // First chunk with tool call start and thought signature in extra_content
            r#"{"id":"chatcmpl-gemini-thought","object":"chat.completion.chunk","created":1711357598,"model":"gemini-3-pro","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"function-call-1","type":"function","function":{"name":"check_flight","arguments":""},"extra_content":{"google":{"thought_signature":"<Signature A>"}}}]},"finish_reason":null}]}"#,
            // Subsequent chunks with arguments
            r#"{"id":"chatcmpl-gemini-thought","object":"chat.completion.chunk","created":1711357598,"model":"gemini-3-pro","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"flight\":"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-gemini-thought","object":"chat.completion.chunk","created":1711357598,"model":"gemini-3-pro","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"AA100\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-gemini-thought","object":"chat.completion.chunk","created":1711357598,"model":"gemini-3-pro","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({
                "tools": [function_tool("check_flight", json!({
                    "type": "object",
                    "properties": { "flight": { "type": "string" } },
                    "required": ["flight"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#",
                }))],
            })))
            .await
            .unwrap();
        let result = stream_parts(result).await;

        assert_match_object(
            find(&result, "tool-call"),
            &json!({
                "type": "tool-call",
                "toolCallId": "function-call-1",
                "toolName": "check_flight",
                "input": "{\"flight\":\"AA100\"}",
                "providerMetadata": { "test-provider": { "thoughtSignature": "<Signature A>" } },
            }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream parallel tool calls with signature only on first call
    #[tokio::test]
    async fn should_stream_parallel_tool_calls_with_signature_only_on_first_call() {
        let fetch = stream_mock(&[
            // First chunk with two tool calls - only first has thought signature
            r#"{"id":"chatcmpl-gemini-parallel","object":"chat.completion.chunk","created":1711357598,"model":"gemini-3-pro","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call-paris","type":"function","function":{"name":"get_weather","arguments":""},"extra_content":{"google":{"thought_signature":"<Signature A>"}}},{"index":1,"id":"call-london","type":"function","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#,
            // Arguments for first call
            r#"{"id":"chatcmpl-gemini-parallel","object":"chat.completion.chunk","created":1711357598,"model":"gemini-3-pro","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"location\":\"Paris\"}"}}]},"finish_reason":null}]}"#,
            // Arguments for second call
            r#"{"id":"chatcmpl-gemini-parallel","object":"chat.completion.chunk","created":1711357598,"model":"gemini-3-pro","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"location\":\"London\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-gemini-parallel","object":"chat.completion.chunk","created":1711357598,"model":"gemini-3-pro","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({
                "tools": [function_tool("get_weather", json!({
                    "type": "object",
                    "properties": { "location": { "type": "string" } },
                    "required": ["location"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#",
                }))],
            })))
            .await
            .unwrap();
        let result = stream_parts(result).await;
        let tool_call_events = of_type(&result, "tool-call");

        assert_eq!(tool_call_events.len(), 2);
        // First tool call should have thought signature
        assert_match_object(
            tool_call_events[0],
            &json!({
                "type": "tool-call",
                "toolCallId": "call-paris",
                "toolName": "get_weather",
                "providerMetadata": { "test-provider": { "thoughtSignature": "<Signature A>" } },
            }),
        );
        // Second tool call should NOT have thought signature
        assert_match_object(
            tool_call_events[1],
            &json!({
                "type": "tool-call",
                "toolCallId": "call-london",
                "toolName": "get_weather",
            }),
        );
        assert_eq!(tool_call_events[1].get("providerMetadata"), None);
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream tool call deltas when tool call arguments are passed in the first chunk
    #[tokio::test]
    async fn should_stream_tool_call_deltas_when_tool_call_arguments_are_passed_in_the_first_chunk()
    {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":"{\""}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"va"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"lue"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\""}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"Spark"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"le"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" Day"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","system_fingerprint":"fp_10c08bf97d","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"queue_time":0.061348671,"prompt_tokens":18,"prompt_time":0.000211569,"completion_tokens":439,"completion_time":0.798181818,"total_tokens":457,"total_time":0.798393387}}"#,
        ]);

        let result = model(&fetch).do_stream(&test_tool_options()).await.unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                    "modelId": "grok-3",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "toolName": "test-tool",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "va",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "lue",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\":\"",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "Spark",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "le",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": " Day",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "\"}",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"value\":\"Sparkle Day\"}",
                    "toolCallId": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "toolName": "test-tool",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_calls",
                        "unified": "tool-calls"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 18,
                            "total": 18
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 439,
                            "total": 439
                        },
                        "raw": {
                            "completion_time": 0.798181818,
                            "completion_tokens": 439,
                            "prompt_time": 0.000211569,
                            "prompt_tokens": 18,
                            "queue_time": 0.061348671,
                            "total_time": 0.798393387,
                            "total_tokens": 457
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should not duplicate tool calls when there is an additional empty chunk after the tool call has been completed
    #[tokio::test]
    async fn should_not_duplicate_tool_calls_when_there_is_an_additional_empty_chunk() {
        let fetch = stream_mock(&[
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"role":"assistant","content":""},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":226,"completion_tokens":0}}"#,
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"id":"chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa","type":"function","index":0,"function":{"name":"searchGoogle"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":233,"completion_tokens":7}}"#,
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\": \""}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":241,"completion_tokens":15}}"#,
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"latest"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":242,"completion_tokens":16}}"#,
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" news"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":243,"completion_tokens":17}}"#,
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" on"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":244,"completion_tokens":18}}"#,
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" ai\"}"}}]},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":226,"total_tokens":245,"completion_tokens":19}}"#,
            // empty arguments chunk after the tool call has already been finished:
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"logprobs":null,"finish_reason":"tool_calls","stop_reason":128008}],"usage":{"prompt_tokens":226,"total_tokens":246,"completion_tokens":20}}"#,
            r#"{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[],"usage":{"prompt_tokens":226,"total_tokens":246,"completion_tokens":20}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({
                "tools": [function_tool("searchGoogle", json!({
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#",
                }))],
            })))
            .await
            .unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chat-2267f7e2910a4254bac0650ba74cfc1c",
                    "modelId": "meta/llama-3.1-8b-instruct:fp8",
                    "timestamp": "2024-12-02T17:57:21.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "toolName": "searchGoogle",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"query\": \"",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "latest",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": " news",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": " on",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": " ai\"}",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "delta": "",
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-delta"
                },
                {
                    "id": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"query\": \"latest news on ai\"}",
                    "toolCallId": "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa",
                    "toolName": "searchGoogle",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_calls",
                        "unified": "tool-calls"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 226,
                            "total": 226
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 20,
                            "total": 20
                        },
                        "raw": {
                            "completion_tokens": 20,
                            "prompt_tokens": 226,
                            "total_tokens": 246
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream tool call that is sent in one chunk
    #[tokio::test]
    async fn should_stream_tool_call_that_is_sent_in_one_chunk() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":"{\"value\":\"Sparkle Day\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","system_fingerprint":"fp_10c08bf97d","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"queue_time":0.061348671,"prompt_tokens":18,"prompt_time":0.000211569,"completion_tokens":439,"completion_time":0.798181818,"total_tokens":457,"total_time":0.798393387}}"#,
        ]);

        let result = model(&fetch).do_stream(&test_tool_options()).await.unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                    "modelId": "grok-3",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "toolName": "test-tool",
                    "type": "tool-input-start"
                },
                {
                    "delta": "{\"value\":\"Sparkle Day\"}",
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-delta"
                },
                {
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-end"
                },
                {
                    "input": "{\"value\":\"Sparkle Day\"}",
                    "toolCallId": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "toolName": "test-tool",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_calls",
                        "unified": "tool-calls"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 18,
                            "total": 18
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 439,
                            "total": 439
                        },
                        "raw": {
                            "completion_time": 0.798181818,
                            "completion_tokens": 439,
                            "prompt_time": 0.000211569,
                            "prompt_tokens": 18,
                            "queue_time": 0.061348671,
                            "total_time": 0.798393387,
                            "total_tokens": 457
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should stream empty tool call that is sent in one chunk
    #[tokio::test]
    async fn should_stream_empty_tool_call_that_is_sent_in_one_chunk() {
        let fetch = stream_mock(&[
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1711357598,"model":"grok-3","system_fingerprint":"fp_3bc1b5746c","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":""}}]},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798","object":"chat.completion.chunk","created":1729171479,"model":"grok-3","system_fingerprint":"fp_10c08bf97d","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"queue_time":0.061348671,"prompt_tokens":18,"prompt_time":0.000211569,"completion_tokens":439,"completion_time":0.798181818,"total_tokens":457,"total_time":0.798393387}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({
                "tools": [function_tool("test-tool", json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#",
                }))],
            })))
            .await
            .unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "id": "chatcmpl-e7f8e220-656c-4455-a132-dacfc1370798",
                    "modelId": "grok-3",
                    "timestamp": "2024-03-25T09:06:38.000Z",
                    "type": "response-metadata"
                },
                {
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "toolName": "test-tool",
                    "type": "tool-input-start"
                },
                {
                    "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "type": "tool-input-end"
                },
                {
                    "input": "",
                    "toolCallId": "call_O17Uplv4lJvD6DVdIvFFeRMw",
                    "toolName": "test-tool",
                    "type": "tool-call"
                },
                {
                    "finishReason": {
                        "raw": "tool_calls",
                        "unified": "tool-calls"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 0,
                            "noCache": 18,
                            "total": 18
                        },
                        "outputTokens": {
                            "reasoning": 0,
                            "text": 439,
                            "total": 439
                        },
                        "raw": {
                            "completion_time": 0.798181818,
                            "completion_tokens": 439,
                            "prompt_time": 0.000211569,
                            "prompt_tokens": 18,
                            "queue_time": 0.061348671,
                            "total_time": 0.798393387,
                            "total_tokens": 457
                        }
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should preserve structured error stream parts
    #[tokio::test]
    async fn should_preserve_structured_error_stream_parts() {
        let fetch = stream_mock(&[
            r#"{"error": {"message": "Context length exceeded", "code": "CONTEXT_LENGTH_EXCEEDED"}}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "error": {
                        "code": "CONTEXT_LENGTH_EXCEEDED",
                        "message": "Context length exceeded"
                    },
                    "type": "error"
                },
                {
                    "finishReason": {
                        "unified": "error"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should handle unparsable stream parts
    #[tokio::test]
    async fn should_handle_unparsable_stream_parts() {
        let fetch = stream_mock(&["{unparsable}"]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();
        let mut parts = stream_parts(result).await;

        let mut expected = json!([
            {
                "type": "stream-start",
                "warnings": []
            },
            {
                "error": {
                    "name": "AI_JSONParseError",
                    "message": "JSON parsing failed: Text: {unparsable}.\nError message: SyntaxError: Expected property name or '}' in JSON at position 1 (line 1 column 2)"
                },
                "type": "error"
            },
            {
                "finishReason": {
                    "unified": "error"
                },
                "providerMetadata": {
                    "test-provider": {}
                },
                "type": "finish",
                "usage": {
                    "inputTokens": {},
                    "outputTokens": {}
                }
            }
        ]);
        // The `SyntaxError` text after "Error message: " is the JS runtime's
        // `JSON.parse` message; only the part before it is compared.
        let runtime_specific = |parts: &mut Vec<Value>| {
            let message = parts[1]["error"]["message"].take();
            let message = message.as_str().unwrap_or_default();
            message
                .split_once("\nError message: ")
                .map_or(message.to_string(), |(prefix, _)| prefix.to_string())
        };
        let message = runtime_specific(&mut parts);
        let expected_message = runtime_specific(expected.as_array_mut().unwrap());
        assert_eq!(Value::Array(parts), expected);
        assert_eq!(message, expected_message);
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should pass the messages and the model
    #[tokio::test]
    async fn should_pass_the_messages_and_the_model() {
        let fetch = MockFetch::new(vec![prepare_stream_response(&[], "stop")]);

        model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3",
                "stream": true
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should pass video input as video_url in doStream
    #[tokio::test]
    async fn should_pass_video_input_as_video_url_in_do_stream() {
        let fetch = MockFetch::new(vec![prepare_stream_response(&[], "stop")]);

        model(&fetch)
            .do_stream(&stream_options(json!({
                "prompt": [{
                    "role": "user",
                    "content": [{
                        "type": "file",
                        "data": { "type": "url", "url": "https://example.com/video.mp4" },
                        "mediaType": "video/mp4",
                    }],
                }],
            })))
            .await
            .unwrap();

        assert_match_object(
            &request_body(&fetch),
            &json!({
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "video_url",
                        "video_url": { "url": "https://example.com/video.mp4" },
                    }],
                }],
                "stream": true,
            }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let fetch = MockFetch::new(vec![prepare_stream_response(&[], "stop")]);
        let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
            base_url: "https://my.api.com/v1".into(),
            headers: Some(headers(&[
                ("Authorization", "Bearer test-api-key"),
                ("Custom-Provider-Header", "provider-header-value"),
            ])),
            ..settings(&fetch)
        })
        .unwrap();

        provider
            .chat("grok-3")
            .do_stream(&stream_options(json!({
                "headers": { "Custom-Request-Header": "request-header-value" },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_headers(&fetch.seen()[0]),
            json!({
                "authorization": "Bearer test-api-key",
                "content-type": "application/json",
                "custom-provider-header": "provider-header-value",
                "custom-request-header": "request-header-value"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should include provider-specific options
    #[tokio::test]
    async fn should_include_provider_specific_options() {
        let fetch = MockFetch::new(vec![prepare_stream_response(&[], "stop")]);

        provider(&fetch)
            .chat("grok-3")
            .do_stream(&stream_options(json!({
                "providerOptions": { "test-provider": { "someCustomOption": "test-value" } },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3",
                "someCustomOption": "test-value",
                "stream": true
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should not include provider-specific options for different provider
    #[tokio::test]
    async fn should_not_include_provider_specific_options_for_different_provider() {
        let fetch = MockFetch::new(vec![prepare_stream_response(&[], "stop")]);

        provider(&fetch)
            .chat("grok-3")
            .do_stream(&stream_options(json!({
                "providerOptions": { "notThisProviderName": { "someCustomOption": "test-value" } },
            })))
            .await
            .unwrap();

        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "grok-3",
                "stream": true
            })
        );
    }

    mod camel_case_provider_options {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › camelCase provider options › should accept camelCase provider options key for hyphenated provider name
        #[tokio::test]
        async fn should_accept_camel_case_provider_options_key_for_hyphenated_provider_name() {
            let fetch = MockFetch::new(vec![prepare_stream_response(&[], "stop")]);

            provider(&fetch)
                .chat("grok-3")
                .do_stream(&stream_options(json!({
                    "providerOptions": { "testProvider": { "someCustomOption": "test-value" } },
                })))
                .await
                .unwrap();

            assert_match_object(
                &request_body(&fetch),
                &json!({ "someCustomOption": "test-value" }),
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › camelCase provider options › should prefer camelCase options over raw-name options
        #[tokio::test]
        async fn should_prefer_camel_case_options_over_raw_name_options() {
            let fetch = MockFetch::new(vec![prepare_stream_response(&[], "stop")]);

            provider(&fetch)
                .chat("grok-3")
                .do_stream(&stream_options(json!({
                    "providerOptions": {
                        "test-provider": { "someCustomOption": "raw-value" },
                        "testProvider": { "someCustomOption": "camel-value" },
                    },
                })))
                .await
                .unwrap();

            assert_match_object(
                &request_body(&fetch),
                &json!({ "someCustomOption": "camel-value" }),
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › camelCase provider options › should emit deprecated warning when raw provider options key is used
        #[tokio::test]
        async fn should_emit_deprecated_warning_when_raw_provider_options_key_is_used() {
            let fetch = MockFetch::new(vec![prepare_stream_response(&["Hello"], "stop")]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_stream(&stream_options(json!({
                    "providerOptions": { "test-provider": { "reasoningEffort": "high" } },
                })))
                .await
                .unwrap();
            let parts = stream_parts(result).await;

            assert_eq!(
                find(&parts, "stream-start")["warnings"],
                json!([
                    {
                        "message": "Use 'testProvider' instead.",
                        "setting": "providerOptions key 'test-provider'",
                        "type": "deprecated"
                    }
                ])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › camelCase provider options › should not emit deprecated warning when camelCase provider options key is used
        #[tokio::test]
        async fn should_not_emit_deprecated_warning_when_camel_case_provider_options_key_is_used() {
            let fetch = MockFetch::new(vec![prepare_stream_response(&["Hello"], "stop")]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_stream(&stream_options(json!({
                    "providerOptions": { "testProvider": { "reasoningEffort": "high" } },
                })))
                .await
                .unwrap();
            let parts = stream_parts(result).await;

            assert_eq!(find(&parts, "stream-start")["warnings"], json!([]));
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › camelCase provider options › should use camelCase metadata key in finish event when camelCase options are used
        #[tokio::test]
        async fn should_use_camel_case_metadata_key_in_finish_event_when_camel_case_options_are_used()
         {
            let fetch = MockFetch::new(vec![prepare_stream_response(&["Hello"], "stop")]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_stream(&stream_options(json!({
                    "providerOptions": { "testProvider": {} },
                })))
                .await
                .unwrap();
            let parts = stream_parts(result).await;
            let finish_part = find(&parts, "finish");

            assert!(
                finish_part["providerMetadata"]
                    .get("testProvider")
                    .is_some()
            );
            assert!(
                finish_part["providerMetadata"]
                    .get("test-provider")
                    .is_none()
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › camelCase provider options › should use raw metadata key in finish event when raw options are used
        #[tokio::test]
        async fn should_use_raw_metadata_key_in_finish_event_when_raw_options_are_used() {
            let fetch = MockFetch::new(vec![prepare_stream_response(&["Hello"], "stop")]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_stream(&stream_options(json!({
                    "providerOptions": { "test-provider": {} },
                })))
                .await
                .unwrap();
            let parts = stream_parts(result).await;

            assert!(
                find(&parts, "finish")["providerMetadata"]
                    .get("test-provider")
                    .is_some()
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › camelCase provider options › should use raw metadata key in finish event when no provider options are passed
        #[tokio::test]
        async fn should_use_raw_metadata_key_in_finish_event_when_no_provider_options_are_passed() {
            let fetch = MockFetch::new(vec![prepare_stream_response(&["Hello"], "stop")]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_stream(&stream_options(json!({})))
                .await
                .unwrap();
            let parts = stream_parts(result).await;

            assert!(
                find(&parts, "finish")["providerMetadata"]
                    .get("test-provider")
                    .is_some()
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › camelCase provider options › should use camelCase metadata key for thought signatures in streamed tool calls
        #[tokio::test]
        async fn should_use_camel_case_metadata_key_for_thought_signatures_in_streamed_tool_calls()
        {
            let fetch = stream_mock(&[
                r#"{"id":"chat-id","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"test_tool","arguments":"{\"a\":1}"},"extra_content":{"google":{"thought_signature":"<Sig>"}}}]},"finish_reason":null}]}"#,
                r#"{"id":"chat-id","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
            ]);

            let result = provider(&fetch)
                .chat("grok-3")
                .do_stream(&stream_options(json!({
                    "providerOptions": { "testProvider": {} },
                })))
                .await
                .unwrap();
            let parts = stream_parts(result).await;

            assert_match_object(
                find(&parts, "tool-call"),
                &json!({
                    "type": "tool-call",
                    "providerMetadata": { "testProvider": { "thoughtSignature": "<Sig>" } },
                }),
            );
        }
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let fetch = MockFetch::new(vec![prepare_stream_response(&[], "stop")]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(result.request).unwrap(),
            json!({
                "body": {
                    "messages": [
                        {
                            "content": "Hello",
                            "role": "user"
                        }
                    ],
                    "model": "grok-3",
                    "stream": true
                }
            })
        );
    }

    mod usage_details_in_streaming {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › usage details in streaming › should extract detailed token usage from stream finish
        #[tokio::test]
        async fn should_extract_detailed_token_usage_from_stream_finish() {
            let fetch = stream_mock(&[
                r#"{"id":"chat-id","choices":[{"delta":{"content":"Hello"}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":30,"prompt_tokens_details":{"cached_tokens":5},"completion_tokens_details":{"reasoning_tokens":10,"accepted_prediction_tokens":15,"rejected_prediction_tokens":5}}}"#,
            ]);

            let result = model(&fetch)
                .do_stream(&stream_options(json!({})))
                .await
                .unwrap();
            let parts = stream_parts(result).await;

            assert_eq!(
                find(&parts, "finish"),
                &json!({
                    "finishReason": {
                        "raw": "stop",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "test-provider": {
                            "acceptedPredictionTokens": 15,
                            "rejectedPredictionTokens": 5
                        }
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 5,
                            "noCache": 15,
                            "total": 20
                        },
                        "outputTokens": {
                            "reasoning": 10,
                            "text": 20,
                            "total": 30
                        },
                        "raw": {
                            "completion_tokens": 30,
                            "completion_tokens_details": {
                                "accepted_prediction_tokens": 15,
                                "reasoning_tokens": 10,
                                "rejected_prediction_tokens": 5
                            },
                            "prompt_tokens": 20,
                            "prompt_tokens_details": {
                                "cached_tokens": 5
                            }
                        }
                    }
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › usage details in streaming › should handle missing token details in stream
        #[tokio::test]
        async fn should_handle_missing_token_details_in_stream() {
            let fetch = stream_mock(&[
                r#"{"id":"chat-id","choices":[{"delta":{"content":"Hello"}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":30}}"#,
            ]);

            let result = model(&fetch)
                .do_stream(&stream_options(json!({})))
                .await
                .unwrap();
            let parts = stream_parts(result).await;

            assert_eq!(
                find(&parts, "finish")["providerMetadata"]["test-provider"],
                json!({})
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › usage details in streaming › should handle partial token details in stream
        #[tokio::test]
        async fn should_handle_partial_token_details_in_stream() {
            let fetch = stream_mock(&[
                r#"{"id":"chat-id","choices":[{"delta":{"content":"Hello"}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":30,"total_tokens":50,"prompt_tokens_details":{"cached_tokens":5},"completion_tokens_details":{"reasoning_tokens":10}}}"#,
            ]);

            let result = model(&fetch)
                .do_stream(&stream_options(json!({})))
                .await
                .unwrap();
            let parts = stream_parts(result).await;

            assert_eq!(
                find(&parts, "finish"),
                &json!({
                    "finishReason": {
                        "raw": "stop",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {
                            "cacheRead": 5,
                            "noCache": 15,
                            "total": 20
                        },
                        "outputTokens": {
                            "reasoning": 10,
                            "text": 20,
                            "total": 30
                        },
                        "raw": {
                            "completion_tokens": 30,
                            "completion_tokens_details": {
                                "reasoning_tokens": 10
                            },
                            "prompt_tokens": 20,
                            "prompt_tokens_details": {
                                "cached_tokens": 5
                            },
                            "total_tokens": 50
                        }
                    }
                })
            );
        }
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › doStream › should extract response metadata after an empty placeholder chunk
    #[tokio::test]
    async fn should_extract_response_metadata_after_an_empty_placeholder_chunk() {
        let fetch = MockFetch::new(vec![prepare_chunks_fixture_response(
            "placeholder-first-chunk",
            &[],
        )]);

        let result = model(&fetch)
            .do_stream(&stream_options(json!({})))
            .await
            .unwrap();
        let events = stream_parts(result).await;

        assert_eq!(
            find(&events, "response-metadata"),
            &json!({
                "type": "response-metadata",
                "id": "chatcmpl-123",
                "modelId": "actual-model-name",
                // `new Date(1700000000 * 1000)`
                "timestamp": "2023-11-14T22:13:20.000Z",
            })
        );
    }
}

mod metadata_extraction {
    use super::*;

    /// The model of both tests: `metadataExtractor: testMetadataExtractor`.
    fn extractor_model(fetch: &Arc<MockFetch>) -> OpenAICompatibleChatModel {
        direct_model(
            "gpt-5",
            OpenAICompatibleProviderSettings {
                metadata_extractor: Some(Arc::new(TestMetadataExtractor)),
                ..settings(fetch)
            },
        )
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › metadata extraction › should process metadata from complete response
    #[tokio::test]
    async fn should_process_metadata_from_complete_response() {
        let fetch = json_mock(json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-5",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant" },
                "finish_reason": "stop",
            }],
            "test_field": "test_value",
        }));

        let result = extractor_model(&fetch)
            .do_generate(&call_options(json!({})))
            .await
            .unwrap();

        assert_eq!(
            result_json(&result)["providerMetadata"],
            json!({ "test-provider": { "value": "test_value" } })
        );
        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "gpt-5"
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › metadata extraction › should process metadata from streaming response
    #[tokio::test]
    async fn should_process_metadata_from_streaming_response() {
        let fetch = stream_mock(&[
            r#"{"choices":[{"delta":{"content":"Hello"}}]}"#,
            r#"{"choices":[{"finish_reason":"stop"}],"test_field":"test_value"}"#,
        ]);

        let result = extractor_model(&fetch)
            .do_stream(&call_options(json!({ "includeRawChunks": false })))
            .await
            .unwrap();
        let parts = stream_parts(result).await;

        assert_eq!(
            find(&parts, "finish")["providerMetadata"],
            json!({ "test-provider": { "value": "test_value" } })
        );
        assert_eq!(
            request_body(&fetch),
            json!({
                "messages": [
                    {
                        "content": "Hello",
                        "role": "user"
                    }
                ],
                "model": "gpt-5",
                "stream": true
            })
        );
    }
}

mod raw_chunks {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › raw chunks › should include raw chunks when includeRawChunks is true
    #[tokio::test]
    async fn should_include_raw_chunks_when_include_raw_chunks_is_true() {
        let fetch = stream_mock(&[
            r#"{"id":"chat-id","choices":[{"delta":{"content":"Hello"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ]);

        let result = model(&fetch)
            .do_stream(&call_options(json!({ "includeRawChunks": true })))
            .await
            .unwrap();

        assert_eq!(
            Value::Array(stream_parts(result).await),
            json!([
                {
                    "type": "stream-start",
                    "warnings": []
                },
                {
                    "rawValue": {
                        "choices": [
                            {
                                "delta": {
                                    "content": "Hello"
                                }
                            }
                        ],
                        "id": "chat-id"
                    },
                    "type": "raw"
                },
                {
                    "id": "chat-id",
                    "type": "response-metadata"
                },
                {
                    "id": "txt-0",
                    "type": "text-start"
                },
                {
                    "delta": "Hello",
                    "id": "txt-0",
                    "type": "text-delta"
                },
                {
                    "rawValue": {
                        "choices": [
                            {
                                "delta": {},
                                "finish_reason": "stop"
                            }
                        ]
                    },
                    "type": "raw"
                },
                {
                    "id": "txt-0",
                    "type": "text-end"
                },
                {
                    "finishReason": {
                        "raw": "stop",
                        "unified": "stop"
                    },
                    "providerMetadata": {
                        "test-provider": {}
                    },
                    "type": "finish",
                    "usage": {
                        "inputTokens": {},
                        "outputTokens": {}
                    }
                }
            ])
        );
    }
}

mod transform_request_body {
    use super::*;

    /// `prepareTransformJsonResponse`.
    fn prepare_transform_json_response() -> Arc<MockFetch> {
        json_mock(json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "grok-3",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello!" },
                "finish_reason": "stop",
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
        }))
    }

    /// `prepareTransformStreamResponse`.
    fn prepare_transform_stream_response() -> Arc<MockFetch> {
        stream_mock(&[
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1711115037,"model":"grok-3","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1711115037,"model":"grok-3","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}"#,
        ])
    }

    /// `vi.fn(body => ({ ...body, custom_field: 'added-by-transform' }))`:
    /// the transform and the bodies it was called with.
    fn transform_fn() -> (
        crate::openai_compatible::TransformRequestBody,
        Arc<Mutex<Vec<Value>>>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let transform: crate::openai_compatible::TransformRequestBody =
            Arc::new(move |mut body: Value| {
                seen.lock().unwrap().push(body.clone());
                body["custom_field"] = json!("added-by-transform");
                body
            });
        (transform, calls)
    }

    /// `new OpenAICompatibleChatLanguageModel('grok-3', { provider:
    /// 'test-provider', url: ({ path }) => `https://my.api.com/v1${path}`,
    /// headers: () => ({}), ... })`.
    fn transform_model(
        fetch: &Arc<MockFetch>,
        transform: Option<crate::openai_compatible::TransformRequestBody>,
    ) -> OpenAICompatibleChatModel {
        direct_model(
            "grok-3",
            OpenAICompatibleProviderSettings {
                base_url: "https://my.api.com/v1".into(),
                transform_request_body: transform,
                ..settings(fetch)
            },
        )
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › transformRequestBody › should transform request body in doGenerate when transformRequestBody is provided
    #[tokio::test]
    async fn should_transform_request_body_in_do_generate_when_transform_request_body_is_provided()
    {
        let (transform, calls) = transform_fn();
        let fetch = prepare_transform_json_response();
        let model = transform_model(&fetch, Some(transform));

        model.do_generate(&call_options(json!({}))).await.unwrap();

        // Verify transform was called
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["model"], "grok-3");
        assert_eq!(
            calls[0]["messages"],
            json!([{ "role": "user", "content": "Hello" }])
        );
        // Verify transformed body was sent
        assert_match_object(
            &request_body(&fetch),
            &json!({ "custom_field": "added-by-transform" }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › transformRequestBody › should transform request body in doStream when transformRequestBody is provided
    #[tokio::test]
    async fn should_transform_request_body_in_do_stream_when_transform_request_body_is_provided() {
        let (transform, calls) = transform_fn();
        let fetch = prepare_transform_stream_response();
        let model = transform_model(&fetch, Some(transform));

        let result = model.do_stream(&call_options(json!({}))).await.unwrap();
        // Consume the stream
        stream_parts(result).await;

        // Verify transform was called
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["model"], "grok-3");
        assert_eq!(
            calls[0]["messages"],
            json!([{ "role": "user", "content": "Hello" }])
        );
        assert_eq!(calls[0]["stream"], true);
        // Verify transformed body was sent
        assert_match_object(
            &request_body(&fetch),
            &json!({ "custom_field": "added-by-transform" }),
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-language-model.test.ts › transformRequestBody › should work without transformRequestBody
    #[tokio::test]
    async fn should_work_without_transform_request_body() {
        let fetch = prepare_transform_json_response();
        let model = transform_model(&fetch, None);

        model.do_generate(&call_options(json!({}))).await.unwrap();
        let request_body = request_body(&fetch);

        assert_match_object(&request_body, &json!({ "model": "grok-3" }));
        assert_eq!(request_body.get("custom_field"), None);
    }
}

// ── Older aimux tests (previously the unit tests of `openai_compatible/config.rs`) ──

mod chat_settings {
    use crate::openai_compatible::ChatSettings;

    static MODELS: std::sync::LazyLock<Vec<String>> =
        std::sync::LazyLock::new(|| vec!["qwen3.7-plus".to_string(), "kimi-k*".to_string()]);

    fn supports(model_id: &str) -> bool {
        ChatSettings {
            supports_structured_outputs: true,
            structured_output_models: Some(&MODELS),
            ..ChatSettings::default()
        }
        .for_model(model_id)
        .supports_structured_outputs
    }

    #[test]
    fn a_model_list_matches_the_exact_id() {
        assert!(supports("qwen3.7-plus"));
        assert!(!supports("qwen3.7-plus-2026-09-01"));
        assert!(!supports("qwen3.7-plusplus"));
        assert!(!supports("qwen3.7"));
    }

    #[test]
    fn an_entry_ending_in_a_star_matches_a_prefix() {
        assert!(supports("kimi-k"));
        assert!(supports("kimi-k2.5"));
        assert!(!supports("kimi-latest"));
        assert!(!supports("kimi"));
    }

    #[test]
    fn without_a_model_list_the_flag_applies() {
        let settings = ChatSettings {
            supports_structured_outputs: true,
            ..ChatSettings::default()
        };
        assert!(settings.for_model("any").supports_structured_outputs);
    }
}

// ── Older aimux tests (previously the unit tests of `openai_compatible/chat.rs`) ──

mod error_structure_and_base_url {
    use std::sync::Arc;

    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use aimux_core::error::AiMuxError;
    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::LanguageModelMessage;
    use aimux_core::options::CallOptions;
    use aimux_provider_utils::ProviderErrorParts;

    use crate::openai_compatible::{Assembly, BaseUrl, ChatSettings, OpenAICompatibleProvider};
    use crate::shared::Credential;

    fn hello() -> CallOptions {
        CallOptions::new(vec![LanguageModelMessage::user_text("hi")])
    }

    fn provider(base_url: BaseUrl, chat: ChatSettings) -> OpenAICompatibleProvider {
        OpenAICompatibleProvider::assemble(Assembly {
            name: "acme".to_string(),
            base_url,
            credential: Credential::None,
            fixed_headers: Vec::new(),
            headers: None,
            query_params: None,
            fetch: None,
            transform_request_body: None,
            chat,
            package: crate::shared::aisdk_pins::OPENAI_COMPATIBLE,
        })
        .unwrap()
    }

    async fn serve(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(response)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn an_error_structure_decides_the_message_and_code() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(400).set_body_json(json!({"detail": "bad prompt", "kind": 42})),
        )
        .await;
        let chat = ChatSettings {
            error_structure: Arc::new(|data: &Value| ProviderErrorParts {
                message: data["detail"].as_str().unwrap_or_default().to_string(),
                provider_code: data["kind"].as_u64().map(|k| k.to_string()),
            }),
            ..ChatSettings::default()
        };
        let error = provider(BaseUrl::Fixed(server.uri()), chat)
            .chat("m")
            .do_generate(&hello())
            .await
            .unwrap_err();
        match error {
            AiMuxError::ApiCall(detail) => {
                assert_eq!(detail.message, "bad prompt");
                assert_eq!(detail.provider_code.as_deref(), Some("42"));
            }
            other => panic!("expected ApiCall, got {other:?}"),
        }
    }

    /// A preset base URL is data the request resolves: creating the provider
    /// and the model reads nothing, a request that cannot resolve it fails,
    /// and the next request resolves it again.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_lazy_base_url_is_evaluated_per_request_and_its_error_fails_the_request() {
        use crate::preset::{AuthMode, ParamSpec, PresetDescriptor};

        const VAR: &str = "AIMUX_TEST_COMPAT_LAZY_BASE_URL";
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "c", "model": "m",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
            })),
        )
        .await;
        // SAFETY: the test is `#[serial]` and no other test reads `VAR`.
        unsafe { std::env::remove_var(VAR) };
        let descriptor: &'static PresetDescriptor = Box::leak(Box::new(PresetDescriptor {
            name: "acme".to_string(),
            display: "Acme".to_string(),
            base_url: "http://{host}/v1".to_string(),
            env_var: String::new(),
            auth: AuthMode::None,
            base_url_env: Some(VAR.to_string()),
            max_tokens_key: None,
            convert_usage: None,
            params: vec![ParamSpec {
                name: "host".to_string(),
                env: Vec::new(),
                default: None,
                derive: None,
            }],
            user_agent: ("openai-compatible", "3.0.62"),
            structured_output_models: None,
        }));
        let lazy = BaseUrl::Preset {
            descriptor,
            params: Arc::new(std::collections::HashMap::new()),
        };
        let provider = provider(lazy, ChatSettings::default());
        let model = provider.chat("m");
        let error = model.do_generate(&hello()).await.unwrap_err();
        assert!(matches!(error, AiMuxError::InvalidArgument(_)), "{error:?}");
        // SAFETY: as above.
        unsafe { std::env::set_var(VAR, server.uri()) };
        let result = model.do_generate(&hello()).await;
        // SAFETY: as above.
        unsafe { std::env::remove_var(VAR) };
        result.expect("resolved on the next request");
    }
}

// ── Older ports (previously `tests/openai_compatible_upstream_e2e_test.rs`) ──
//
// End-to-end subset of the pinned openai-compatible upstream tests.

mod upstream_e2e {
    use std::sync::Arc;

    use aimux_core::AiMuxError;
    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::LanguageModelMessage;
    use aimux_core::options::CallOptions;
    use aimux_core::result::GenerateContent;
    use aimux_core::shared::provider_namespace;
    use aimux_core::stream_part::StreamPart;
    use aimux_core::tool::FunctionTool;
    use aimux_core::types::FinishReasonUnified;
    use futures::StreamExt;
    use serde_json::{Value, json};

    use crate::openai_compatible::tests::support::*;
    use crate::openai_compatible::{
        OpenAICompatibleProvider, OpenAICompatibleProviderSettings, create_openai_compatible,
    };
    use crate::test_support::mock_fetch::{Canned, MockFetch};

    /// partial: `should use camelCase metadata key when camelCase provider options are used`; full port in src/openai_compatible/tests/chat/chat_language_model.rs.
    /// partial: `should extract detailed token usage when available`; full port in src/openai_compatible/tests/chat/chat_language_model.rs.
    #[tokio::test]
    async fn chat_generate_metadata_and_usage() {
        let usage = json!({"prompt_tokens":20,"completion_tokens":30,"total_tokens":50,
            "prompt_tokens_details":{"cached_tokens":5},
            "completion_tokens_details":{"reasoning_tokens":10,"accepted_prediction_tokens":15,"rejected_prediction_tokens":5}});
        let fetch = MockFetch::new(vec![Canned::json(&chat_response(
            json!({"content":"Hello!"}),
            usage.clone(),
        ))]);
        let mut opts = options();
        opts.provider_options = Some(
            provider_namespace("testProvider", json!({"reasoningEffort":"high"}))
                .expect("provider metadata must be an object"),
        );
        let result = provider(&fetch)
            .chat("test-model")
            .do_generate(&opts)
            .await
            .unwrap();
        let expected = json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}],"reasoning_effort":"high"});
        assert_request(&fetch, "/chat/completions", expected.clone());
        assert_eq!(result.request.unwrap().body, Some(expected));
        assert_eq!(result.content.len(), 1);
        assert!(
            matches!(&result.content[0], GenerateContent::Text { text, .. } if text == "Hello!")
        );
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("stop"));
        assert_eq!(result.usage.input_tokens.total, Some(20));
        assert_eq!(result.usage.input_tokens.no_cache, Some(15));
        assert_eq!(result.usage.input_tokens.cache_read, Some(5));
        assert_eq!(result.usage.input_tokens.cache_write, None);
        assert_eq!(result.usage.output_tokens.total, Some(30));
        assert_eq!(result.usage.output_tokens.text, Some(20));
        assert_eq!(result.usage.output_tokens.reasoning, Some(10));
        assert_eq!(result.usage.raw, usage.as_object().cloned());
        let metadata = result.provider_metadata.unwrap();
        assert_eq!(
            metadata,
            provider_namespace(
                "testProvider",
                json!({"acceptedPredictionTokens":15,"rejectedPredictionTokens":5})
            )
            .expect("provider metadata must be an object")
        );
        assert!(!metadata.contains_key("test-provider"));
        let response = result.response.unwrap();
        assert_eq!(response.id.as_deref(), Some("chatcmpl-test"));
        assert_eq!(response.model_id.as_deref(), Some("test-model"));
        assert_eq!(
            response.headers.unwrap()["content-type"],
            "application/json"
        );
        assert!(result.warnings.is_empty());
    }

    /// partial: `should stream reasoning content before text deltas`; full port in src/openai_compatible/tests/chat/chat_language_model.rs.
    #[tokio::test]
    async fn chat_stream_reasoning_before_text() {
        let delta = |value: Value| {
            json!({"id":"chatcmpl-test","created":1711357598,
            "model":"test-model","choices":[{"index":0,"delta":value,"finish_reason":null}]})
        };
        let fetch = MockFetch::new(vec![stream_response(&[
            delta(json!({"role":"assistant","content":"","reasoning_content":"Let me think"})),
            delta(json!({"content":"","reasoning_content":" about this"})),
            delta(json!({"content":"Here's"})),
            delta(json!({"content":" my response"})),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":18,"completion_tokens":439}}),
        ])]);
        let result = provider(&fetch)
            .chat("test-model")
            .do_stream(&options())
            .await
            .unwrap();
        assert_request(&fetch, "/chat/completions", stream_request());
        assert_eq!(result.request.unwrap().body, Some(stream_request()));
        let parts: Vec<_> = result.stream.map(|part| part.unwrap()).collect().await;
        assert_eq!(parts.len(), 11);
        assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
        assert!(
            matches!(&parts[1], StreamPart::ResponseMetadata(metadata) if metadata.id.as_deref() == Some("chatcmpl-test") && metadata.model_id.as_deref() == Some("test-model"))
        );
        assert!(matches!(&parts[2], StreamPart::ReasoningStart { id, .. } if id == "reasoning-0"));
        assert!(matches!(&parts[5], StreamPart::ReasoningEnd { id, .. } if id == "reasoning-0"));
        assert!(matches!(&parts[6], StreamPart::TextStart { id, .. } if id == "txt-0"));
        assert!(matches!(&parts[9], StreamPart::TextEnd { id, .. } if id == "txt-0"));
        let deltas: Vec<_> = parts
            .iter()
            .filter_map(|part| match part {
                StreamPart::ReasoningDelta { id, delta, .. } => {
                    Some(("reasoning", id.as_str(), delta.as_str()))
                }
                StreamPart::TextDelta { id, delta, .. } => {
                    Some(("text", id.as_str(), delta.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            deltas,
            vec![
                ("reasoning", "reasoning-0", "Let me think"),
                ("reasoning", "reasoning-0", " about this"),
                ("text", "txt-0", "Here's"),
                ("text", "txt-0", " my response")
            ]
        );
        let StreamPart::Finish {
            finish_reason,
            usage,
            provider_metadata,
        } = &parts[10]
        else {
            panic!("expected finish");
        };
        assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(usage.input_tokens.total, Some(18));
        assert_eq!(usage.input_tokens.no_cache, Some(18));
        assert_eq!(usage.input_tokens.cache_read, Some(0));
        assert_eq!(usage.output_tokens.total, Some(439));
        assert_eq!(usage.output_tokens.text, Some(439));
        assert_eq!(usage.output_tokens.reasoning, Some(0));
        assert_eq!(
            provider_metadata.as_ref().unwrap(),
            &provider_namespace("test-provider", json!({}))
                .expect("provider metadata must be an object")
        );
    }

    /// partial: `should parse thought signature from extra_content and include in providerMetadata`; full port in src/openai_compatible/tests/chat/chat_language_model.rs.
    #[tokio::test]
    async fn chat_tool_call_preserves_arguments_and_signature() {
        let fetch = MockFetch::new(vec![Canned::json(&chat_response(
            json!({"tool_calls":[{
            "id":"function-call-1","type":"function",
            "function":{"name":"check_flight","arguments":"{\"flight\":\"AA100\"}"},
            "extra_content":{"google":{"thought_signature":"<Signature A>"}}}]}),
            Value::Null,
        ))]);
        let schema = json!({"type":"object","properties":{"flight":{"type":"string"}},
            "required":["flight"],"additionalProperties":false,"$schema":"http://json-schema.org/draft-07/schema#"});
        let mut opts = options();
        opts.tools = Some(vec![
            FunctionTool::new("check_flight", schema.clone()).into(),
        ]);
        let result = provider(&fetch)
            .chat("test-model")
            .do_generate(&opts)
            .await
            .unwrap();
        assert_request(
            &fetch,
            "/chat/completions",
            json!({"model":"test-model",
            "messages":[{"role":"user","content":"Hello"}],
            "tools":[{"type":"function","function":{"name":"check_flight","parameters":schema}}]}),
        );
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("stop"));
        let GenerateContent::ToolCall(call) = &result.content[0] else {
            panic!("expected tool call");
        };
        assert_eq!(call.tool_call_id, "function-call-1");
        assert_eq!(call.tool_name, "check_flight");
        assert_eq!(call.input, "{\"flight\":\"AA100\"}");
        assert_eq!(
            call.provider_metadata.as_ref().unwrap(),
            &provider_namespace("test-provider", json!({"thoughtSignature":"<Signature A>"}))
                .expect("provider metadata must be an object")
        );
    }

    /// partial: `should preserve structured error stream parts`; full port in src/openai_compatible/tests/chat/chat_language_model.rs.
    #[tokio::test]
    async fn chat_stream_preserves_structured_error() {
        let fetch = MockFetch::new(vec![stream_response(&[json!({"error":{
            "message":"Context length exceeded","code":"CONTEXT_LENGTH_EXCEEDED"}})])]);
        let result = provider(&fetch)
            .chat("test-model")
            .do_stream(&options())
            .await
            .unwrap();
        assert_request(&fetch, "/chat/completions", stream_request());
        let parts: Vec<_> = result.stream.map(|part| part.unwrap()).collect().await;
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
        let StreamPart::Error {
            error: AiMuxError::ApiCall(error),
        } = &parts[1]
        else {
            panic!("expected structured API error");
        };
        assert_eq!(error.message, "Context length exceeded");
        assert_eq!(
            error.provider_code.as_deref(),
            Some("CONTEXT_LENGTH_EXCEEDED")
        );
        let StreamPart::Finish {
            finish_reason,
            usage,
            provider_metadata,
        } = &parts[2]
        else {
            panic!("expected finish");
        };
        assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
        assert_eq!(finish_reason.raw, None);
        assert_eq!(usage.input_tokens.total, None);
        assert_eq!(usage.output_tokens.total, None);
        assert_eq!(usage.raw, None);
        assert_eq!(
            provider_metadata.as_ref().unwrap(),
            &provider_namespace("test-provider", json!({}))
                .expect("provider metadata must be an object")
        );
    }

    /// Upstream: @ai-sdk/openai-compatible 3.0.62
    /// src/chat/openai-compatible-chat-language-model.ts:588-595 passes an error
    /// chunk's `error` on as it is: even a numeric `code` that looks like an HTTP
    /// status is no status, so the error is not retryable.
    #[tokio::test]
    async fn chat_stream_error_takes_no_status_from_a_numeric_code() {
        for (code, provider_code) in [
            (json!(429), "429"),
            (json!(503), "503"),
            (json!("429"), "429"),
        ] {
            let payload = json!({"message": "Slow down", "code": code});
            let fetch = MockFetch::new(vec![stream_response(&[json!({ "error": payload })])]);
            let parts: Vec<_> = provider(&fetch)
                .chat("test-model")
                .do_stream(&options())
                .await
                .unwrap()
                .stream
                .map(|part| part.unwrap())
                .collect()
                .await;
            let error = parts
                .iter()
                .find_map(|part| match part {
                    StreamPart::Error {
                        error: AiMuxError::ApiCall(error),
                    } => Some(error),
                    _ => None,
                })
                .expect("expected an API error part");
            assert_eq!(error.message, "Slow down", "{code}");
            assert_eq!(
                error.provider_code.as_deref(),
                Some(provider_code),
                "{code}"
            );
            assert_eq!(error.status_code, None, "{code}");
            assert!(!error.is_retryable, "{code}");
            assert_eq!(error.data.as_ref(), Some(&payload), "{code}");
        }
    }

    /// A tool result whose output is `content` (a text part and a base64 image
    /// part), as the AI SDK passes it to the provider.
    fn multi_part_tool_prompt(
        extra: Option<aimux_core::language_model_message::ToolResultContent>,
    ) -> CallOptions {
        use aimux_core::language_model_message::{
            FilePart, TextPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart,
        };
        use aimux_core::shared::{FileBytes, FileData};
        let mut value = vec![
            ToolResultContent::Text(TextPart {
                text: "The weather is sunny".into(),
                provider_options: None,
            }),
            ToolResultContent::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Base64("AAECAw==".into()),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            }),
        ];
        value.extend(extra);
        CallOptions::new(vec![
            LanguageModelMessage::user_text("Weather?"),
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call-1".into(),
                    tool_name: "weather".into(),
                    output: ToolResultOutput::Content { value },
                    provider_options: None,
                })],
                provider_options: None,
            },
        ])
    }

    fn multi_part_provider(
        fetch: &Arc<MockFetch>,
        supported: Option<bool>,
    ) -> OpenAICompatibleProvider {
        create_openai_compatible(OpenAICompatibleProviderSettings {
            name: "test-provider".into(),
            base_url: BASE_URL.into(),
            fetch: Some(fetch.transport()),
            supports_multi_part_tool_content: supported,
            ..Default::default()
        })
        .unwrap()
    }

    /// partial: `should pass multi-part tool content when supported`; full port in src/openai_compatible/tests/chat/chat_language_model.rs.
    /// partial: `should convert multi-part tool content when supported`; full port in src/openai_compatible/tests/chat/convert_to_chat_messages.rs.
    /// partial: `should stringify multi-part tool content by default`; full port in src/openai_compatible/tests/chat/convert_to_chat_messages.rs.
    /// Upstream: @ai-sdk/openai-compatible 3.0.62
    /// src/chat/convert-to-openai-compatible-chat-messages.ts:303-306 sends a
    /// `content` tool output as an array of content parts
    /// (`convertToolContentPart`, :162-176) when `supportsMultiPartToolContent`
    /// (openai-compatible-provider.ts:94-100, :183) is set, and as
    /// `JSON.stringify(output.value)` otherwise (the default, `false`).
    #[tokio::test]
    async fn chat_multi_part_tool_content_follows_the_setting() {
        let response = || {
            Canned::json(&chat_response(
                json!({"role":"assistant","content":"ok"}),
                json!({"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}),
            ))
        };
        let fetch = MockFetch::new(vec![response(), response(), response()]);
        let tool_message = |index: usize| fetch.seen()[index].json_body()["messages"][1].clone();

        multi_part_provider(&fetch, Some(true))
            .chat("test-model")
            .do_generate(&multi_part_tool_prompt(None))
            .await
            .unwrap();
        assert_eq!(
            tool_message(0),
            json!({
                "role": "tool",
                "tool_call_id": "call-1",
                "content": [
                    {"type": "text", "text": "The weather is sunny"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAECAw=="}}
                ]
            })
        );

        for (index, supported) in [(1, None), (2, Some(false))] {
            multi_part_provider(&fetch, supported)
                .chat("test-model")
                .do_generate(&multi_part_tool_prompt(None))
                .await
                .unwrap();
            let message = tool_message(index);
            let content: Value =
                serde_json::from_str(message["content"].as_str().expect("a JSON string")).unwrap();
            assert_eq!(
                content,
                json!([
                    {"type": "text", "text": "The weather is sunny"},
                    {"type": "file", "mediaType": "image/png", "data": {"type": "data", "data": "AAECAw=="}}
                ]),
                "{supported:?}"
            );
        }
    }

    /// Upstream: convert-to-openai-compatible-chat-messages.ts:166-170 rejects a
    /// custom part in multi-part tool content.
    #[tokio::test]
    async fn chat_multi_part_tool_content_rejects_a_custom_part() {
        let fetch = MockFetch::new(Vec::new());
        let error = multi_part_provider(&fetch, Some(true))
            .chat("test-model")
            .do_generate(&multi_part_tool_prompt(Some(
                aimux_core::language_model_message::ToolResultContent::Custom {
                    provider_options: None,
                },
            )))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AiMuxError::UnsupportedFunctionality(what) if what == "custom tool content parts"),
            "{error:?}"
        );
        assert!(fetch.seen().is_empty());
    }
}
