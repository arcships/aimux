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
const COMPLETIONS_URL: &str = "https://api.openai.com/v1/completions";
/// The pinned user agent (upstream mocks the version as `0.0.0-test`).
const USER_AGENT: &str = "ai-sdk-openai/4.0.83";

const COMPLETION_TEXT_JSON: &str = include_str!("fixtures/openai-completion-text.json");
const COMPLETION_TEXT_CHUNKS: &str = include_str!("fixtures/openai-completion-text.chunks.txt");

fn test_prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

fn test_logprobs() -> Value {
    json!({
        "tokens": [" ever", " after", ".\n\n", "The", " end", "."],
        "token_logprobs": [
            -0.0664508, -0.014520033, -1.3820221, -0.7890417, -0.5323165, -0.10247037,
        ],
        "top_logprobs": [
            { " ever": -0.0664508 },
            { " after": -0.014520033 },
            { ".\n\n": -1.3820221 },
            { "The": -0.7890417 },
            { " end": -0.5323165 },
            { ".": -0.10247037 },
        ],
    })
}

/// The test server's `json-value` response (`content-type`, `content-length`
/// and the case's extra headers).
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

/// `data: <chunk>` events followed by `[DONE]`.
fn sse(chunks: &[String]) -> Vec<String> {
    let mut events: Vec<String> = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    events.push("data: [DONE]\n\n".into());
    events
}

/// `prepareChunksFixtureResponse('openai-completion-text')`.
fn chunks_fixture() -> Canned {
    let lines: Vec<String> = COMPLETION_TEXT_CHUNKS
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    stream_chunks(&sse(&lines))
}

/// `prepareJsonFixtureResponse('openai-completion-text')`.
fn json_fixture() -> Canned {
    json_value(&serde_json::from_str(COMPLETION_TEXT_JSON).unwrap())
}

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

fn call_options(options: Value) -> CallOptions {
    let mut options = options;
    if options.get("prompt").is_none() {
        options["prompt"] = test_prompt();
    }
    serde_json::from_value(options).expect("valid call options")
}

async fn generate(
    model_id: &str,
    canned: Canned,
    options: Value,
) -> (GenerateResult, Arc<MockFetch>) {
    let mock = MockFetch::new(vec![canned]);
    let result = provider(&mock)
        .completion(model_id)
        .do_generate(&call_options(options))
        .await
        .expect("doGenerate succeeds");
    (result, mock)
}

async fn stream_result(
    model_id: &str,
    canned: Canned,
    options: Value,
) -> (Result<StreamResult, AiMuxError>, Arc<MockFetch>) {
    let mock = MockFetch::new(vec![canned]);
    let result = provider(&mock)
        .completion(model_id)
        .do_stream(&call_options(options))
        .await;
    (result, mock)
}

async fn stream(model_id: &str, canned: Canned, options: Value) -> (StreamResult, Arc<MockFetch>) {
    let (result, mock) = stream_result(model_id, canned, options).await;
    (result.expect("doStream succeeds"), mock)
}

async fn stream_parts(model_id: &str, canned: Canned, options: Value) -> Vec<Value> {
    let (result, _) = stream(model_id, canned, options).await;
    result.stream.map(part_json).collect().await
}

/// A stream part as its AI SDK JSON (see the chat model tests).
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
    assert_eq!(seen[0].url, COMPLETIONS_URL);
    seen[0].json_body()
}

fn request_headers(mock: &MockFetch) -> (Value, String) {
    let mut headers = mock.seen()[0].headers.clone();
    let user_agent = headers.remove("user-agent").unwrap_or_default();
    (json!(headers), user_agent)
}

fn completion(choice: Value) -> Value {
    json!({
        "id": "cmpl-96cAM1v77r4jXa4qb2NSmRREV5oWB",
        "object": "text_completion",
        "created": 1711363706,
        "model": "gpt-3.5-turbo-instruct",
        "choices": [choice],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
    })
}

fn prompt_body() -> Value {
    json!({
        "model": "gpt-3.5-turbo-instruct",
        "prompt": "user:\nHello\n\nassistant:\n",
        "stop": ["\nuser:"],
    })
}

fn prompt_stream_body() -> Value {
    let mut body = prompt_body();
    body["stream"] = json!(true);
    body["stream_options"] = json!({ "include_usage": true });
    body
}

fn custom_header_provider(mock: &Arc<MockFetch>) -> OpenAIProvider {
    provider_with(
        mock,
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
    )
}

