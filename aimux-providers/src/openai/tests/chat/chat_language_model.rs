use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch};

const BASE_URL: &str = "https://api.openai.com/v1";
const CHAT_URL: &str = "https://api.openai.com/v1/chat/completions";
/// The pinned user agent (upstream mocks the version as `0.0.0-test`).
const USER_AGENT: &str = "ai-sdk-openai/4.0.83";

const OPENAI_TEXT_JSON: &str = include_str!("fixtures/openai-text.json");
const OPENAI_TEXT_CHUNKS: &str = include_str!("fixtures/openai-text.chunks.txt");
const AZURE_MODEL_ROUTER_CHUNKS: &str = include_str!("fixtures/azure-model-router.1.chunks.txt");
const MODEL_ROUTER_SNAPSHOT: &str = include_str!("snapshots/model_router.json");

fn test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

fn test_logprobs() -> Value {
    let entry = |token: &str, logprob: f64| json!({ "token": token, "logprob": logprob, "top_logprobs": [{ "token": token, "logprob": logprob }] });
    json!({
        "content": [
            entry("Hello", -0.0009994634),
            entry("!", -0.13410144),
            entry(" How", -0.0009250381),
            entry(" can", -0.047709424),
            entry(" I", -0.000009014684),
            entry(" assist", -0.009125131),
            entry(" you", -0.0000066306106),
            entry(" today", -0.00011093382),
            entry("?", -0.00004596782),
        ],
    })
}

/// The test server's `json-value` response: the body with `content-type`
/// and `content-length`, plus the case's extra headers.
fn json_value_with_headers(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let mut canned = Canned::json(body);
    canned
        .headers
        .push(("content-length".into(), canned.body.len().to_string()));
    for (name, value) in headers {
        canned.headers.push(((*name).into(), (*value).into()));
    }
    canned
}

fn json_value(body: &Value) -> Canned {
    json_value_with_headers(body, &[])
}

/// The test server's `stream-chunks` response, with its default headers.
fn stream_chunks_with_headers(chunks: &[String], headers: &[(&str, &str)]) -> Canned {
    let mut all = vec![
        ("content-type".to_string(), "text/event-stream".to_string()),
        ("cache-control".to_string(), "no-cache".to_string()),
        ("connection".to_string(), "keep-alive".to_string()),
    ];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    Canned {
        status: 200,
        headers: all,
        body: chunks.concat().into_bytes(),
    }
}

fn stream_chunks(chunks: &[String]) -> Canned {
    stream_chunks_with_headers(chunks, &[])
}

/// `prepareChunksFixtureResponse`: one `data:` event per non-empty line,
/// then `[DONE]`.
fn chunks_fixture(text: &str) -> Canned {
    let mut chunks: Vec<String> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("data: {line}\n\n"))
        .collect();
    chunks.push("data: [DONE]\n\n".into());
    stream_chunks(&chunks)
}

/// `prepareJsonFixtureResponse('openai-text')`.
fn openai_text_json() -> Canned {
    json_value(&serde_json::from_str(OPENAI_TEXT_JSON).unwrap())
}

/// One SSE event per upstream chunk string (`data: ...\n\n`).
fn data(chunks: &[&str]) -> Vec<String> {
    chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect()
}

/// `createOpenAI({ apiKey: 'test-api-key' })` on the mock transport.
fn provider_with(mock: &Arc<MockFetch>, settings: OpenAIProviderSettings) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some(BASE_URL.into()),
        fetch: Some(mock.transport()),
        ..settings
    })
    .unwrap()
}

fn provider(mock: &Arc<MockFetch>) -> OpenAIProvider {
    provider_with(mock, OpenAIProviderSettings::default())
}

/// The upstream `doGenerate` / `doStream` options literal; `prompt` defaults
/// to `TEST_PROMPT`.
fn call_options(options: Value) -> CallOptions {
    let mut options = options;
    if options.get("prompt").is_none() {
        options["prompt"] = test_prompt();
    }
    serde_json::from_value(options).expect("valid call options")
}

async fn generate_result(
    model_id: &str,
    canned: Canned,
    options: Value,
) -> (Result<GenerateResult, AiMuxError>, Arc<MockFetch>) {
    let mock = MockFetch::new(vec![canned]);
    let result = provider(&mock)
        .chat(model_id)
        .do_generate(&call_options(options))
        .await;
    (result, mock)
}

async fn generate(
    model_id: &str,
    canned: Canned,
    options: Value,
) -> (GenerateResult, Arc<MockFetch>) {
    let (result, mock) = generate_result(model_id, canned, options).await;
    (result.expect("doGenerate succeeds"), mock)
}

async fn stream_result(
    model_id: &str,
    canned: Canned,
    options: Value,
) -> (Result<StreamResult, AiMuxError>, Arc<MockFetch>) {
    let mock = MockFetch::new(vec![canned]);
    let result = provider(&mock)
        .chat(model_id)
        .do_stream(&call_options(options))
        .await;
    (result, mock)
}

async fn stream(model_id: &str, canned: Canned, options: Value) -> (StreamResult, Arc<MockFetch>) {
    let (result, mock) = stream_result(model_id, canned, options).await;
    (result.expect("doStream succeeds"), mock)
}

/// `convertReadableStreamToArray(stream)` as JSON values.
async fn collect(result: StreamResult) -> Vec<Value> {
    result.stream.map(part_json).collect().await
}

async fn stream_parts(
    model_id: &str,
    canned: Canned,
    options: Value,
) -> (Vec<Value>, Arc<MockFetch>) {
    let (result, mock) = stream(model_id, canned, options).await;
    (collect(result).await, mock)
}

/// A stream part as its AI SDK JSON. An error part's `error` is the API call
/// error's AI SDK fields (`message`, `statusCode`, `isRetryable`, `data`,
/// `code`); a stream that fails (an `Err` item) is `{ "streamError": ... }`.
fn part_json(item: Result<StreamPart, AiMuxError>) -> Value {
    match item {
        Ok(StreamPart::Error { error }) => json!({ "type": "error", "error": error_json(&error) }),
        Ok(part) => serde_json::to_value(part).unwrap(),
        Err(error) => json!({ "streamError": serde_json::to_value(error).unwrap() }),
    }
}

fn error_json(error: &AiMuxError) -> Value {
    match error {
        AiMuxError::ApiCall(error) => {
            let mut value = Map::new();
            if let Some(code) = &error.provider_code {
                value.insert("code".into(), json!(code));
            }
            if let Some(data) = &error.data {
                value.insert("data".into(), data.clone());
            }
            value.insert("isRetryable".into(), json!(error.is_retryable));
            value.insert("message".into(), json!(error.message));
            if let Some(status) = error.status_code {
                value.insert("statusCode".into(), json!(status));
            }
            Value::Object(value)
        }
        other => serde_json::to_value(other).unwrap(),
    }
}

/// Upstream's provider stream error also carries the OpenAI error `type`,
/// which aimux's API call error keeps only inside `data`; the field is left
/// out of the comparison.
fn without_error_type(mut part: Value) -> Value {
    if let Some(error) = part.get_mut("error").and_then(Value::as_object_mut) {
        error.remove("type");
    }
    part
}

fn request_body(mock: &MockFetch) -> Value {
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, CHAT_URL);
    seen[0].json_body()
}

/// `server.calls[0].requestHeaders` (the user agent is reported apart).
fn request_headers(mock: &MockFetch) -> (Value, String) {
    let mut headers = mock.seen()[0].headers.clone();
    let user_agent = headers.remove("user-agent").unwrap_or_default();
    (json!(headers), user_agent)
}

/// The usual completion body of the upstream cases.
fn completion(message: Value, finish_reason: &str, usage: Value) -> Value {
    json!({
        "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "gpt-3.5-turbo-0125",
        "choices": [{ "index": 0, "message": message, "finish_reason": finish_reason }],
        "usage": usage,
        "system_fingerprint": "fp_3bc1b5746c",
    })
}

fn usage_4_34_30() -> Value {
    json!({ "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 })
}

fn empty_message() -> Value {
    json!({ "role": "assistant", "content": "" })
}

fn tool_call_message() -> Value {
    json!({
        "role": "assistant",
        "content": "",
        "tool_calls": [{
            "id": "call_O17Uplv4lJvD6DVdIvFFeRMw",
            "type": "function",
            "function": { "name": "test-tool", "arguments": "{\"value\":\"Spark\"}" },
        }],
    })
}

fn test_tool_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "value": { "type": "string" } },
        "required": ["value"],
        "additionalProperties": false,
        "$schema": "http://json-schema.org/draft-07/schema#",
    })
}

fn test_tool() -> Value {
    json!({ "type": "function", "name": "test-tool", "inputSchema": test_tool_schema() })
}

fn hello_body(model: &str) -> Value {
    json!({ "model": model, "messages": [{ "role": "user", "content": "Hello" }] })
}

fn hello_stream_body(model: &str) -> Value {
    json!({
        "model": model,
        "messages": [{ "role": "user", "content": "Hello" }],
        "stream": true,
        "stream_options": { "include_usage": true },
    })
}

fn with(mut value: Value, extra: Value) -> Value {
    for (key, field) in extra.as_object().unwrap() {
        value[key] = field.clone();
    }
    value
}

fn warnings(result: &GenerateResult) -> Value {
    serde_json::to_value(&result.warnings).unwrap()
}

fn tool_call_content() -> Value {
    json!([{
        "input": "{\"value\":\"Spark\"}",
        "toolCallId": "call_O17Uplv4lJvD6DVdIvFFeRMw",
        "toolName": "test-tool",
        "type": "tool-call",
    }])
}

fn finish_usage(input: u32, output: u32, raw: Value) -> Value {
    json!({
        "inputTokens": { "cacheRead": 0, "noCache": input, "total": input },
        "outputTokens": { "reasoning": 0, "text": output, "total": output },
        "raw": raw,
    })
}

