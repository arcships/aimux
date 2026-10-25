//! Port of `@ai-sdk/openai-compatible` 3.0.62
//! `src/completion/openai-compatible-completion-language-model.test.ts`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::StreamResult;
use aimux_core::stream_part::StreamPart;

use crate::openai_compatible::completion::{CompletionConfig, OpenAICompatibleCompletionModel};
use crate::openai_compatible::config::{BaseUrl, ChatSettings, CompatModelConfig};
use crate::openai_compatible::{
    OpenAICompatibleProvider, OpenAICompatibleProviderSettings, create_openai_compatible,
};
use crate::shared::{Credential, ProviderHeaders};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

/// The user agent aimux pins (upstream mocks `0.0.0-test`); compared apart
/// from the other request headers, which upstream's test server lists
/// without it.
const USER_AGENT: &str = "ai-sdk-openai-compatible/3.0.62 ai-sdk-provider-utils/5.0.53";

const PROMPT_TEXT: &str = "user:\nHello\n\nassistant:\n";

fn test_prompt() -> Vec<LanguageModelMessage> {
    vec![LanguageModelMessage::user_text("Hello")]
}

fn call_options() -> CallOptions {
    CallOptions::new(test_prompt())
}

fn stream_options() -> CallOptions {
    let mut options = call_options();
    options.include_raw_chunks = Some(false);
    options
}

fn with_provider_options(mut options: CallOptions, provider_options: Value) -> CallOptions {
    options.provider_options = Some(serde_json::from_value(provider_options).unwrap());
    options
}

fn header_map(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

/// `createOpenAICompatible({ baseURL: 'https://my.api.com/v1/', name:
/// 'test-provider', headers })` on the mock transport.
fn create_provider(mock: &Arc<MockFetch>, headers: &[(&str, &str)]) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        base_url: "https://my.api.com/v1/".into(),
        name: "test-provider".into(),
        headers: Some(
            headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
                .collect(),
        ),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// The module-level `provider` of the upstream file.
fn provider(mock: &Arc<MockFetch>) -> OpenAICompatibleProvider {
    create_provider(mock, &[("Authorization", "Bearer test-api-key")])
}

/// The module-level `model` of the upstream file.
fn model(mock: &Arc<MockFetch>) -> OpenAICompatibleCompletionModel {
    provider(mock).completion("gpt-3.5-turbo-instruct")
}

/// `new OpenAICompatibleCompletionLanguageModel(modelId, { provider, url: ()
/// => '', headers: () => ({}) })`.
fn model_with_provider(model_id: &str, provider: &str) -> OpenAICompatibleCompletionModel {
    OpenAICompatibleCompletionModel::from_config(
        model_id.to_string(),
        CompatModelConfig {
            provider: provider.to_string(),
            base_url: BaseUrl::Fixed(String::new()),
            query_params: None,
            headers: ProviderHeaders::bearer(Credential::None, Vec::new(), None),
            fetch_headers: None,
            fetch: None,
            transform_request_body: None,
            chat: ChatSettings::default(),
        },
    )
}

/// `model['providerOptionsName']`: the name the compatible config derives
/// from the provider string.
fn provider_options_name(model: &OpenAICompatibleCompletionModel) -> String {
    match &model.config {
        CompletionConfig::Compatible(config) => config.provider_options_name().to_string(),
        CompletionConfig::Native(_) => unreachable!("a compatible model"),
    }
}

/// A `json-value` response of the test server: `content-type` and
/// `content-length` are its default headers.
fn json_response(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let body = serde_json::to_vec(body).unwrap();
    let mut all = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), body.len().to_string()),
    ];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    Canned {
        status: 200,
        headers: all,
        body,
    }
}