fn expected_headers() -> Value {
    json!({
        "authorization": "Bearer test-api-key",
        "content-type": "application/json",
        "custom-provider-header": "provider-header-value",
        "custom-request-header": "request-header-value",
        "openai-organization": "test-organization",
        "openai-project": "test-project",
    })
}

fn stream_start() -> Value {
    json!({ "type": "stream-start", "warnings": [] })
}

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

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should extract text response
    #[tokio::test]
    async fn should_extract_text_response() {
        let body =
            completion(json!({ "text": "Hello, World!", "index": 0, "finish_reason": "stop" }));

        let (result, _) = generate("gpt-3.5-turbo-instruct", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.content),
            json!([{ "text": "Hello, World!", "type": "text" }])
        );
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should extract usage
    #[tokio::test]
    async fn should_extract_usage() {
        let mut body = completion(json!({ "text": "", "index": 0, "finish_reason": "stop" }));
        body["usage"] = json!({ "prompt_tokens": 20, "total_tokens": 25, "completion_tokens": 5 });

        let (result, _) = generate("gpt-3.5-turbo-instruct", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.usage),
            json!({
                "inputTokens": { "noCache": 20, "total": 20 },
                "outputTokens": { "text": 5, "total": 5 },
                "raw": { "completion_tokens": 5, "prompt_tokens": 20, "total_tokens": 25 },
            })
        );
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let (result, _) = generate("gpt-3.5-turbo-instruct", json_fixture(), json!({})).await;

        assert_eq!(json!(result.request), json!({ "body": prompt_body() }));
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should send additional response information
    #[tokio::test]
    async fn should_send_additional_response_information() {
        let body = json!({
            "id": "test-id",
            "object": "text_completion",
            "created": 123,
            "model": "test-model",
            "choices": [{ "text": "", "index": 0, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
        });

        let (result, _) = generate("gpt-3.5-turbo-instruct", json_value(&body), json!({})).await;

        let response = json!(result.response);
        assert_eq!(
            json!({
                "id": response["id"],
                "timestamp": response["timestamp"],
                "modelId": response["modelId"],
            }),
            json!({
                "id": "test-id",
                "timestamp": "1970-01-01T00:02:03.000Z",
                "modelId": "test-model",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should extract logprobs
    #[tokio::test]
    async fn should_extract_logprobs() {
        let body = completion(json!({
            "text": "",
            "index": 0,
            "logprobs": test_logprobs(),
            "finish_reason": "stop",
        }));

        let (result, _) = generate(
            "gpt-3.5-turbo",
            json_value(&body),
            json!({ "providerOptions": { "openai": { "logprobs": 1 } } }),
        )
        .await;

        assert_eq!(
            json!(result.provider_metadata)["openai"]["logprobs"],
            test_logprobs()
        );
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should extract finish reason
    #[tokio::test]
    async fn should_extract_finish_reason() {
        let body = completion(json!({ "text": "", "index": 0, "finish_reason": "stop" }));

        let (result, _) = generate("gpt-3.5-turbo-instruct", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.finish_reason),
            json!({ "raw": "stop", "unified": "stop" })
        );
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should support unknown finish reason
    #[tokio::test]
    async fn should_support_unknown_finish_reason() {
        let body = completion(json!({ "text": "", "index": 0, "finish_reason": "eos" }));

        let (result, _) = generate("gpt-3.5-turbo-instruct", json_value(&body), json!({})).await;

        assert_eq!(
            json!(result.finish_reason),
            json!({ "raw": "eos", "unified": "other" })
        );
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let body = completion(json!({ "text": "", "index": 0, "finish_reason": "stop" }));

        let (result, _) = generate(
            "gpt-3.5-turbo-instruct",
            json_value_with_headers(&body, &[("test-header", "test-value")]),
            json!({}),
        )
        .await;

        assert_eq!(
            json!(result.response.unwrap().headers),
            json!({
                "content-length": "250",
                "content-type": "application/json",
                "test-header": "test-value",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should pass the model and the prompt
    #[tokio::test]
    async fn should_pass_the_model_and_the_prompt() {
        let (_, mock) = generate("gpt-3.5-turbo-instruct", json_fixture(), json!({})).await;

        assert_eq!(request_body(&mock), prompt_body());
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![json_fixture()]);

        custom_header_provider(&mock)
            .completion("gpt-3.5-turbo-instruct")
            .do_generate(&call_options(json!({
                "headers": { "Custom-Request-Header": "request-header-value" },
            })))
            .await
            .unwrap();

        assert_eq!(request_headers(&mock).0, expected_headers());
    }
}

mod do_stream {
    use super::*;

    fn no_raw() -> Value {
        json!({ "includeRawChunks": false })
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doStream › should stream text deltas
    #[tokio::test]
    async fn should_stream_text_deltas() {
        let logprobs = test_logprobs();
        let text = |id: &str, created: u32, text: &str, finish: &str| {
            format!(
                r#"{{"id":"{id}","object":"text_completion","created":{created},"choices":[{{"text":{},"index":0,"logprobs":{logprobs},"finish_reason":{finish}}}],"model":"gpt-3.5-turbo-instruct"}}"#,
                json!(text)
            )
        };
        let chunks = sse(&[
            text("cmpl-96c64EdfhOw8pjFFgVpLuT8k2MtdT", 1711363440, "Hello", "null"),
            text("cmpl-96c64EdfhOw8pjFFgVpLuT8k2MtdT", 1711363440, ", ", "null"),
            text("cmpl-96c64EdfhOw8pjFFgVpLuT8k2MtdT", 1711363440, "World!", "null"),
            text("cmpl-96c3yLQE1TtZCd6n6OILVmzev8M8H", 1711363310, "", r#""stop""#),
            r#"{"id":"cmpl-96c3yLQE1TtZCd6n6OILVmzev8M8H","object":"text_completion","created":1711363310,"model":"gpt-3.5-turbo-instruct","usage":{"prompt_tokens":10,"total_tokens":372,"completion_tokens":362},"choices":[]}"#.to_string(),
        ]);

        let parts = stream_parts("gpt-3.5-turbo-instruct", stream_chunks(&chunks), no_raw()).await;

        assert_eq!(
            parts,
            [
                stream_start(),
                json!({
                    "id": "cmpl-96c64EdfhOw8pjFFgVpLuT8k2MtdT",
                    "modelId": "gpt-3.5-turbo-instruct",
                    "timestamp": "2024-03-25T10:44:00.000Z",
                    "type": "response-metadata",
                }),
                json!({ "id": "0", "type": "text-start" }),
                json!({ "delta": "Hello", "id": "0", "type": "text-delta" }),
                json!({ "delta": ", ", "id": "0", "type": "text-delta" }),
                json!({ "delta": "World!", "id": "0", "type": "text-delta" }),
                json!({ "id": "0", "type": "text-end" }),
                json!({
                    "finishReason": { "raw": "stop", "unified": "stop" },
                    "providerMetadata": { "openai": { "logprobs": test_logprobs() } },
                    "type": "finish",
                    "usage": {
                        "inputTokens": { "noCache": 10, "total": 10 },
                        "outputTokens": { "text": 362, "total": 362 },
                        "raw": { "completion_tokens": 362, "prompt_tokens": 10, "total_tokens": 372 },
                    },
                }),
            ]
        );
    }

    /// A chunk with `"error": null` is a normal chunk (#397): upstream's
    /// chunk schema is a union with `openaiErrorDataSchema`, and a chunk with
    /// `choices` takes the first branch, which drops the unknown key. On the
    /// first chunk the null member used to fail the call before any output.
    #[tokio::test]
    async fn a_null_error_member_is_not_an_error_frame() {
        let chunks = sse(&[
            r#"{"id":"cmpl-x","object":"text_completion","created":1711363440,"choices":[{"text":"hello","index":0,"logprobs":null,"finish_reason":null}],"model":"gpt-3.5-turbo-instruct","error":null}"#.to_string(),
            r#"{"id":"cmpl-x","object":"text_completion","created":1711363440,"choices":[{"text":"","index":0,"logprobs":null,"finish_reason":"stop"}],"model":"gpt-3.5-turbo-instruct"}"#.to_string(),
            r#"{"id":"cmpl-x","object":"text_completion","created":1711363440,"model":"gpt-3.5-turbo-instruct","usage":{"prompt_tokens":1,"total_tokens":2,"completion_tokens":1},"choices":[],"error":null}"#.to_string(),
        ]);

        let parts = stream_parts("gpt-3.5-turbo-instruct", stream_chunks(&chunks), no_raw()).await;

        assert!(
            parts.iter().all(|part| part["type"] != "error"),
            "{parts:?}"
        );
        assert!(
            parts.contains(&json!({ "delta": "hello", "id": "0", "type": "text-delta" })),
            "{parts:?}"
        );
        let finish = parts.last().unwrap();
        assert_eq!(finish["type"], "finish", "{parts:?}");
        assert_eq!(finish["finishReason"]["unified"], "stop", "{parts:?}");
        assert_eq!(finish["usage"]["outputTokens"]["total"], 1, "{parts:?}");
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doStream › should throw an api error when the first stream chunk is an error
    #[tokio::test]
    async fn should_throw_an_api_error_when_the_first_stream_chunk_is_an_error() {
        let chunks = sse(&[r#"{"error":{"message": "The server had an error processing your request. Sorry about that! You can retry your request, or contact us through our help center at help.openai.com if you keep seeing this error.","type":"server_error","param":null,"code":null}}"#.to_string()]);

        let (result, _) =
            stream_result("gpt-3.5-turbo-instruct", stream_chunks(&chunks), no_raw()).await;

        let Err(AiMuxError::ApiCall(error)) = result else {
            panic!("expected an API call error, got {:?}", result.map(|_| ()));
        };
        assert_eq!(
            error.message,
            "The server had an error processing your request. Sorry about that! You can retry your request, or contact us through our help center at help.openai.com if you keep seeing this error."
        );
        assert_eq!(error.status_code, Some(500));
        assert!(error.is_retryable);
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doStream › should forward error stream parts after output has started
    #[tokio::test]
    async fn should_forward_error_stream_parts_after_output_has_started() {
        let chunks = sse(&[
            r#"{"id":"cmpl-error-after-output","object":"text_completion","created":1711363440,"choices":[{"text":"Hello","index":0,"logprobs":null,"finish_reason":null}],"model":"gpt-3.5-turbo-instruct"}"#.to_string(),
            r#"{"error":{"message":"stream failed after output","type":"server_error","param":null,"code":null}}"#.to_string(),
        ]);

        let parts = stream_parts("gpt-3.5-turbo-instruct", stream_chunks(&chunks), no_raw()).await;

        let expected = [
            stream_start(),
            json!({
                "id": "cmpl-error-after-output",
                "modelId": "gpt-3.5-turbo-instruct",
                "timestamp": "2024-03-25T10:44:00.000Z",
                "type": "response-metadata",
            }),
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

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doStream › should handle unparsable stream parts
    #[tokio::test]
    async fn should_handle_unparsable_stream_parts() {
        let chunks = vec![
            "data: {unparsable}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ];

        let parts = stream_parts("gpt-3.5-turbo-instruct", stream_chunks(&chunks), no_raw()).await;

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

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doStream › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let (result, _) = stream("gpt-3.5-turbo-instruct", chunks_fixture(), no_raw()).await;

        assert_eq!(
            json!(result.request),
            json!({ "body": prompt_stream_body() })
        );
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doStream › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let chunks = sse(&[
            r#"{"id":"cmpl-96c3yLQE1TtZCd6n6OILVmzev8M8H","object":"text_completion","created":1711363310,"choices":[{"text":"","index":0,"logprobs":null,"finish_reason":"stop"}],"model":"gpt-3.5-turbo-instruct"}"#.to_string(),
            r#"{"id":"cmpl-96c3yLQE1TtZCd6n6OILVmzev8M8H","object":"text_completion","created":1711363310,"model":"gpt-3.5-turbo-instruct","usage":{"prompt_tokens":10,"total_tokens":372,"completion_tokens":362},"choices":[]}"#.to_string(),
        ]);

        let (result, _) = stream(
            "gpt-3.5-turbo-instruct",
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

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doStream › should pass the model and the prompt
    #[tokio::test]
    async fn should_pass_the_model_and_the_prompt() {
        let (_, mock) = stream("gpt-3.5-turbo-instruct", chunks_fixture(), no_raw()).await;

        assert_eq!(request_body(&mock), prompt_stream_body());
    }

    /// upstream: @ai-sdk/openai src/completion/openai-completion-language-model.test.ts › doStream › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![chunks_fixture()]);

        custom_header_provider(&mock)
            .completion("gpt-3.5-turbo-instruct")
            .do_stream(&call_options(json!({
                "headers": { "Custom-Request-Header": "request-header-value" },
                "includeRawChunks": false,
            })))
            .await
            .unwrap();

        let (headers, user_agent) = request_headers(&mock);
        assert_eq!(headers, expected_headers());
        assert!(user_agent.contains(USER_AGENT), "{user_agent}");
    }
}