/// The finish part after a stream error: no reason, no usage.
fn error_finish() -> Value {
    json!({
        "finishReason": { "unified": "error" },
        "providerMetadata": { "openai": {} },
        "type": "finish",
        "usage": { "inputTokens": {}, "outputTokens": {} },
    })
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should extract text response
    #[tokio::test]
    async fn should_extract_text_response() {
        let body = json!({
            "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-3.5-turbo-0125",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello, World!" },
                "finish_reason": "stop",
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "system_fingerprint": "fp_3bc1b5746c",
        });

        let (result, _) = generate("gpt-3.5-turbo", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.content),
            json!([{ "text": "Hello, World!", "type": "text" }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should extract an audio transcript alongside tool calls
    #[tokio::test]
    async fn should_extract_an_audio_transcript_alongside_tool_calls() {
        let body = json!({
            "id": "chatcmpl-audio",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-audio-1.5",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "audio": {
                        "id": "audio-1",
                        "data": "base64-audio",
                        "expires_at": 1711118637,
                        "transcript": "Fix the login bug",
                    },
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": { "name": "test-tool", "arguments": "{\"value\":\"Spark\"}" },
                    }],
                },
                "finish_reason": "tool_calls",
            }],
        });

        let (result, _) = generate("gpt-3.5-turbo", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.content),
            json!([
                { "type": "text", "text": "Fix the login bug" },
                {
                    "type": "tool-call",
                    "toolCallId": "call-1",
                    "toolName": "test-tool",
                    "input": "{\"value\":\"Spark\"}",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should reject a response without choices
    #[tokio::test]
    async fn should_reject_a_response_without_choices() {
        let body = json!({
            "id": "chatcmpl-empty",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-3.5-turbo-0125",
            "choices": [],
            "usage": { "prompt_tokens": 4, "total_tokens": 4, "completion_tokens": 0 },
        });

        let (result, _) = generate_result("gpt-3.5-turbo", json_value(&body), json!({})).await;

        match result {
            Err(AiMuxError::InvalidResponseData { message, .. }) => {
                assert_eq!(message, "Response did not contain any choices.");
            }
            other => panic!("expected InvalidResponseData, got {other:?}"),
        }
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should extract usage
    #[tokio::test]
    async fn should_extract_usage() {
        let body = completion(
            empty_message(),
            "stop",
            json!({ "prompt_tokens": 20, "total_tokens": 25, "completion_tokens": 5 }),
        );

        let (result, _) = generate("gpt-3.5-turbo", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.usage),
            json!({
                "inputTokens": { "cacheRead": 0, "noCache": 20, "total": 20 },
                "outputTokens": { "reasoning": 0, "text": 5, "total": 5 },
                "raw": { "completion_tokens": 5, "prompt_tokens": 20, "total_tokens": 25 },
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let (result, _) = generate("gpt-3.5-turbo", openai_text_json(), json!({})).await;

        assert_eq!(
            json!(result.request),
            json!({
                "body": {
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gpt-3.5-turbo",
                },
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send additional response information
    #[tokio::test]
    async fn should_send_additional_response_information() {
        let body = json!({
            "id": "test-id",
            "object": "chat.completion",
            "created": 123,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "" },
                "finish_reason": "stop",
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
            "system_fingerprint": "fp_3bc1b5746c",
        });

        let (result, _) = generate("gpt-3.5-turbo", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.response),
            json!({
                "body": {
                    "choices": [{
                        "finish_reason": "stop",
                        "index": 0,
                        "message": { "content": "", "role": "assistant" },
                    }],
                    "created": 123,
                    "id": "test-id",
                    "model": "test-model",
                    "object": "chat.completion",
                    "system_fingerprint": "fp_3bc1b5746c",
                    "usage": { "completion_tokens": 30, "prompt_tokens": 4, "total_tokens": 34 },
                },
                "headers": { "content-length": "275", "content-type": "application/json" },
                "id": "test-id",
                "modelId": "test-model",
                "timestamp": "1970-01-01T00:02:03.000Z",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should support partial usage
    #[tokio::test]
    async fn should_support_partial_usage() {
        let body = completion(
            empty_message(),
            "stop",
            json!({ "prompt_tokens": 20, "total_tokens": 20 }),
        );

        let (result, _) = generate("gpt-3.5-turbo", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.usage),
            json!({
                "inputTokens": { "cacheRead": 0, "noCache": 20, "total": 20 },
                "outputTokens": { "reasoning": 0, "text": 0, "total": 0 },
                "raw": { "prompt_tokens": 20, "total_tokens": 20 },
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should extract logprobs
    #[tokio::test]
    async fn should_extract_logprobs() {
        let mut body = completion(empty_message(), "stop", usage_4_34_30());
        body["choices"][0]["logprobs"] = test_logprobs();

        let (result, _) = generate(
            "gpt-3.5-turbo",
            json_value(&body),
            json!({ "providerOptions": { "openai": { "logprobs": 1 } } }),
        )
        .await;

        assert_eq!(
            json!(result.provider_metadata)["openai"]["logprobs"],
            test_logprobs()["content"]
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should extract finish reason
    #[tokio::test]
    async fn should_extract_finish_reason() {
        let body = completion(empty_message(), "stop", usage_4_34_30());

        let (result, _) = generate("gpt-3.5-turbo", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.finish_reason),
            json!({ "raw": "stop", "unified": "stop" })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should support unknown finish reason
    #[tokio::test]
    async fn should_support_unknown_finish_reason() {
        let body = completion(empty_message(), "eos", usage_4_34_30());

        let (result, _) = generate("gpt-3.5-turbo", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.finish_reason),
            json!({ "raw": "eos", "unified": "other" })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let body = completion(empty_message(), "stop", usage_4_34_30());

        let (result, _) = generate(
            "gpt-3.5-turbo",
            json_value_with_headers(&body, &[("test-header", "test-value")]),
            json!({}),
        )
        .await;

        assert_eq!(
            json!(result.response.unwrap().headers),
            json!({
                "content-length": "321",
                "content-type": "application/json",
                "test-header": "test-value",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass the model and the messages
    #[tokio::test]
    async fn should_pass_the_model_and_the_messages() {
        let (_, mock) = generate("gpt-3.5-turbo", openai_text_json(), json!({})).await;

        assert_eq!(request_body(&mock), hello_body("gpt-3.5-turbo"));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass settings
    #[tokio::test]
    async fn should_pass_settings() {
        let (_, mock) = generate(
            "gpt-3.5-turbo",
            openai_text_json(),
            json!({
                "providerOptions": {
                    "openai": {
                        "logitBias": { "50256": -100 },
                        "parallelToolCalls": false,
                        "user": "test-user-id",
                    },
                },
            }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            json!({
                "logit_bias": { "50256": -100 },
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gpt-3.5-turbo",
                "parallel_tool_calls": false,
                "user": "test-user-id",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should not set reasoning_effort when reasoning is "provider-default"
    #[tokio::test]
    async fn should_not_set_reasoning_effort_when_reasoning_is_provider_default() {
        let (_, mock) = generate(
            "o4-mini",
            openai_text_json(),
            json!({ "reasoning": "provider-default" }),
        )
        .await;

        assert_eq!(request_body(&mock), hello_body("o4-mini"));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass top-level reasoning as reasoning_effort
    #[tokio::test]
    async fn should_pass_top_level_reasoning_as_reasoning_effort() {
        let (_, mock) = generate(
            "o4-mini",
            openai_text_json(),
            json!({ "reasoning": "medium" }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            with(
                hello_body("o4-mini"),
                json!({ "reasoning_effort": "medium" })
            )
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should prefer providerOptions reasoningEffort over top-level reasoning
    #[tokio::test]
    async fn should_prefer_provider_options_reasoning_effort_over_top_level_reasoning() {
        let (_, mock) = generate(
            "o4-mini",
            openai_text_json(),
            json!({
                "reasoning": "medium",
                "providerOptions": { "openai": { "reasoningEffort": "high" } },
            }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            with(hello_body("o4-mini"), json!({ "reasoning_effort": "high" }))
        );
    }

    async fn reasoning_effort_setting(model_id: &str, effort: &str) {
        let (_, mock) = generate(
            model_id,
            openai_text_json(),
            json!({ "providerOptions": { "openai": { "reasoningEffort": effort } } }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            with(hello_body(model_id), json!({ "reasoning_effort": effort }))
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass reasoningEffort setting from provider metadata
    #[tokio::test]
    async fn should_pass_reasoning_effort_setting_from_provider_metadata() {
        reasoning_effort_setting("o4-mini", "low").await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass reasoningEffort setting from settings
    #[tokio::test]
    async fn should_pass_reasoning_effort_setting_from_settings() {
        reasoning_effort_setting("o4-mini", "high").await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass reasoningEffort xhigh setting
    #[tokio::test]
    async fn should_pass_reasoning_effort_xhigh_setting() {
        reasoning_effort_setting("gpt-5.1-codex-max", "xhigh").await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass reasoningEffort max setting
    #[tokio::test]
    async fn should_pass_reasoning_effort_max_setting() {
        reasoning_effort_setting("gpt-5.6", "max").await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass textVerbosity setting from provider options
    #[tokio::test]
    async fn should_pass_text_verbosity_setting_from_provider_options() {
        let (_, mock) = generate(
            "gpt-4o",
            openai_text_json(),
            json!({ "providerOptions": { "openai": { "textVerbosity": "low" } } }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            with(hello_body("gpt-4o"), json!({ "verbosity": "low" }))
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass tools and toolChoice
    #[tokio::test]
    async fn should_pass_tools_and_tool_choice() {
        let (_, mock) = generate(
            "gpt-3.5-turbo",
            openai_text_json(),
            json!({
                "tools": [test_tool()],
                "toolChoice": { "type": "tool", "toolName": "test-tool" },
            }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            json!({
                "messages": [{ "content": "Hello", "role": "user" }],
                "model": "gpt-3.5-turbo",
                "tool_choice": { "function": { "name": "test-tool" }, "type": "function" },
                "tools": [{
                    "function": { "name": "test-tool", "parameters": test_tool_schema() },
                    "type": "function",
                }],
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![openai_text_json()]);
        let provider = provider_with(
            &mock,
            OpenAIProviderSettings {
                organization: Some("test-organization".into()),
                project: Some("test-project".into()),
                headers: Some(
                    [(
                        "Custom-Provider-Header".to_string(),
                        Some("provider-header-value".to_string()),
                    )]
                    .into_iter()
                    .collect(),
                ),
                ..OpenAIProviderSettings::default()
            },
        );

        provider
            .chat("gpt-3.5-turbo")
            .do_generate(&call_options(json!({
                "headers": { "Custom-Request-Header": "request-header-value" },
            })))
            .await
            .unwrap();

        let (headers, user_agent) = request_headers(&mock);
        assert_eq!(
            headers,
            json!({
                "authorization": "Bearer test-api-key",
                "content-type": "application/json",
                "custom-provider-header": "provider-header-value",
                "custom-request-header": "request-header-value",
                "openai-organization": "test-organization",
                "openai-project": "test-project",
            })
        );
        assert!(user_agent.contains(USER_AGENT), "{user_agent}");
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should parse tool results
    #[tokio::test]
    async fn should_parse_tool_results() {
        let body = completion(tool_call_message(), "stop", usage_4_34_30());

        let (result, _) = generate(
            "gpt-3.5-turbo",
            json_value(&body),
            json!({
                "tools": [test_tool()],
                "toolChoice": { "type": "tool", "toolName": "test-tool" },
            }),
        )
        .await;

        assert_eq!(json!(result.content), tool_call_content());
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should parse annotations/citations
    #[tokio::test]
    async fn should_parse_annotations_citations() {
        let body = completion(
            json!({
                "role": "assistant",
                "content": "Based on the search results [doc1], I found information.",
                "annotations": [{
                    "type": "url_citation",
                    "url_citation": {
                        "start_index": 24,
                        "end_index": 29,
                        "url": "https://example.com/doc1.pdf",
                        "title": "Document 1",
                    },
                }],
            }),
            "stop",
            usage_4_34_30(),
        );

        let (result, _) = generate("gpt-3.5-turbo", json_value(&body), json!({})).await;

        let mut content = json!(result.content);
        // `id: expect.any(String)`
        assert!(content[1]["id"].is_string(), "{content}");
        content[1]["id"] = json!("<any string>");
        assert_eq!(
            content,
            json!([
                {
                    "text": "Based on the search results [doc1], I found information.",
                    "type": "text",
                },
                {
                    "id": "<any string>",
                    "sourceType": "url",
                    "title": "Document 1",
                    "type": "source",
                    "url": "https://example.com/doc1.pdf",
                },
            ])
        );
    }

    mod response_format {
        use super::*;

        const MODEL: &str = "gpt-4o-2024-08-06";

        fn json_schema_body(json_schema: Value) -> Value {
            with(
                hello_body(MODEL),
                json!({ "response_format": { "json_schema": json_schema, "type": "json_schema" } }),
            )
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › response format › should not send a response_format when response format is text
        #[tokio::test]
        async fn should_not_send_a_response_format_when_response_format_is_text() {
            let (_, mock) = generate(
                MODEL,
                openai_text_json(),
                json!({ "responseFormat": { "type": "text" } }),
            )
            .await;

            assert_eq!(request_body(&mock), hello_body(MODEL));
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › response format › should forward json response format as "json_object" without schema
        #[tokio::test]
        async fn should_forward_json_response_format_as_json_object_without_schema() {
            let (_, mock) = generate(
                MODEL,
                openai_text_json(),
                json!({ "responseFormat": { "type": "json" } }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                with(
                    hello_body(MODEL),
                    json!({ "response_format": { "type": "json_object" } })
                )
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › response format › should forward json response format as "json_object" and include schema
        #[tokio::test]
        async fn should_forward_json_response_format_as_json_object_and_include_schema() {
            let (result, mock) = generate(
                MODEL,
                openai_text_json(),
                json!({ "responseFormat": { "type": "json", "schema": test_tool_schema() } }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                json_schema_body(json!({
                    "name": "response",
                    "schema": test_tool_schema(),
                    "strict": true,
                }))
            );
            assert_eq!(warnings(&result), json!([]));
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › response format › should remove string propertyNames from response schemas and warn
        #[tokio::test]
        async fn should_remove_string_property_names_from_response_schemas_and_warn() {
            let (result, mock) = generate(
                MODEL,
                openai_text_json(),
                json!({
                    "responseFormat": {
                        "type": "json",
                        "schema": {
                            "type": "object",
                            "properties": {
                                "variables": {
                                    "type": "object",
                                    "propertyNames": { "type": "string", "format": "uuid" },
                                    "additionalProperties": { "type": "string" },
                                },
                            },
                            "required": ["variables"],
                            "additionalProperties": false,
                        },
                    },
                }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                json_schema_body(json!({
                    "name": "response",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {
                            "variables": {
                                "type": "object",
                                "additionalProperties": { "type": "string" },
                            },
                        },
                        "required": ["variables"],
                        "additionalProperties": false,
                    },
                }))
            );
            assert_eq!(
                warnings(&result),
                json!([{
                    "type": "compatibility",
                    "feature": "JSON Schema propertyNames",
                    "details": "OpenAI does not support JSON Schema propertyNames. It was removed before sending the schema, so OpenAI will not enforce property-name constraints.",
                }])
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › response format › should use json_schema & strict with responseFormat json
        #[tokio::test]
        async fn should_use_json_schema_and_strict_with_response_format_json() {
            let (_, mock) = generate(
                MODEL,
                openai_text_json(),
                json!({ "responseFormat": { "type": "json", "schema": test_tool_schema() } }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                json_schema_body(json!({
                    "name": "response",
                    "schema": test_tool_schema(),
                    "strict": true,
                }))
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › response format › should set name & description with responseFormat json
        #[tokio::test]
        async fn should_set_name_and_description_with_response_format_json() {
            let (_, mock) = generate(
                MODEL,
                openai_text_json(),
                json!({
                    "responseFormat": {
                        "type": "json",
                        "name": "test-name",
                        "description": "test description",
                        "schema": test_tool_schema(),
                    },
                }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                json_schema_body(json!({
                    "description": "test description",
                    "name": "test-name",
                    "schema": test_tool_schema(),
                    "strict": true,
                }))
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › response format › should allow for undefined schema with responseFormat json when structuredOutputs are enabled
        #[tokio::test]
        async fn should_allow_for_undefined_schema_with_response_format_json() {
            let (_, mock) = generate(
                MODEL,
                openai_text_json(),
                json!({
                    "responseFormat": {
                        "type": "json",
                        "name": "test-name",
                        "description": "test description",
                    },
                }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                with(
                    hello_body(MODEL),
                    json!({ "response_format": { "type": "json_object" } })
                )
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › response format › should set strict with tool call
        #[tokio::test]
        async fn should_set_strict_with_tool_call() {
            let body = completion(tool_call_message(), "stop", usage_4_34_30());

            let (result, mock) = generate(
                MODEL,
                json_value(&body),
                json!({
                    "tools": [{
                        "type": "function",
                        "name": "test-tool",
                        "description": "test description",
                        "inputSchema": test_tool_schema(),
                    }],
                    "toolChoice": { "type": "required" },
                }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                with(
                    hello_body(MODEL),
                    json!({
                        "tool_choice": "required",
                        "tools": [{
                            "function": {
                                "description": "test description",
                                "name": "test-tool",
                                "parameters": test_tool_schema(),
                            },
                            "type": "function",
                        }],
                    })
                )
            );
            assert_eq!(json!(result.content), tool_call_content());
        }
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should set strict for tool usage
    #[tokio::test]
    async fn should_set_strict_for_tool_usage() {
        let body = completion(tool_call_message(), "stop", usage_4_34_30());

        let (result, mock) = generate(
            "gpt-4o-2024-08-06",
            json_value(&body),
            json!({
                "tools": [test_tool()],
                "toolChoice": { "type": "tool", "toolName": "test-tool" },
            }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            with(
                hello_body("gpt-4o-2024-08-06"),
                json!({
                    "tool_choice": { "function": { "name": "test-tool" }, "type": "function" },
                    "tools": [{
                        "function": { "name": "test-tool", "parameters": test_tool_schema() },
                        "type": "function",
                    }],
                })
            )
        );
        assert_eq!(json!(result.content), tool_call_content());
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should return cached_tokens in prompt_details_tokens
    #[tokio::test]
    async fn should_return_cached_tokens_in_prompt_details_tokens() {
        let usage = json!({
            "prompt_tokens": 2000,
            "completion_tokens": 20,
            "total_tokens": 2020,
            "prompt_tokens_details": { "cached_tokens": 1152, "cache_write_tokens": 256 },
        });
        let body = completion(empty_message(), "stop", usage.clone());

        let (result, _) = generate("gpt-4o-mini", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.usage),
            json!({
                "inputTokens": { "cacheRead": 1152, "cacheWrite": 256, "noCache": 592, "total": 2000 },
                "outputTokens": { "reasoning": 0, "text": 20, "total": 20 },
                "raw": usage,
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should return accepted_prediction_tokens and rejected_prediction_tokens in completion_details_tokens
    #[tokio::test]
    async fn should_return_accepted_and_rejected_prediction_tokens_in_completion_details() {
        let body = completion(
            empty_message(),
            "stop",
            json!({
                "prompt_tokens": 15,
                "completion_tokens": 20,
                "total_tokens": 35,
                "completion_tokens_details": {
                    "accepted_prediction_tokens": 123,
                    "rejected_prediction_tokens": 456,
                },
            }),
        );

        let (result, _) = generate("gpt-4o-mini", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.provider_metadata),
            json!({ "openai": { "acceptedPredictionTokens": 123, "rejectedPredictionTokens": 456 } })
        );
    }

    mod reasoning_models {
        use super::*;

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › reasoning models › should clear out temperature, top_p, frequency_penalty, presence_penalty and return warnings
        #[tokio::test]
        async fn should_clear_out_temperature_top_p_frequency_penalty_presence_penalty() {
            let (result, mock) = generate(
                "o4-mini",
                openai_text_json(),
                json!({
                    "temperature": 0.5,
                    "topP": 0.7,
                    "frequencyPenalty": 0.2,
                    "presencePenalty": 0.3,
                }),
            )
            .await;

            assert_eq!(request_body(&mock), hello_body("o4-mini"));
            assert_eq!(
                warnings(&result),
                json!([
                    {
                        "details": "temperature is not supported for reasoning models",
                        "feature": "temperature",
                        "type": "unsupported",
                    },
                    {
                        "details": "topP is not supported for reasoning models",
                        "feature": "topP",
                        "type": "unsupported",
                    },
                    {
                        "details": "frequencyPenalty is not supported for reasoning models",
                        "feature": "frequencyPenalty",
                        "type": "unsupported",
                    },
                    {
                        "details": "presencePenalty is not supported for reasoning models",
                        "feature": "presencePenalty",
                        "type": "unsupported",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › reasoning models › should convert maxOutputTokens to max_completion_tokens
        #[tokio::test]
        async fn should_convert_max_output_tokens_to_max_completion_tokens() {
            let (_, mock) = generate(
                "o4-mini",
                openai_text_json(),
                json!({ "maxOutputTokens": 1000 }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                with(
                    hello_body("o4-mini"),
                    json!({ "max_completion_tokens": 1000 })
                )
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › reasoning models › should allow temperature when top-level reasoning is none on gpt-5.1
        #[tokio::test]
        async fn should_allow_temperature_when_top_level_reasoning_is_none_on_gpt_5_1() {
            let (result, mock) = generate(
                "gpt-5.1",
                openai_text_json(),
                json!({ "reasoning": "none", "temperature": 0.5 }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                with(
                    hello_body("gpt-5.1"),
                    json!({ "reasoning_effort": "none", "temperature": 0.5 })
                )
            );
            assert_eq!(warnings(&result), json!([]));
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › reasoning models › should preserve disabled reasoning for %s
        #[tokio::test]
        async fn should_preserve_disabled_reasoning_for_model() {
            for model_id in ["gpt-6-sol", "gpt-6-luna"] {
                let (result, mock) = generate(
                    model_id,
                    openai_text_json(),
                    json!({ "providerOptions": { "openai": { "reasoningEffort": "none" } } }),
                )
                .await;

                // toMatchObject
                let body = request_body(&mock);
                assert_eq!(body["model"], json!(model_id), "{model_id}");
                assert_eq!(
                    body["reasoning_effort"],
                    json!("none"),
                    "{model_id}: {body}"
                );
                assert_eq!(warnings(&result), json!([]), "{model_id}");
            }
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › reasoning models › should omit unsupported GPT-6 reasoning effort %s
        #[tokio::test]
        async fn should_omit_unsupported_gpt_6_reasoning_effort() {
            for effort in ["none", "minimal"] {
                let (result, mock) = generate(
                    "gpt-6-astra",
                    openai_text_json(),
                    json!({ "providerOptions": { "openai": { "reasoningEffort": effort } } }),
                )
                .await;

                assert_eq!(request_body(&mock), hello_body("gpt-6-astra"), "{effort}");
                assert_eq!(
                    warnings(&result),
                    json!([{
                        "type": "unsupported",
                        "feature": "reasoningEffort",
                        "details": "gpt-6-astra only supports the following reasoning efforts: low, medium, high, xhigh, max",
                    }]),
                    "{effort}"
                );
            }
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › reasoning models › should strip sampling and logprob settings for GPT-6 models
        #[tokio::test]
        async fn should_strip_sampling_and_logprob_settings_for_gpt_6_models() {
            let (result, mock) = generate(
                "gpt-6-astra",
                openai_text_json(),
                json!({
                    "temperature": 0.5,
                    "topP": 0.7,
                    "providerOptions": { "openai": { "reasoningEffort": "low", "logprobs": 5 } },
                }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                with(
                    hello_body("gpt-6-astra"),
                    json!({ "reasoning_effort": "low" })
                )
            );
            assert_eq!(
                warnings(&result),
                json!([
                    {
                        "type": "unsupported",
                        "feature": "temperature",
                        "details": "temperature is not supported for reasoning models",
                    },
                    {
                        "type": "unsupported",
                        "feature": "topP",
                        "details": "topP is not supported for reasoning models",
                    },
                    { "type": "other", "message": "logprobs is not supported for reasoning models" },
                    { "type": "other", "message": "topLogprobs is not supported for reasoning models" },
                ])
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › reasoning models › should still clear temperature when top-level reasoning is none on o4-mini
        #[tokio::test]
        async fn should_still_clear_temperature_when_top_level_reasoning_is_none_on_o4_mini() {
            let (result, mock) = generate(
                "o4-mini",
                openai_text_json(),
                json!({ "reasoning": "none", "temperature": 0.5 }),
            )
            .await;

            assert_eq!(
                request_body(&mock),
                with(hello_body("o4-mini"), json!({ "reasoning_effort": "none" }))
            );
            assert_eq!(
                warnings(&result),
                json!([{
                    "details": "temperature is not supported for reasoning models",
                    "feature": "temperature",
                    "type": "unsupported",
                }])
            );
        }
    }

    fn system_prompt() -> Value {
        json!([
            { "role": "system", "content": "You are a helpful assistant." },
            { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
        ])
    }

    fn system_body(model: &str, role: &str) -> Value {
        json!({
            "model": model,
            "messages": [
                { "role": role, "content": "You are a helpful assistant." },
                { "role": "user", "content": "Hello" },
            ],
        })
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should allow forcing reasoning behavior for unrecognized model IDs via providerOptions
    #[tokio::test]
    async fn should_allow_forcing_reasoning_behavior_for_unrecognized_model_ids() {
        let (result, mock) = generate(
            "stealth-reasoning-model",
            openai_text_json(),
            json!({
                "temperature": 0.5,
                "topP": 0.7,
                "providerOptions": { "openai": { "forceReasoning": true } },
            }),
        )
        .await;

        assert_eq!(request_body(&mock), hello_body("stealth-reasoning-model"));
        assert_eq!(
            warnings(&result),
            json!([
                {
                    "details": "temperature is not supported for reasoning models",
                    "feature": "temperature",
                    "type": "unsupported",
                },
                {
                    "details": "topP is not supported for reasoning models",
                    "feature": "topP",
                    "type": "unsupported",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should default systemMessageMode to developer when forcing reasoning
    #[tokio::test]
    async fn should_default_system_message_mode_to_developer_when_forcing_reasoning() {
        let (result, mock) = generate(
            "stealth-reasoning-model",
            openai_text_json(),
            json!({
                "prompt": system_prompt(),
                "providerOptions": { "openai": { "forceReasoning": true } },
            }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            system_body("stealth-reasoning-model", "developer")
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should use developer messages for o1
    #[tokio::test]
    async fn should_use_developer_messages_for_o1() {
        let (result, mock) = generate(
            "o1",
            openai_text_json(),
            json!({ "prompt": system_prompt() }),
        )
        .await;

        assert_eq!(request_body(&mock), system_body("o1", "developer"));
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should allow overriding systemMessageMode via providerOptions
    #[tokio::test]
    async fn should_allow_overriding_system_message_mode_via_provider_options() {
        let (result, mock) = generate(
            "gpt-4o",
            openai_text_json(),
            json!({
                "prompt": system_prompt(),
                "providerOptions": { "openai": { "systemMessageMode": "developer" } },
            }),
        )
        .await;

        assert_eq!(request_body(&mock), system_body("gpt-4o", "developer"));
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should use default systemMessageMode when not overridden
    #[tokio::test]
    async fn should_use_default_system_message_mode_when_not_overridden() {
        let (result, mock) = generate(
            "gpt-4o",
            openai_text_json(),
            json!({ "prompt": system_prompt() }),
        )
        .await;

        assert_eq!(request_body(&mock), system_body("gpt-4o", "system"));
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should return the reasoning tokens in the provider metadata
    #[tokio::test]
    async fn should_return_the_reasoning_tokens_in_the_provider_metadata() {
        let usage = json!({
            "prompt_tokens": 15,
            "completion_tokens": 20,
            "total_tokens": 35,
            "completion_tokens_details": { "reasoning_tokens": 10 },
        });
        let body = completion(empty_message(), "stop", usage.clone());

        let (result, _) = generate("o4-mini", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.usage),
            json!({
                "inputTokens": { "cacheRead": 0, "noCache": 15, "total": 15 },
                "outputTokens": { "reasoning": 10, "text": 10, "total": 20 },
                "raw": usage,
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send max_completion_tokens extension setting
    #[tokio::test]
    async fn should_send_max_completion_tokens_extension_setting() {
        let mut body = completion(empty_message(), "stop", usage_4_34_30());
        body["model"] = json!("o4-mini");

        let (_, mock) = generate(
            "o4-mini",
            json_value(&body),
            json!({ "providerOptions": { "openai": { "maxCompletionTokens": 255 } } }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            with(
                hello_body("o4-mini"),
                json!({ "max_completion_tokens": 255 })
            )
        );
    }

    async fn extension_setting(model_id: &str, options: Value, extra: Value) {
        let (_, mock) = generate(
            model_id,
            openai_text_json(),
            json!({ "providerOptions": { "openai": options } }),
        )
        .await;

        assert_eq!(request_body(&mock), with(hello_body(model_id), extra));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send prediction extension setting
    #[tokio::test]
    async fn should_send_prediction_extension_setting() {
        extension_setting(
            "gpt-3.5-turbo",
            json!({ "prediction": { "type": "content", "content": "Hello, World!" } }),
            json!({ "prediction": { "type": "content", "content": "Hello, World!" } }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send store extension setting
    #[tokio::test]
    async fn should_send_store_extension_setting() {
        extension_setting(
            "gpt-3.5-turbo",
            json!({ "store": true }),
            json!({ "store": true }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send metadata extension values
    #[tokio::test]
    async fn should_send_metadata_extension_values() {
        extension_setting(
            "gpt-3.5-turbo",
            json!({ "metadata": { "custom": "value" } }),
            json!({ "metadata": { "custom": "value" } }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send promptCacheKey extension value
    #[tokio::test]
    async fn should_send_prompt_cache_key_extension_value() {
        extension_setting(
            "gpt-3.5-turbo",
            json!({ "promptCacheKey": "test-cache-key-123" }),
            json!({ "prompt_cache_key": "test-cache-key-123" }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send promptCacheRetention extension value
    #[tokio::test]
    async fn should_send_prompt_cache_retention_extension_value() {
        extension_setting(
            "gpt-3.5-turbo",
            json!({ "promptCacheRetention": "24h" }),
            json!({ "prompt_cache_retention": "24h" }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send promptCacheOptions extension value
    #[tokio::test]
    async fn should_send_prompt_cache_options_extension_value() {
        extension_setting(
            "gpt-5.6",
            json!({ "promptCacheOptions": { "mode": "explicit", "ttl": "30m" } }),
            json!({ "prompt_cache_options": { "mode": "explicit", "ttl": "30m" } }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should omit legacy prompt cache retention for GPT-6 models
    #[tokio::test]
    async fn should_omit_legacy_prompt_cache_retention_for_gpt_6_models() {
        let (result, mock) = generate(
            "gpt-6-astra",
            openai_text_json(),
            json!({ "providerOptions": { "openai": { "promptCacheRetention": "24h" } } }),
        )
        .await;

        assert_eq!(request_body(&mock), hello_body("gpt-6-astra"));
        assert_eq!(
            warnings(&result),
            json!([{
                "type": "unsupported",
                "feature": "promptCacheRetention",
                "details": "promptCacheRetention is not supported by GPT-6 and later models; use promptCacheOptions instead",
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send safetyIdentifier extension value
    #[tokio::test]
    async fn should_send_safety_identifier_extension_value() {
        extension_setting(
            "gpt-3.5-turbo",
            json!({ "safetyIdentifier": "test-safety-identifier-123" }),
            json!({ "safety_identifier": "test-safety-identifier-123" }),
        )
        .await;
    }

    async fn search_preview_temperature(model_id: &str) {
        let (result, mock) =
            generate(model_id, openai_text_json(), json!({ "temperature": 0.7 })).await;

        let body = request_body(&mock);
        assert_eq!(body["model"], json!(model_id));
        assert_eq!(body.get("temperature"), None);
        assert_eq!(
            warnings(&result),
            json!([{
                "details": "temperature is not supported for the search preview models and has been removed.",
                "feature": "temperature",
                "type": "unsupported",
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should remove temperature setting for gpt-4o-search-preview and add warning
    #[tokio::test]
    async fn should_remove_temperature_setting_for_gpt_4o_search_preview_and_add_warning() {
        search_preview_temperature("gpt-4o-search-preview").await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should remove temperature setting for gpt-4o-mini-search-preview and add warning
    #[tokio::test]
    async fn should_remove_temperature_setting_for_gpt_4o_mini_search_preview_and_add_warning() {
        search_preview_temperature("gpt-4o-mini-search-preview").await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should remove temperature setting for gpt-4o-mini-search-preview-2025-03-11 and add warning
    #[tokio::test]
    async fn should_remove_temperature_setting_for_gpt_4o_mini_search_preview_2025_03_11() {
        search_preview_temperature("gpt-4o-mini-search-preview-2025-03-11").await;
    }

    async fn service_tier(model_id: &str, tier: &str) -> (GenerateResult, Value) {
        let (result, mock) = generate(
            model_id,
            openai_text_json(),
            json!({ "providerOptions": { "openai": { "serviceTier": tier } } }),
        )
        .await;
        let body = request_body(&mock);
        (result, body)
    }

    const PRIORITY_WARNING: &str = "priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported";

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send serviceTier flex processing setting
    #[tokio::test]
    async fn should_send_service_tier_flex_processing_setting() {
        let (_, body) = service_tier("o4-mini", "flex").await;
        assert_eq!(
            body,
            with(hello_body("o4-mini"), json!({ "service_tier": "flex" }))
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should show warning when using flex processing with unsupported model
    #[tokio::test]
    async fn should_show_warning_when_using_flex_processing_with_unsupported_model() {
        let (result, body) = service_tier("gpt-4o-mini", "flex").await;
        assert_eq!(body.get("service_tier"), None);
        assert_eq!(
            warnings(&result),
            json!([{
                "details": "flex processing is only available for o3, o4-mini, and gpt-5 models",
                "feature": "serviceTier",
                "type": "unsupported",
            }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should allow flex processing with o4-mini model without warnings
    #[tokio::test]
    async fn should_allow_flex_processing_with_o4_mini_model_without_warnings() {
        let (result, body) = service_tier("o4-mini", "flex").await;
        assert_eq!(body["service_tier"], json!("flex"));
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send serviceTier priority processing setting
    #[tokio::test]
    async fn should_send_service_tier_priority_processing_setting() {
        let (_, body) = service_tier("gpt-4o-mini", "priority").await;
        assert_eq!(
            body,
            with(
                hello_body("gpt-4o-mini"),
                json!({ "service_tier": "priority" })
            )
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should show warning when using priority processing with unsupported model
    #[tokio::test]
    async fn should_show_warning_when_using_priority_processing_with_unsupported_model() {
        let (result, body) = service_tier("gpt-3.5-turbo", "priority").await;
        assert_eq!(body.get("service_tier"), None);
        assert_eq!(
            warnings(&result),
            json!([{ "details": PRIORITY_WARNING, "feature": "serviceTier", "type": "unsupported" }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should allow priority processing with gpt-4o model without warnings
    #[tokio::test]
    async fn should_allow_priority_processing_with_gpt_4o_model_without_warnings() {
        let (result, body) = service_tier("gpt-4o", "priority").await;
        assert_eq!(body["service_tier"], json!("priority"));
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should allow priority processing with o3 model without warnings
    #[tokio::test]
    async fn should_allow_priority_processing_with_o3_model_without_warnings() {
        let (result, body) = service_tier("o4-mini", "priority").await;
        assert_eq!(body["service_tier"], json!("priority"));
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send serviceTier fast processing setting
    #[tokio::test]
    async fn should_send_service_tier_fast_processing_setting() {
        let (_, body) = service_tier("gpt-4o-mini", "fast").await;
        assert_eq!(
            body,
            with(hello_body("gpt-4o-mini"), json!({ "service_tier": "fast" }))
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should show warning when using fast processing with unsupported model
    #[tokio::test]
    async fn should_show_warning_when_using_fast_processing_with_unsupported_model() {
        let (result, body) = service_tier("gpt-3.5-turbo", "fast").await;
        assert_eq!(body.get("service_tier"), None);
        assert_eq!(
            warnings(&result),
            json!([{ "details": PRIORITY_WARNING, "feature": "serviceTier", "type": "unsupported" }])
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should allow fast processing with gpt-4o model without warnings
    #[tokio::test]
    async fn should_allow_fast_processing_with_gpt_4o_model_without_warnings() {
        let (result, body) = service_tier("gpt-4o", "fast").await;
        assert_eq!(body["service_tier"], json!("fast"));
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doGenerate › should send serviceTier ultrafast processing setting
    #[tokio::test]
    async fn should_send_service_tier_ultrafast_processing_setting() {
        let (_, body) = service_tier("gpt-5.6-sol", "ultrafast").await;
        assert_eq!(
            body,
            with(
                hello_body("gpt-5.6-sol"),
                json!({ "service_tier": "ultrafast" })
            )
        );
    }
}

mod do_stream {
    use super::*;

    const ID: &str = "chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP";

    fn text_chunk(model: &str, index: u32, delta: &str, finish: &str, extra: &str) -> String {
        format!(
            r#"{{"id":"{ID}","object":"chat.completion.chunk","created":1702657020,"model":"{model}","system_fingerprint":null,"choices":[{{"index":{index},"delta":{delta},"finish_reason":{finish}{extra}}}]}}"#
        )
    }

    fn usage_chunk(model: &str, usage: &str) -> String {
        format!(
            r#"{{"id":"{ID}","object":"chat.completion.chunk","created":1702657020,"model":"{model}","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{usage}}}"#
        )
    }

    fn done(mut chunks: Vec<String>) -> Vec<String> {
        chunks.push("data: [DONE]\n\n".into());
        chunks
    }

    fn sse(chunks: &[String]) -> Vec<String> {
        done(
            chunks
                .iter()
                .map(|chunk| format!("data: {chunk}\n\n"))
                .collect(),
        )
    }

    /// A tool-call chunk of the `gpt-3.5-turbo-0125` tool streams.
    fn tool_chunk(choice: &str) -> String {
        format!(
            r#"{{"id":"{ID}","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[{choice}]}}"#
        )
    }

    fn tool_args_chunk(arguments: &str) -> String {
        tool_chunk(&format!(
            r#"{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":{}}}}}]}},"logprobs":null,"finish_reason":null}}"#,
            json!(arguments)
        ))
    }

    fn tool_stream_tail() -> Vec<String> {
        vec![
            tool_chunk(r#"{"index":0,"delta":{},"logprobs":null,"finish_reason":"tool_calls"}"#),
            format!(
                r#"{{"id":"{ID}","object":"chat.completion.chunk","created":1711357598,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{{"prompt_tokens":53,"completion_tokens":17,"total_tokens":70}}}}"#
            ),
        ]
    }

    fn tool_options() -> Value {
        json!({ "tools": [test_tool()], "includeRawChunks": false })
    }

    fn response_metadata(id: &str, model: &str, timestamp: &str) -> Value {
        json!({ "id": id, "modelId": model, "timestamp": timestamp, "type": "response-metadata" })
    }

    fn stream_start() -> Value {
        json!({ "type": "stream-start", "warnings": [] })
    }

    fn tool_parts(id: &str, name: &str, deltas: &[&str], input: &str) -> Vec<Value> {
        let mut parts = vec![json!({ "id": id, "toolName": name, "type": "tool-input-start" })];
        parts.extend(
            deltas
                .iter()
                .map(|delta| json!({ "delta": delta, "id": id, "type": "tool-input-delta" })),
        );
        parts.push(json!({ "id": id, "type": "tool-input-end" }));
        parts.push(
            json!({ "input": input, "toolCallId": id, "toolName": name, "type": "tool-call" }),
        );
        parts
    }

    fn tool_finish() -> Value {
        json!({
            "finishReason": { "raw": "tool_calls", "unified": "tool-calls" },
            "providerMetadata": { "openai": {} },
            "type": "finish",
            "usage": finish_usage(
                53,
                17,
                json!({ "completion_tokens": 17, "prompt_tokens": 53, "total_tokens": 70 }),
            ),
        })
    }

    fn no_raw() -> Value {
        json!({ "includeRawChunks": false })
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should stream text after Azure content filter chunks
    #[tokio::test]
    async fn should_stream_text_after_azure_content_filter_chunks() {
        let chunks = done(data(&[
            r#"{"choices":[],"created":0,"id":"","model":"","object":"","prompt_filter_results":[{"prompt_index":0,"content_filter_results":{}}]}"#,
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"","role":"assistant"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[{"content_filter_offsets":{},"content_filter_results":{},"finish_reason":null,"index":0}],"created":0,"id":"","model":"","object":""}"#,
        ]));

        let (parts, _) = stream_parts("gpt-3.5-turbo", stream_chunks(&chunks), no_raw()).await;

        let deltas: Vec<Value> = parts
            .into_iter()
            .filter(|part| part["type"] == "text-delta")
            .collect();
        assert_eq!(
            deltas,
            [
                json!({ "type": "text-delta", "id": "0", "delta": "" }),
                json!({ "type": "text-delta", "id": "0", "delta": "Hello" }),
            ]
        );
    }

    /// Strict stream mode (#403): a reply that ends without a
    /// `finish_reason` fails instead of finishing with `other`, and the tool
    /// call it accumulated is not delivered. With the option off the tool
    /// call is flushed at the end of the body, as in the AI SDK.
    #[tokio::test]
    async fn strict_stream_fails_without_a_finish_reason_and_holds_back_tool_calls() {
        let chunks = done(data(&[
            r#"{"id":"x","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"weather","arguments":"{\"city\":\"Rome\"}"}}]},"finish_reason":null}]}"#,
        ]));
        let is_tool_call = |part: &Result<StreamPart, AiMuxError>| matches!(part, Ok(StreamPart::ToolCall(call)) if call.tool_call_id == "call_1");

        let (result, _) = stream_result("gpt-4o", stream_chunks(&chunks), no_raw()).await;
        let lenient: Vec<_> = result.unwrap().stream.collect().await;
        assert!(lenient.iter().any(is_tool_call), "{lenient:?}");

        let (result, _) = stream_result(
            "gpt-4o",
            stream_chunks(&chunks),
            json!({ "includeRawChunks": false, "strictStream": true }),
        )
        .await;
        let strict: Vec<_> = result.unwrap().stream.collect().await;
        assert!(!strict.iter().any(is_tool_call), "{strict:?}");
        assert!(
            !strict
                .iter()
                .any(|part| matches!(part, Ok(StreamPart::Finish { .. }))),
            "{strict:?}"
        );
        let Some(Err(AiMuxError::InvalidResponseData { message, .. })) = strict.last() else {
            panic!("expected the strict stream to fail: {strict:?}");
        };
        assert_eq!(
            message,
            "Strict stream: the stream ended without a finish_reason"
        );
    }

    /// Strict stream mode (#403): a complete reply finishes as usual; its
    /// tool call is delivered right before `Finish`.
    #[tokio::test]
    async fn strict_stream_delivers_a_complete_reply() {
        let chunks = done(data(&[
            r#"{"id":"x","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"weather","arguments":"{\"city\":\"Rome\"}"}}]},"finish_reason":null}]}"#,
            r#"{"id":"x","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]));
        let (result, _) = stream_result(
            "gpt-4o",
            stream_chunks(&chunks),
            json!({ "includeRawChunks": false, "strictStream": true }),
        )
        .await;
        let parts: Vec<_> = result.unwrap().stream.collect().await;
        let n = parts.len();
        assert!(n >= 2, "{parts:?}");
        assert!(
            matches!(&parts[n - 2], Ok(StreamPart::ToolCall(call)) if call.tool_call_id == "call_1"),
            "{parts:?}"
        );
        assert!(
            matches!(
                &parts[n - 1],
                Ok(StreamPart::Finish { finish_reason, .. })
                    if finish_reason.unified == aimux_core::types::FinishReasonUnified::ToolCalls
            ),
            "{parts:?}"
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should stream text deltas
    #[tokio::test]
    async fn should_stream_text_deltas() {
        let model = "gpt-3.5-turbo-0613";
        let chunks = sse(&[
            text_chunk(model, 0, r#"{"role":"assistant","content":""}"#, "null", ""),
            text_chunk(model, 1, r#"{"content":"Hello"}"#, "null", ""),
            text_chunk(model, 1, r#"{"content":", "}"#, "null", ""),
            text_chunk(model, 1, r#"{"content":"World!"}"#, "null", ""),
            text_chunk(
                model,
                0,
                "{}",
                r#""stop""#,
                &format!(r#","logprobs":{}"#, test_logprobs()),
            ),
            usage_chunk(
                model,
                r#"{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}"#,
            ),
        ]);

        let (parts, _) = stream_parts("gpt-3.5-turbo", stream_chunks(&chunks), no_raw()).await;

        assert_eq!(
            parts,
            [
                stream_start(),
                response_metadata(ID, model, "2023-12-15T16:17:00.000Z"),
                json!({ "id": "0", "type": "text-start" }),
                json!({ "delta": "", "id": "0", "type": "text-delta" }),
                json!({ "delta": "Hello", "id": "0", "type": "text-delta" }),
                json!({ "delta": ", ", "id": "0", "type": "text-delta" }),
                json!({ "delta": "World!", "id": "0", "type": "text-delta" }),
                json!({ "id": "0", "type": "text-end" }),
                json!({
                    "finishReason": { "raw": "stop", "unified": "stop" },
                    "providerMetadata": { "openai": { "logprobs": test_logprobs()["content"] } },
                    "type": "finish",
                    "usage": finish_usage(
                        17,
                        227,
                        json!({ "completion_tokens": 227, "prompt_tokens": 17, "total_tokens": 244 }),
                    ),
                }),
            ]
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should stream annotations/citations
    #[tokio::test]
    async fn should_stream_annotations_citations() {
        let model = "gpt-3.5-turbo-0125";
        let chunks = sse(&[
            text_chunk(model, 0, r#"{"role":"assistant","content":""}"#, "null", ""),
            text_chunk(
                model,
                1,
                r#"{"content":"Based on search results"}"#,
                "null",
                "",
            ),
            text_chunk(
                model,
                1,
                r#"{"annotations":[{"type":"url_citation","url_citation":{"start_index":24,"end_index":29,"url":"https://example.com/doc1.pdf","title":"Document 1"}}]}"#,
                "null",
                "",
            ),
            text_chunk(model, 0, "{}", r#""stop""#, ""),
            usage_chunk(
                model,
                r#"{"prompt_tokens":17,"completion_tokens":227,"total_tokens":244}"#,
            ),
        ]);

        let (parts, _) = stream_parts("gpt-3.5-turbo", stream_chunks(&chunks), no_raw()).await;

        // expect.arrayContaining([...]) with objectContaining / expect.any(String)
        let contains = |expected: &Value| parts.iter().any(|part| part == expected);
        for expected in [
            stream_start(),
            json!({ "type": "text-start", "id": "0" }),
            json!({ "type": "text-delta", "id": "0", "delta": "" }),
            json!({ "type": "text-delta", "id": "0", "delta": "Based on search results" }),
            json!({ "type": "text-end", "id": "0" }),
        ] {
            assert!(contains(&expected), "missing {expected} in {parts:?}");
        }
        assert!(
            parts
                .iter()
                .any(|part| part["type"] == "response-metadata" && part["id"] == ID),
            "{parts:?}"
        );
        assert!(
            parts.iter().any(|part| part["type"] == "source"
                && part["sourceType"] == "url"
                && part["id"].is_string()
                && part["url"] == "https://example.com/doc1.pdf"
                && part["title"] == "Document 1"
                && part.as_object().unwrap().len() == 5),
            "{parts:?}"
        );
        assert!(
            parts.iter().any(|part| part["type"] == "finish"
                && part["finishReason"] == json!({ "unified": "stop", "raw": "stop" })),
            "{parts:?}"
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should stream tool deltas
    #[tokio::test]
    async fn should_stream_tool_deltas() {
        let mut chunks = vec![tool_chunk(
            r#"{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":""}}]},"logprobs":null,"finish_reason":null}"#,
        )];
        for arguments in ["{\"", "value", "\":\"", "Spark", "le", " Day", "\"}"] {
            chunks.push(tool_args_chunk(arguments));
        }
        chunks.extend(tool_stream_tail());

        let (parts, _) = stream_parts(
            "gpt-3.5-turbo",
            stream_chunks(&sse(&chunks)),
            tool_options(),
        )
        .await;

        let mut expected = vec![
            stream_start(),
            response_metadata(ID, "gpt-3.5-turbo-0125", "2024-03-25T09:06:38.000Z"),
        ];
        expected.extend(tool_parts(
            "call_O17Uplv4lJvD6DVdIvFFeRMw",
            "test-tool",
            &["{\"", "value", "\":\"", "Spark", "le", " Day", "\"}"],
            "{\"value\":\"Sparkle Day\"}",
        ));
        expected.push(tool_finish());
        assert_eq!(parts, expected);
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should stream tool call deltas when tool call arguments are passed in the first chunk
    #[tokio::test]
    async fn should_stream_tool_call_deltas_when_arguments_are_passed_in_the_first_chunk() {
        let mut chunks = vec![tool_chunk(
            r#"{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":"{\""}}]},"logprobs":null,"finish_reason":null}"#,
        )];
        for arguments in ["va", "lue", "\":\"", "Spark", "le", " Day", "\"}"] {
            chunks.push(tool_args_chunk(arguments));
        }
        chunks.extend(tool_stream_tail());

        let (parts, _) = stream_parts(
            "gpt-3.5-turbo",
            stream_chunks(&sse(&chunks)),
            tool_options(),
        )
        .await;

        let mut expected = vec![
            stream_start(),
            response_metadata(ID, "gpt-3.5-turbo-0125", "2024-03-25T09:06:38.000Z"),
        ];
        expected.extend(tool_parts(
            "call_O17Uplv4lJvD6DVdIvFFeRMw",
            "test-tool",
            &["{\"", "va", "lue", "\":\"", "Spark", "le", " Day", "\"}"],
            "{\"value\":\"Sparkle Day\"}",
        ));
        expected.push(tool_finish());
        assert_eq!(parts, expected);
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should not duplicate tool calls when there is an additional empty chunk after the tool call has been completed
    #[tokio::test]
    async fn should_not_duplicate_tool_calls_with_an_additional_empty_chunk_after_completion() {
        let chunk = |choice: &str, usage: &str| {
            format!(
                r#"{{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[{choice}],"usage":{usage}}}"#
            )
        };
        let args = |arguments: &str, finish: &str| {
            format!(
                r#"{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":{}}}}}]}},"logprobs":null,"finish_reason":{finish}}}"#,
                json!(arguments)
            )
        };
        let usage = |total: u32, completion: u32| {
            format!(
                r#"{{"prompt_tokens":226,"total_tokens":{total},"completion_tokens":{completion}}}"#
            )
        };
        let mut last = args("", r#""tool_calls""#);
        last.insert_str(last.len() - 1, r#","stop_reason":128008"#);
        let chunks = vec![
            chunk(
                r#"{"index":0,"delta":{"role":"assistant","content":""},"logprobs":null,"finish_reason":null}"#,
                &usage(226, 0),
            ),
            chunk(
                r#"{"index":0,"delta":{"tool_calls":[{"id":"chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa","type":"function","index":0,"function":{"name":"searchGoogle"}}]},"logprobs":null,"finish_reason":null}"#,
                &usage(233, 7),
            ),
            chunk(&args("{\"query\": \"", "null"), &usage(241, 15)),
            chunk(&args("latest", "null"), &usage(242, 16)),
            chunk(&args(" news", "null"), &usage(243, 17)),
            chunk(&args(" on", "null"), &usage(244, 18)),
            chunk(&args(" ai\"}", "null"), &usage(245, 19)),
            // empty arguments chunk after the tool call has already been finished:
            chunk(&last, &usage(246, 20)),
            format!(
                r#"{{"id":"chat-2267f7e2910a4254bac0650ba74cfc1c","object":"chat.completion.chunk","created":1733162241,"model":"meta/llama-3.1-8b-instruct:fp8","choices":[],"usage":{}}}"#,
                usage(246, 20)
            ),
        ];

        let (parts, _) = stream_parts(
            "gpt-3.5-turbo",
            stream_chunks(&sse(&chunks)),
            json!({
                "tools": [{
                    "type": "function",
                    "name": "searchGoogle",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "query": { "type": "string" } },
                        "required": ["query"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                    },
                }],
                "includeRawChunks": false,
            }),
        )
        .await;

        let id = "chatcmpl-tool-b3b307239370432d9910d4b79b4dbbaa";
        let mut expected = vec![
            stream_start(),
            response_metadata(
                "chat-2267f7e2910a4254bac0650ba74cfc1c",
                "meta/llama-3.1-8b-instruct:fp8",
                "2024-12-02T17:57:21.000Z",
            ),
            json!({ "id": "0", "type": "text-start" }),
            json!({ "delta": "", "id": "0", "type": "text-delta" }),
            json!({ "id": id, "toolName": "searchGoogle", "type": "tool-input-start" }),
        ];
        for delta in ["{\"query\": \"", "latest", " news", " on", " ai\"}", ""] {
            expected.push(json!({ "delta": delta, "id": id, "type": "tool-input-delta" }));
        }
        expected.extend([
            json!({ "id": "0", "type": "text-end" }),
            json!({ "id": id, "type": "tool-input-end" }),
            json!({
                "input": "{\"query\": \"latest news on ai\"}",
                "toolCallId": id,
                "toolName": "searchGoogle",
                "type": "tool-call",
            }),
            json!({
                "finishReason": { "raw": "tool_calls", "unified": "tool-calls" },
                "providerMetadata": { "openai": {} },
                "type": "finish",
                "usage": finish_usage(
                    226,
                    20,
                    json!({ "completion_tokens": 20, "prompt_tokens": 226, "total_tokens": 246 }),
                ),
            }),
        ]);
        assert_eq!(parts, expected);
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should not finalize tool call early when partial JSON is coincidentally parsable
    #[tokio::test]
    async fn should_not_finalize_tool_call_early_when_partial_json_is_coincidentally_parsable() {
        let chunk = |choice: &str| {
            format!(
                r#"{{"id":"chatcmpl-early","object":"chat.completion.chunk","created":1733162241,"model":"gpt-4","choices":[{choice}]}}"#
            )
        };
        let args = |arguments: &str| {
            chunk(&format!(
                r#"{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":{}}}}}]}},"finish_reason":null}}"#,
                json!(arguments)
            ))
        };
        let chunks = vec![
            chunk(
                r#"{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_early123","type":"function","function":{"name":"search","arguments":""}}]},"finish_reason":null}"#,
            ),
            args("{\"query\": \"test\"}"),
            args(""),
            args(", \"limit\": 10}"),
            chunk(r#"{"index":0,"delta":{},"finish_reason":"tool_calls"}"#),
        ];

        let (parts, _) = stream_parts(
            "gpt-3.5-turbo",
            stream_chunks(&sse(&chunks)),
            json!({
                "tools": [{
                    "type": "function",
                    "name": "search",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" },
                            "limit": { "type": "number" },
                        },
                        "required": ["query"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                    },
                }],
                "includeRawChunks": false,
            }),
        )
        .await;

        let tool_calls: Vec<&Value> = parts
            .iter()
            .filter(|part| part["type"] == "tool-call")
            .collect();
        assert_eq!(
            tool_calls.first().copied(),
            Some(&json!({
                "type": "tool-call",
                "toolCallId": "call_early123",
                "toolName": "search",
                "input": "{\"query\": \"test\"}, \"limit\": 10}",
            }))
        );
        assert_eq!(tool_calls.len(), 1);
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should stream tool call with missing type field (Azure AI Foundry / Mistral)
    #[tokio::test]
    async fn should_stream_tool_call_with_missing_type_field() {
        let chunks = sse(&[
            r#"{"id":"chatcmpl-azure-001","object":"chat.completion.chunk","created":1711357598,"model":"mistral-large","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc123","function":{"name":"test-tool","arguments":""}}]},"finish_reason":null}]}"#.to_string(),
            r#"{"id":"chatcmpl-azure-001","object":"chat.completion.chunk","created":1711357598,"model":"mistral-large","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"value\""}}]},"finish_reason":null}]}"#.to_string(),
            r#"{"id":"chatcmpl-azure-001","object":"chat.completion.chunk","created":1711357598,"model":"mistral-large","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"hello\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#.to_string(),
        ]);

        let (parts, _) =
            stream_parts("gpt-3.5-turbo", stream_chunks(&chunks), tool_options()).await;

        // toMatchObject
        let start = parts
            .iter()
            .find(|part| part["type"] == "tool-input-start")
            .unwrap_or_else(|| panic!("no tool-input-start in {parts:?}"));
        assert_eq!(start["id"], "call_abc123");
        assert_eq!(start["toolName"], "test-tool");
        let call = parts
            .iter()
            .find(|part| part["type"] == "tool-call")
            .unwrap_or_else(|| panic!("no tool-call in {parts:?}"));
        assert_eq!(call["toolCallId"], "call_abc123");
        assert_eq!(call["toolName"], "test-tool");
        assert_eq!(call["input"], "{\"value\":\"hello\"}");
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should stream tool call that is sent in one chunk
    #[tokio::test]
    async fn should_stream_tool_call_that_is_sent_in_one_chunk() {
        let mut chunks = vec![tool_chunk(
            r#"{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_O17Uplv4lJvD6DVdIvFFeRMw","type":"function","function":{"name":"test-tool","arguments":"{\"value\":\"Sparkle Day\"}"}}]},"logprobs":null,"finish_reason":null}"#,
        )];
        chunks.extend(tool_stream_tail());

        let (parts, _) = stream_parts(
            "gpt-3.5-turbo",
            stream_chunks(&sse(&chunks)),
            tool_options(),
        )
        .await;

        let mut expected = vec![
            stream_start(),
            response_metadata(ID, "gpt-3.5-turbo-0125", "2024-03-25T09:06:38.000Z"),
        ];
        expected.extend(tool_parts(
            "call_O17Uplv4lJvD6DVdIvFFeRMw",
            "test-tool",
            &["{\"value\":\"Sparkle Day\"}"],
            "{\"value\":\"Sparkle Day\"}",
        ));
        expected.push(tool_finish());
        assert_eq!(parts, expected);
    }

    async fn early_error(chunk: &str) -> Box<aimux_core::ApiCallError> {
        let (result, _) = stream_result(
            "gpt-3.5-turbo",
            stream_chunks(&sse(&[chunk.to_string()])),
            no_raw(),
        )
        .await;
        match result {
            Err(AiMuxError::ApiCall(error)) => error,
            other => panic!("expected an API call error, got {:?}", other.map(|_| ())),
        }
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should throw an api error when the first stream chunk is an error
    #[tokio::test]
    async fn should_throw_an_api_error_when_the_first_stream_chunk_is_an_error() {
        let error = early_error(
            r#"{"error":{"message": "The server had an error processing your request. Sorry about that! You can retry your request, or contact us through our help center at help.openai.com if you keep seeing this error.","type":"server_error","param":null,"code":null}}"#,
        )
        .await;

        assert_eq!(
            error.message,
            "The server had an error processing your request. Sorry about that! You can retry your request, or contact us through our help center at help.openai.com if you keep seeing this error."
        );
        assert_eq!(error.status_code, Some(500));
        assert!(error.is_retryable);
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should preserve numeric status codes from early stream errors
    #[tokio::test]
    async fn should_preserve_numeric_status_codes_from_early_stream_errors() {
        let error = early_error(
            r#"{"error":{"message":"bad request","type":"provider_error","param":null,"code":400}}"#,
        )
        .await;

        assert_eq!(error.message, "bad request");
        assert_eq!(error.status_code, Some(400));
        assert!(!error.is_retryable);
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should forward error stream parts after output has started
    #[tokio::test]
    async fn should_forward_error_stream_parts_after_output_has_started() {
        let chunks = sse(&[
            r#"{"id":"chatcmpl-error-after-output","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#.to_string(),
            r#"{"error":{"message":"stream failed after output","type":"server_error","param":null,"code":null}}"#.to_string(),
        ]);

        let (parts, _) = stream_parts("gpt-3.5-turbo", stream_chunks(&chunks), no_raw()).await;

        let expected = [
            stream_start(),
            response_metadata(
                "chatcmpl-error-after-output",
                "gpt-3.5-turbo-0613",
                "2023-12-15T16:17:00.000Z",
            ),
            json!({ "id": "0", "type": "text-start" }),
            json!({ "delta": "Hello", "id": "0", "type": "text-delta" }),
            json!({
                "error": {
                    "data": {
                        "code": null,
                        "message": "stream failed after output",
                        "param": null,
                        "type": "server_error",
                    },
                    "isRetryable": true,
                    "message": "stream failed after output",
                    "statusCode": 500,
                    "type": "server_error",
                },
                "type": "error",
            }),
            json!({ "id": "0", "type": "text-end" }),
            error_finish(),
        ];
        assert_eq!(parts, expected.map(without_error_type));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should handle unparsable stream parts
    #[tokio::test]
    async fn should_handle_unparsable_stream_parts() {
        let chunks = vec![
            "data: {unparsable}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ];

        let (parts, _) = stream_parts("gpt-3.5-turbo", stream_chunks(&chunks), no_raw()).await;

        assert_eq!(parts.len(), 3, "{parts:?}");
        assert_eq!(parts[0], stream_start());
        // `[AI_JSONParseError: JSON parsing failed: Text: {unparsable}.
        // Error message: ...]`; the JS SyntaxError text is runtime-specific.
        assert_eq!(parts[1]["type"], "error", "{parts:?}");
        assert_eq!(parts[1]["error"]["name"], "AI_JSONParseError", "{parts:?}");
        assert!(
            parts[1]["error"]["message"]
                .as_str()
                .is_some_and(|message| message
                    .starts_with("JSON parsing failed: Text: {unparsable}.\nError message: ")),
            "{parts:?}"
        );
        assert_eq!(parts[2], error_finish());
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let (result, _) = stream(
            "gpt-3.5-turbo",
            chunks_fixture(OPENAI_TEXT_CHUNKS),
            no_raw(),
        )
        .await;

        assert_eq!(
            json!(result.request),
            json!({ "body": hello_stream_body("gpt-3.5-turbo") })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let model = "gpt-3.5-turbo-0613";
        let chunks = sse(&[
            text_chunk(model, 0, r#"{"role":"assistant","content":""}"#, "null", ""),
            text_chunk(model, 0, "{}", r#""stop""#, r#","logprobs":null"#),
            usage_chunk(
                model,
                r#"{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}"#,
            ),
        ]);

        let (result, _) = stream(
            "gpt-3.5-turbo",
            stream_chunks_with_headers(&chunks, &[("test-header", "test-value")]),
            no_raw(),
        )
        .await;

        assert_eq!(
            json!(result.response.unwrap().headers),
            json!({
                "content-type": "text/event-stream",
                "cache-control": "no-cache",
                "connection": "keep-alive",
                "test-header": "test-value",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should pass the messages and the model
    #[tokio::test]
    async fn should_pass_the_messages_and_the_model() {
        let (_, mock) = stream(
            "gpt-3.5-turbo",
            chunks_fixture(OPENAI_TEXT_CHUNKS),
            no_raw(),
        )
        .await;

        assert_eq!(request_body(&mock), hello_stream_body("gpt-3.5-turbo"));
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![chunks_fixture(OPENAI_TEXT_CHUNKS)]);
        let provider = provider_with(
            &mock,
            OpenAIProviderSettings {
                organization: Some("test-organization".into()),
                project: Some("test-project".into()),
                headers: Some(
                    [(
                        "Custom-Provider-Header".to_string(),
                        Some("provider-header-value".to_string()),
                    )]
                    .into_iter()
                    .collect(),
                ),
                ..OpenAIProviderSettings::default()
            },
        );

        provider
            .chat("gpt-3.5-turbo")
            .do_stream(&call_options(json!({
                "headers": { "Custom-Request-Header": "request-header-value" },
                "includeRawChunks": false,
            })))
            .await
            .unwrap();

        let (headers, _) = request_headers(&mock);
        assert_eq!(
            headers,
            json!({
                "authorization": "Bearer test-api-key",
                "content-type": "application/json",
                "custom-provider-header": "provider-header-value",
                "custom-request-header": "request-header-value",
                "openai-organization": "test-organization",
                "openai-project": "test-project",
            })
        );
    }

    async fn final_part(usage: &str) -> (Value, Value) {
        let model = "gpt-3.5-turbo-0613";
        let chunks = sse(&[
            text_chunk(model, 0, r#"{"role":"assistant","content":""}"#, "null", ""),
            text_chunk(model, 0, "{}", r#""stop""#, r#","logprobs":null"#),
            usage_chunk(model, usage),
        ]);
        let (parts, mock) = stream_parts("gpt-3.5-turbo", stream_chunks(&chunks), no_raw()).await;
        (request_body(&mock), parts.last().cloned().unwrap())
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should return cached tokens in providerMetadata
    #[tokio::test]
    async fn should_return_cached_tokens_in_provider_metadata() {
        let (body, last) = final_part(
            r#"{"prompt_tokens":2000,"completion_tokens":20,"total_tokens":2020,"prompt_tokens_details":{"cached_tokens":1152,"cache_write_tokens":256}}"#,
        )
        .await;

        assert_eq!(body, hello_stream_body("gpt-3.5-turbo"));
        assert_eq!(
            last,
            json!({
                "finishReason": { "raw": "stop", "unified": "stop" },
                "providerMetadata": { "openai": {} },
                "type": "finish",
                "usage": {
                    "inputTokens": { "cacheRead": 1152, "cacheWrite": 256, "noCache": 592, "total": 2000 },
                    "outputTokens": { "reasoning": 0, "text": 20, "total": 20 },
                    "raw": {
                        "completion_tokens": 20,
                        "prompt_tokens": 2000,
                        "prompt_tokens_details": { "cache_write_tokens": 256, "cached_tokens": 1152 },
                        "total_tokens": 2020,
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should return accepted_prediction_tokens and rejected_prediction_tokens in providerMetadata
    #[tokio::test]
    async fn should_return_accepted_and_rejected_prediction_tokens_in_provider_metadata() {
        let (body, last) = final_part(
            r#"{"prompt_tokens":15,"completion_tokens":20,"total_tokens":35,"completion_tokens_details":{"accepted_prediction_tokens":123,"rejected_prediction_tokens":456}}"#,
        )
        .await;

        assert_eq!(body, hello_stream_body("gpt-3.5-turbo"));
        assert_eq!(
            last,
            json!({
                "finishReason": { "raw": "stop", "unified": "stop" },
                "providerMetadata": {
                    "openai": { "acceptedPredictionTokens": 123, "rejectedPredictionTokens": 456 },
                },
                "type": "finish",
                "usage": finish_usage(
                    15,
                    20,
                    json!({
                        "completion_tokens": 20,
                        "completion_tokens_details": {
                            "accepted_prediction_tokens": 123,
                            "rejected_prediction_tokens": 456,
                        },
                        "prompt_tokens": 15,
                        "total_tokens": 35,
                    }),
                ),
            })
        );
    }

    async fn stream_extension(model_id: &str, options: Value, extra: Value) {
        let (_, mock) = stream(
            model_id,
            chunks_fixture(OPENAI_TEXT_CHUNKS),
            json!({ "providerOptions": { "openai": options }, "includeRawChunks": false }),
        )
        .await;

        assert_eq!(
            request_body(&mock),
            with(hello_stream_body(model_id), extra)
        );
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should send store extension setting
    #[tokio::test]
    async fn should_send_store_extension_setting() {
        stream_extension(
            "gpt-3.5-turbo",
            json!({ "store": true }),
            json!({ "store": true }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should send metadata extension values
    #[tokio::test]
    async fn should_send_metadata_extension_values() {
        stream_extension(
            "gpt-3.5-turbo",
            json!({ "metadata": { "custom": "value" } }),
            json!({ "metadata": { "custom": "value" } }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should send serviceTier flex processing setting in streaming
    #[tokio::test]
    async fn should_send_service_tier_flex_processing_setting_in_streaming() {
        stream_extension(
            "o4-mini",
            json!({ "serviceTier": "flex" }),
            json!({ "service_tier": "flex" }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should send serviceTier priority processing setting in streaming
    #[tokio::test]
    async fn should_send_service_tier_priority_processing_setting_in_streaming() {
        stream_extension(
            "gpt-4o-mini",
            json!({ "serviceTier": "priority" }),
            json!({ "service_tier": "priority" }),
        )
        .await;
    }

    /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › should set .modelId for model-router request
    #[tokio::test]
    async fn should_set_model_id_for_model_router_request() {
        let (parts, _) = stream_parts(
            "test-azure-model-router",
            chunks_fixture(AZURE_MODEL_ROUTER_CHUNKS),
            json!({}),
        )
        .await;

        let mut snapshot: Value = serde_json::from_str(MODEL_ROUTER_SNAPSHOT).unwrap();
        // Upstream's `usage.raw` is the zod-parsed usage, which strips keys the
        // schema does not know (`audio_tokens`). aimux keeps `Usage.raw` as the
        // provider's verbatim usage object (RFC-0016), so the snapshot's raw is
        // checked as a subset of the fixture's own usage, which replaces it.
        let fixture_usage: Value = serde_json::from_str(
            AZURE_MODEL_ROUTER_CHUNKS
                .lines()
                .rfind(|line| !line.trim().is_empty())
                .unwrap(),
        )
        .unwrap();
        let fixture_usage = fixture_usage["usage"].clone();
        let finish = snapshot.as_array_mut().unwrap().last_mut().unwrap();
        assert_eq!(finish["type"], "finish");
        let raw = &mut finish["usage"]["raw"];
        for (key, value) in raw.as_object().unwrap() {
            if let Some(details) = value.as_object() {
                for (detail, value) in details {
                    assert_eq!(&fixture_usage[key][detail], value, "{key}.{detail}");
                }
            } else {
                assert_eq!(&fixture_usage[key], value, "{key}");
            }
        }
        *raw = fixture_usage;
        assert_eq!(Value::Array(parts), snapshot);
    }

    mod reasoning_models {
        use super::*;

        async fn o4_mini_parts(usage: &str) -> Vec<Value> {
            let model = "o4-mini";
            let chunks = sse(&[
                text_chunk(model, 0, r#"{"role":"assistant","content":""}"#, "null", ""),
                text_chunk(model, 1, r#"{"content":"Hello, World!"}"#, "null", ""),
                text_chunk(model, 0, "{}", r#""stop""#, r#","logprobs":null"#),
                usage_chunk(model, usage),
            ]);
            stream_parts("o4-mini", stream_chunks(&chunks), no_raw())
                .await
                .0
        }

        fn expected(usage: Value) -> Vec<Value> {
            vec![
                stream_start(),
                response_metadata(ID, "o4-mini", "2023-12-15T16:17:00.000Z"),
                json!({ "id": "0", "type": "text-start" }),
                json!({ "delta": "", "id": "0", "type": "text-delta" }),
                json!({ "delta": "Hello, World!", "id": "0", "type": "text-delta" }),
                json!({ "id": "0", "type": "text-end" }),
                json!({
                    "finishReason": { "raw": "stop", "unified": "stop" },
                    "providerMetadata": { "openai": {} },
                    "type": "finish",
                    "usage": usage,
                }),
            ]
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › reasoning models › should stream text delta
        #[tokio::test]
        async fn should_stream_text_delta() {
            let parts =
                o4_mini_parts(r#"{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}"#)
                    .await;

            assert_eq!(
                parts,
                expected(finish_usage(
                    17,
                    227,
                    json!({ "completion_tokens": 227, "prompt_tokens": 17, "total_tokens": 244 }),
                ))
            );
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › reasoning models › should send reasoning tokens
        #[tokio::test]
        async fn should_send_reasoning_tokens() {
            let parts = o4_mini_parts(
                r#"{"prompt_tokens":15,"completion_tokens":20,"total_tokens":35,"completion_tokens_details":{"reasoning_tokens":10}}"#,
            )
            .await;

            assert_eq!(
                parts,
                expected(json!({
                    "inputTokens": { "cacheRead": 0, "noCache": 15, "total": 15 },
                    "outputTokens": { "reasoning": 10, "text": 10, "total": 20 },
                    "raw": {
                        "completion_tokens": 20,
                        "completion_tokens_details": { "reasoning_tokens": 10 },
                        "prompt_tokens": 15,
                        "total_tokens": 35,
                    },
                }))
            );
        }
    }

    mod raw_chunks {
        use super::*;

        fn raw_chunks() -> Vec<String> {
            let model = "gpt-3.5-turbo-0613";
            sse(&[
                text_chunk(model, 0, r#"{"role":"assistant","content":""}"#, "null", ""),
                text_chunk(model, 1, r#"{"content":"Hello"}"#, "null", ""),
                text_chunk(model, 1, r#"{"content":" World!"}"#, "null", ""),
                text_chunk(model, 0, "{}", r#""stop""#, r#","logprobs":null"#),
                usage_chunk(
                    model,
                    r#"{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}"#,
                ),
            ])
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › raw chunks › should include raw chunks when includeRawChunks is enabled
        #[tokio::test]
        async fn should_include_raw_chunks_when_include_raw_chunks_is_enabled() {
            let (parts, _) = stream_parts(
                "gpt-3.5-turbo",
                stream_chunks(&raw_chunks()),
                json!({ "includeRawChunks": true }),
            )
            .await;

            let raw: Vec<Value> = parts
                .into_iter()
                .filter(|part| part["type"] == "raw")
                .collect();
            let chunk = |choice: Value| {
                json!({
                    "choices": [choice],
                    "created": 1702657020,
                    "id": ID,
                    "model": "gpt-3.5-turbo-0613",
                    "object": "chat.completion.chunk",
                    "system_fingerprint": null,
                })
            };
            let expected: Vec<Value> = [
                chunk(json!({ "delta": { "content": "", "role": "assistant" }, "finish_reason": null, "index": 0 })),
                chunk(json!({ "delta": { "content": "Hello" }, "finish_reason": null, "index": 1 })),
                chunk(json!({ "delta": { "content": " World!" }, "finish_reason": null, "index": 1 })),
                chunk(json!({ "delta": {}, "finish_reason": "stop", "index": 0, "logprobs": null })),
                json!({
                    "choices": [],
                    "created": 1702657020,
                    "id": ID,
                    "model": "gpt-3.5-turbo-0613",
                    "object": "chat.completion.chunk",
                    "system_fingerprint": "fp_3bc1b5746c",
                    "usage": { "completion_tokens": 227, "prompt_tokens": 17, "total_tokens": 244 },
                }),
            ]
            .into_iter()
            .map(|raw_value| json!({ "rawValue": raw_value, "type": "raw" }))
            .collect();
            assert_eq!(raw, expected);
        }

        /// upstream: @ai-sdk/openai src/chat/openai-chat-language-model.test.ts › doStream › raw chunks › should not include raw chunks when includeRawChunks is false
        #[tokio::test]
        async fn should_not_include_raw_chunks_when_include_raw_chunks_is_false() {
            let (parts, _) = stream_parts(
                "gpt-3.5-turbo",
                stream_chunks(&raw_chunks()),
                json!({ "includeRawChunks": false }),
            )
            .await;

            assert_eq!(parts.iter().filter(|part| part["type"] == "raw").count(), 0);
        }
    }
}

// ── Older ports (previously the inline tests of `src/openai/model.rs`) ──

// The Chat Completions stream driven chunk by chunk.
mod older_stream {
    use serde_json::json;

    use crate::stream::testing::run;

    use crate::openai::chat::chat_language_model::*;
    use aimux_core::error::AiMuxError;
    use serde_json::Value;

    fn delta(delta: Value) -> Result<Value, AiMuxError> {
        Ok(json!({ "choices": [{ "delta": delta }] }))
    }

    fn tool_calls(tool_calls: Value) -> Result<Value, AiMuxError> {
        delta(json!({ "tool_calls": tool_calls }))
    }

    #[test]
    fn interleaved_tool_calls_stream_as_they_arrive_and_end_on_flush_in_index_order() {
        let parts = run(
            OpenAIChatStream::default(),
            [
                tool_calls(json!([{
                    "index": 1, "id": "call_b", "type": "function",
                    "function": { "name": "time", "arguments": "{" }
                }])),
                tool_calls(json!([{
                    "index": 0, "id": "call_a", "type": "function",
                    "function": { "name": "weather", "arguments": "{\"city\":" }
                }])),
                tool_calls(json!([
                    { "index": 1, "function": { "arguments": "}" } },
                    { "index": 0, "function": { "arguments": "\"Paris\"}" } }
                ])),
                Ok(json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] })),
            ],
        );
        assert_eq!(
            parts,
            [
                "stream-start",
                "tool-input-start call_b time",
                "tool-input-delta call_b {",
                "tool-input-start call_a weather",
                "tool-input-delta call_a {\"city\":",
                "tool-input-delta call_b }",
                "tool-input-delta call_a \"Paris\"}",
                "tool-input-end call_a",
                "tool-call call_a weather {\"city\":\"Paris\"}",
                "tool-input-end call_b",
                "tool-call call_b time {}",
                "finish ToolCalls",
            ]
        );
    }

    /// partial: `should forward error stream parts after output has started`; full port in src/openai/tests/chat/chat_language_model.rs.
    #[test]
    fn an_error_frame_is_an_error_part_and_the_stream_finishes_with_error() {
        let parts = run(
            OpenAIChatStream::default(),
            [
                delta(json!({ "content": "Hi" })),
                Ok(json!({ "error": { "message": "overloaded", "type": "server_error" } })),
            ],
        );
        assert_eq!(parts.len(), 6, "{parts:?}");
        assert_eq!(
            parts[..3],
            ["stream-start", "text-start 0", "text-delta 0 Hi"]
        );
        assert!(parts[3].starts_with("error ") && parts[3].contains("overloaded"));
        assert_eq!(parts[4..], ["text-end 0", "finish Error"]);
    }

    #[test]
    fn flush_closes_open_blocks_and_finishes_with_other_without_a_finish_reason() {
        let parts = run(
            OpenAIChatStream::default(),
            [
                delta(json!({ "reasoning_content": "Hmm" })),
                delta(json!({ "content": "Hi" })),
            ],
        );
        assert_eq!(
            parts,
            [
                "stream-start",
                "reasoning-start reasoning-0",
                "reasoning-delta reasoning-0 Hmm",
                "reasoning-end reasoning-0",
                "text-start 0",
                "text-delta 0 Hi",
                "text-end 0",
                "finish Other",
            ]
        );
    }

    #[test]
    fn a_tool_call_of_another_type_ends_the_stream_without_flush() {
        let parts = run(
            OpenAIChatStream::default(),
            [
                tool_calls(json!([{
                    "index": 0, "id": "call_a", "type": "custom",
                    "function": { "name": "weather", "arguments": "{}" }
                }])),
                delta(json!({ "content": "ignored" })),
            ],
        );
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert_eq!(parts[0], "stream-start");
        assert!(
            parts[1].starts_with("err ") && parts[1].contains("Expected 'function' type."),
            "{parts:?}"
        );
    }
}