/// A `stream-chunks` response of the test server with its default headers.
fn stream_response(chunks: &[&str], headers: &[(&str, &str)]) -> Canned {
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

/// The request headers without `user-agent`, which is checked against the
/// pinned value.
fn request_headers(seen: &Seen) -> BTreeMap<String, String> {
    let mut headers = seen.headers.clone();
    assert_eq!(
        headers.remove("user-agent").as_deref(),
        Some(USER_AGENT),
        "pinned user agent"
    );
    headers
}

fn expected_headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

/// One stream part as upstream's snapshot shows it. An error part of an
/// `{"error": ...}` chunk carries that chunk's `error` value upstream; aimux
/// wraps it in an API call error whose `data` is that value.
fn part_json(part: StreamPart) -> Value {
    match part {
        StreamPart::Error {
            error: AiMuxError::ApiCall(error),
        } => json!({ "type": "error", "error": error.data }),
        other => serde_json::to_value(other).unwrap(),
    }
}

async fn stream_parts(result: StreamResult) -> Vec<Value> {
    result
        .stream
        .map(|part| part_json(part.expect("stream item")))
        .collect()
        .await
}

mod config {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › config › should extract base name from provider string
    #[test]
    fn should_extract_base_name_from_provider_string() {
        let model = model_with_provider("gpt-5", "anthropic.beta");

        assert_eq!(provider_options_name(&model), "anthropic");
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › config › should handle provider without dot notation
    #[test]
    fn should_handle_provider_without_dot_notation() {
        let model = model_with_provider("gpt-5", "openai");

        assert_eq!(provider_options_name(&model), "openai");
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › config › should return empty for empty provider
    #[test]
    fn should_return_empty_for_empty_provider() {
        let model = model_with_provider("gpt-5", "");

        assert_eq!(provider_options_name(&model), "");
    }
}

mod do_generate {
    use super::*;

    /// The arguments of upstream's `prepareJsonResponse`.
    struct JsonResponse {
        content: &'static str,
        usage: Value,
        finish_reason: &'static str,
        id: &'static str,
        created: i64,
        model: &'static str,
        headers: &'static [(&'static str, &'static str)],
    }

    impl Default for JsonResponse {
        fn default() -> Self {
            Self {
                content: "",
                usage: json!({ "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }),
                finish_reason: "stop",
                id: "cmpl-96cAM1v77r4jXa4qb2NSmRREV5oWB",
                created: 1_711_363_706,
                model: "gpt-3.5-turbo-instruct",
                headers: &[],
            }
        }
    }

    fn prepare_json_response(response: JsonResponse) -> Arc<MockFetch> {
        MockFetch::new(vec![json_response(
            &json!({
                "id": response.id,
                "object": "text_completion",
                "created": response.created,
                "model": response.model,
                "choices": [{
                    "text": response.content,
                    "index": 0,
                    "finish_reason": response.finish_reason,
                }],
                "usage": response.usage,
            }),
            response.headers,
        )])
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should extract text response
    #[tokio::test]
    async fn should_extract_text_response() {
        let mock = prepare_json_response(JsonResponse {
            content: "Hello, World!",
            ..Default::default()
        });

        let result = model(&mock).do_generate(&call_options()).await.unwrap();

        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{ "text": "Hello, World!", "type": "text" }])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should extract usage
    #[tokio::test]
    async fn should_extract_usage() {
        let mock = prepare_json_response(JsonResponse {
            usage: json!({ "prompt_tokens": 20, "total_tokens": 25, "completion_tokens": 5 }),
            ..Default::default()
        });

        let result = model(&mock).do_generate(&call_options()).await.unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "noCache": 20, "total": 20 },
                "outputTokens": { "text": 5, "total": 5 },
                "raw": { "completion_tokens": 5, "prompt_tokens": 20, "total_tokens": 25 },
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let mock = prepare_json_response(JsonResponse::default());

        let result = model(&mock).do_generate(&call_options()).await.unwrap();

        assert_eq!(
            serde_json::to_value(&result.request).unwrap(),
            json!({
                "body": {
                    "model": "gpt-3.5-turbo-instruct",
                    "prompt": PROMPT_TEXT,
                    "stop": ["\nuser:"],
                },
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should send additional response information
    #[tokio::test]
    async fn should_send_additional_response_information() {
        let mock = prepare_json_response(JsonResponse {
            id: "test-id",
            created: 123,
            model: "test-model",
            ..Default::default()
        });

        let result = model(&mock).do_generate(&call_options()).await.unwrap();

        assert_eq!(
            serde_json::to_value(&result.response).unwrap(),
            json!({
                "body": {
                    "choices": [{ "finish_reason": "stop", "index": 0, "text": "" }],
                    "created": 123,
                    "id": "test-id",
                    "model": "test-model",
                    "object": "text_completion",
                    "usage": { "completion_tokens": 30, "prompt_tokens": 4, "total_tokens": 34 },
                },
                "headers": {
                    "content-length": "204",
                    "content-type": "application/json",
                },
                "id": "test-id",
                "modelId": "test-model",
                "timestamp": "1970-01-01T00:02:03.000Z",
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should extract finish reason
    #[tokio::test]
    async fn should_extract_finish_reason() {
        let mock = prepare_json_response(JsonResponse {
            finish_reason: "stop",
            ..Default::default()
        });

        let result = provider(&mock)
            .completion("gpt-3.5-turbo-instruct")
            .do_generate(&call_options())
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.finish_reason).unwrap(),
            json!({ "raw": "stop", "unified": "stop" })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should support unknown finish reason
    #[tokio::test]
    async fn should_support_unknown_finish_reason() {
        let mock = prepare_json_response(JsonResponse {
            finish_reason: "eos",
            ..Default::default()
        });

        let result = provider(&mock)
            .completion("gpt-3.5-turbo-instruct")
            .do_generate(&call_options())
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.finish_reason).unwrap(),
            json!({ "raw": "eos", "unified": "other" })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let mock = prepare_json_response(JsonResponse {
            headers: &[("test-header", "test-value")],
            ..Default::default()
        });

        let result = model(&mock).do_generate(&call_options()).await.unwrap();

        assert_eq!(
            result.response.unwrap().headers.unwrap(),
            header_map(&[
                // default headers:
                ("content-length", "250"),
                ("content-type", "application/json"),
                // custom header
                ("test-header", "test-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should pass the model and the prompt
    #[tokio::test]
    async fn should_pass_the_model_and_the_prompt() {
        let mock = prepare_json_response(JsonResponse::default());

        model(&mock).do_generate(&call_options()).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "gpt-3.5-turbo-instruct",
                "prompt": PROMPT_TEXT,
                "stop": ["\nuser:"],
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = prepare_json_response(JsonResponse::default());

        let provider = create_provider(
            &mock,
            &[
                ("Authorization", "Bearer test-api-key"),
                ("Custom-Provider-Header", "provider-header-value"),
            ],
        );

        let mut options = call_options();
        options.headers = Some(header_map(&[(
            "Custom-Request-Header",
            "request-header-value",
        )]));
        provider
            .completion("gpt-3.5-turbo-instruct")
            .do_generate(&options)
            .await
            .unwrap();

        assert_eq!(
            request_headers(&mock.seen()[0]),
            expected_headers(&[
                ("authorization", "Bearer test-api-key"),
                ("content-type", "application/json"),
                ("custom-provider-header", "provider-header-value"),
                ("custom-request-header", "request-header-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should include provider-specific options
    #[tokio::test]
    async fn should_include_provider_specific_options() {
        let mock = prepare_json_response(JsonResponse::default());

        provider(&mock)
            .completion("gpt-3.5-turbo-instruct")
            .do_generate(&with_provider_options(
                call_options(),
                json!({ "test-provider": { "someCustomOption": "test-value" } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "gpt-3.5-turbo-instruct",
                "prompt": PROMPT_TEXT,
                "someCustomOption": "test-value",
                "stop": ["\nuser:"],
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › should not include provider-specific options for different provider
    #[tokio::test]
    async fn should_not_include_provider_specific_options_for_different_provider() {
        let mock = prepare_json_response(JsonResponse::default());

        provider(&mock)
            .completion("gpt-3.5-turbo-instruct")
            .do_generate(&with_provider_options(
                call_options(),
                json!({ "notThisProviderName": { "someCustomOption": "test-value" } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "gpt-3.5-turbo-instruct",
                "prompt": PROMPT_TEXT,
                "stop": ["\nuser:"],
            })
        );
    }

    mod camel_case_provider_options {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › camelCase provider options › should accept camelCase provider options key for hyphenated provider name
        #[tokio::test]
        async fn should_accept_camel_case_provider_options_key_for_hyphenated_provider_name() {
            let mock = prepare_json_response(JsonResponse::default());

            provider(&mock)
                .completion("gpt-3.5-turbo-instruct")
                .do_generate(&with_provider_options(
                    call_options(),
                    json!({ "testProvider": { "someCustomOption": "test-value" } }),
                ))
                .await
                .unwrap();

            assert_eq!(
                mock.seen()[0].json_body()["someCustomOption"],
                json!("test-value")
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › camelCase provider options › should prefer camelCase options over raw-name options
        #[tokio::test]
        async fn should_prefer_camel_case_options_over_raw_name_options() {
            let mock = prepare_json_response(JsonResponse::default());

            provider(&mock)
                .completion("gpt-3.5-turbo-instruct")
                .do_generate(&with_provider_options(
                    call_options(),
                    json!({
                        "test-provider": { "someCustomOption": "raw-value" },
                        "testProvider": { "someCustomOption": "camel-value" },
                    }),
                ))
                .await
                .unwrap();

            assert_eq!(
                mock.seen()[0].json_body()["someCustomOption"],
                json!("camel-value")
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › camelCase provider options › should emit deprecated warning when raw provider options key is used
        #[tokio::test]
        async fn should_emit_deprecated_warning_when_raw_provider_options_key_is_used() {
            let mock = prepare_json_response(JsonResponse::default());

            let result = provider(&mock)
                .completion("gpt-3.5-turbo-instruct")
                .do_generate(&with_provider_options(
                    call_options(),
                    json!({ "test-provider": { "someCustomOption": "test-value" } }),
                ))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([{
                    "message": "Use 'testProvider' instead.",
                    "setting": "providerOptions key 'test-provider'",
                    "type": "deprecated",
                }])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doGenerate › camelCase provider options › should not emit deprecated warning when camelCase provider options key is used
        #[tokio::test]
        async fn should_not_emit_deprecated_warning_when_camel_case_provider_options_key_is_used() {
            let mock = prepare_json_response(JsonResponse::default());

            let result = provider(&mock)
                .completion("gpt-3.5-turbo-instruct")
                .do_generate(&with_provider_options(
                    call_options(),
                    json!({ "testProvider": { "someCustomOption": "test-value" } }),
                ))
                .await
                .unwrap();

            assert_eq!(serde_json::to_value(&result.warnings).unwrap(), json!([]));
        }
    }
}

mod do_stream {
    use super::*;

    fn prepare_empty_stream_response(headers: &[(&str, &str)]) -> Arc<MockFetch> {
        MockFetch::new(vec![stream_response(
            &[
                "data: {\"id\":\"cmpl-96c3yLQE1TtZCd6n6OILVmzev8M8H\",\"object\":\"text_completion\",\"created\":1711363310,\"model\":\"gpt-3.5-turbo-instruct\",\"choices\":[{\"text\":\"\",\"index\":0,\"logprobs\":null,\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"id\":\"cmpl-96c3yLQE1TtZCd6n6OILVmzev8M8H\",\"object\":\"text_completion\",\"created\":1711363310,\"model\":\"gpt-3.5-turbo-instruct\",\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":0,\"total_tokens\":10},\"choices\":[]}\n\n",
                "data: [DONE]\n\n",
            ],
            headers,
        )])
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should stream text deltas
    #[tokio::test]
    async fn should_stream_text_deltas() {
        let mock = MockFetch::new(vec![stream_response(
            &[
                "data: {\"id\":\"cmpl-96c64EdfhOw8pjFFgVpLuT8k2MtdT\",\"object\":\"text_completion\",\"created\":1711363440,\"model\":\"gpt-3.5-turbo-instruct\",\"choices\":[{\"text\":\"Hello\",\"index\":0,\"logprobs\":null,\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"cmpl-96c64EdfhOw8pjFFgVpLuT8k2MtdT\",\"object\":\"text_completion\",\"created\":1711363440,\"model\":\"gpt-3.5-turbo-instruct\",\"choices\":[{\"text\":\",\",\"index\":0,\"logprobs\":null,\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"cmpl-96c64EdfhOw8pjFFgVpLuT8k2MtdT\",\"object\":\"text_completion\",\"created\":1711363440,\"model\":\"gpt-3.5-turbo-instruct\",\"choices\":[{\"text\":\" World!\",\"index\":0,\"logprobs\":null,\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"cmpl-96c3yLQE1TtZCd6n6OILVmzev8M8H\",\"object\":\"text_completion\",\"created\":1711363310,\"model\":\"gpt-3.5-turbo-instruct\",\"choices\":[{\"text\":\"\",\"index\":0,\"logprobs\":null,\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"id\":\"cmpl-96c3yLQE1TtZCd6n6OILVmzev8M8H\",\"object\":\"text_completion\",\"created\":1711363310,\"model\":\"gpt-3.5-turbo-instruct\",\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":362,\"total_tokens\":372},\"choices\":[]}\n\n",
                "data: [DONE]\n\n",
            ],
            &[],
        )]);

        let result = model(&mock).do_stream(&stream_options()).await.unwrap();

        assert_eq!(
            stream_parts(result).await,
            vec![
                json!({ "type": "stream-start", "warnings": [] }),
                json!({
                    "id": "cmpl-96c64EdfhOw8pjFFgVpLuT8k2MtdT",
                    "modelId": "gpt-3.5-turbo-instruct",
                    "timestamp": "2024-03-25T10:44:00.000Z",
                    "type": "response-metadata",
                }),
                json!({ "id": "0", "type": "text-start" }),
                json!({ "delta": "Hello", "id": "0", "type": "text-delta" }),
                json!({ "delta": ",", "id": "0", "type": "text-delta" }),
                json!({ "delta": " World!", "id": "0", "type": "text-delta" }),
                json!({ "delta": "", "id": "0", "type": "text-delta" }),
                json!({ "id": "0", "type": "text-end" }),
                json!({
                    "finishReason": { "raw": "stop", "unified": "stop" },
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

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should handle error stream parts
    #[tokio::test]
    async fn should_handle_error_stream_parts() {
        let mock = MockFetch::new(vec![stream_response(
            &[
                "data: {\"error\":{\"message\":\"The server had an error processing your request. Sorry about that! You can retry your request, or contact us through our help center at help.openai.com if you keep seeing this error.\",\"type\":\"server_error\",\"param\":null,\"code\":null}}\n\n",
                "data: [DONE]\n\n",
            ],
            &[],
        )]);

        let result = model(&mock).do_stream(&stream_options()).await.unwrap();

        assert_eq!(
            stream_parts(result).await,
            vec![
                json!({ "type": "stream-start", "warnings": [] }),
                json!({
                    "error": {
                        "code": null,
                        "message": "The server had an error processing your request. Sorry about that! You can retry your request, or contact us through our help center at help.openai.com if you keep seeing this error.",
                        "param": null,
                        "type": "server_error",
                    },
                    "type": "error",
                }),
                json!({
                    "finishReason": { "unified": "error" },
                    "type": "finish",
                    "usage": { "inputTokens": {}, "outputTokens": {} },
                }),
            ]
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should handle unparsable stream parts
    #[tokio::test]
    async fn should_handle_unparsable_stream_parts() {
        let mock = MockFetch::new(vec![stream_response(
            &["data: {unparsable}\n\n", "data: [DONE]\n\n"],
            &[],
        )]);

        let result = model(&mock).do_stream(&stream_options()).await.unwrap();

        let mut parts = stream_parts(result).await;
        // Upstream's error is `AI_JSONParseError: JSON parsing failed: Text:
        // {unparsable}.\nError message: SyntaxError: ...`; the text after
        // `Error message:` is the JavaScript parser's own wording.
        let error = parts[1]["error"].take();
        assert_eq!(error["name"], json!("AI_JSONParseError"), "{error}");
        assert!(
            error["message"].as_str().is_some_and(|message| message
                .starts_with("JSON parsing failed: Text: {unparsable}.\nError message: ")),
            "{error}"
        );
        assert_eq!(
            parts,
            vec![
                json!({ "type": "stream-start", "warnings": [] }),
                json!({ "error": null, "type": "error" }),
                json!({
                    "finishReason": { "unified": "error" },
                    "type": "finish",
                    "usage": { "inputTokens": {}, "outputTokens": {} },
                }),
            ]
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should send request body
    #[tokio::test]
    async fn should_send_request_body() {
        let mock = prepare_empty_stream_response(&[]);

        let result = model(&mock).do_stream(&stream_options()).await.unwrap();

        assert_eq!(
            serde_json::to_value(&result.request).unwrap(),
            json!({
                "body": {
                    "model": "gpt-3.5-turbo-instruct",
                    "prompt": PROMPT_TEXT,
                    "stop": ["\nuser:"],
                    "stream": true,
                },
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let mock = prepare_empty_stream_response(&[("test-header", "test-value")]);

        let result = model(&mock).do_stream(&stream_options()).await.unwrap();

        assert_eq!(
            result.response.unwrap().headers.unwrap(),
            header_map(&[
                // default headers:
                ("content-type", "text/event-stream"),
                ("cache-control", "no-cache"),
                ("connection", "keep-alive"),
                // custom header
                ("test-header", "test-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should pass the model and the prompt
    #[tokio::test]
    async fn should_pass_the_model_and_the_prompt() {
        let mock = prepare_empty_stream_response(&[]);

        model(&mock).do_stream(&stream_options()).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "gpt-3.5-turbo-instruct",
                "prompt": PROMPT_TEXT,
                "stop": ["\nuser:"],
                "stream": true,
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = prepare_empty_stream_response(&[]);

        let provider = create_provider(
            &mock,
            &[
                ("Authorization", "Bearer test-api-key"),
                ("Custom-Provider-Header", "provider-header-value"),
            ],
        );

        let mut options = stream_options();
        options.headers = Some(header_map(&[(
            "Custom-Request-Header",
            "request-header-value",
        )]));
        provider
            .completion("gpt-3.5-turbo-instruct")
            .do_stream(&options)
            .await
            .unwrap();

        assert_eq!(
            request_headers(&mock.seen()[0]),
            expected_headers(&[
                ("authorization", "Bearer test-api-key"),
                ("content-type", "application/json"),
                ("custom-provider-header", "provider-header-value"),
                ("custom-request-header", "request-header-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should include provider-specific options
    #[tokio::test]
    async fn should_include_provider_specific_options() {
        let mock = prepare_empty_stream_response(&[]);

        model(&mock)
            .do_stream(&with_provider_options(
                stream_options(),
                json!({ "test-provider": { "someCustomOption": "test-value" } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "gpt-3.5-turbo-instruct",
                "prompt": PROMPT_TEXT,
                "someCustomOption": "test-value",
                "stop": ["\nuser:"],
                "stream": true,
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › should not include provider-specific options for different provider
    #[tokio::test]
    async fn should_not_include_provider_specific_options_for_different_provider() {
        let mock = prepare_empty_stream_response(&[]);

        model(&mock)
            .do_stream(&with_provider_options(
                stream_options(),
                json!({ "notThisProviderName": { "someCustomOption": "test-value" } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "gpt-3.5-turbo-instruct",
                "prompt": PROMPT_TEXT,
                "stop": ["\nuser:"],
                "stream": true,
            })
        );
    }

    mod camel_case_provider_options {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › camelCase provider options › should accept camelCase provider options key for hyphenated provider name
        #[tokio::test]
        async fn should_accept_camel_case_provider_options_key_for_hyphenated_provider_name() {
            let mock = prepare_empty_stream_response(&[]);

            model(&mock)
                .do_stream(&with_provider_options(
                    stream_options(),
                    json!({ "testProvider": { "someCustomOption": "test-value" } }),
                ))
                .await
                .unwrap();

            assert_eq!(
                mock.seen()[0].json_body()["someCustomOption"],
                json!("test-value")
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/completion/openai-compatible-completion-language-model.test.ts › doStream › camelCase provider options › should prefer camelCase options over raw-name options
        #[tokio::test]
        async fn should_prefer_camel_case_options_over_raw_name_options() {
            let mock = prepare_empty_stream_response(&[]);

            model(&mock)
                .do_stream(&with_provider_options(
                    stream_options(),
                    json!({
                        "test-provider": { "someCustomOption": "raw-value" },
                        "testProvider": { "someCustomOption": "camel-value" },
                    }),
                ))
                .await
                .unwrap();

            assert_eq!(
                mock.seen()[0].json_body()["someCustomOption"],
                json!("camel-value")
            );
        }
    }
}
