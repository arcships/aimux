//! The DeepSeek chat model (`aimux_providers::deepseek`), ported from the AI SDK
//! `@ai-sdk/deepseek` 3.0.56 test suites:
//!
//! - `chat/deepseek-chat-language-model.test.ts`: `model IDs`, `supportedUrls`,
//!   `doGenerate` (`text`, `reasoning`, `logprobs`, `top-level reasoning`,
//!   `tool call`, `json response format`, `assistant prefix completion`) and
//!   `doStream` (stream errors, `text`, `reasoning`, `logprobs`, `tool call`,
//!   `assistant prefix completion`).
//! - `chat/convert-to-deepseek-chat-messages.test.ts`: `message names`,
//!   `user messages`, `tool calls`, `deepseek-v4 thinking mode` and
//!   `assistant prefix completion`, observed through the request body.
//!
//! Left out: the `json response format with structured outputs` block (it needs
//! a model built with `supportsStructuredOutputs`, which only the Azure wiring
//! sets and this package does not expose), the tool-result `content` output
//! cases (core tool results carry a JSON value, not the AI SDK output union),
//! and the snapshot fixtures (the responses are inlined here).
//!
//! HTTP is a local `wiremock` server; every model is built with
//! `create_deepseek(..).chat(..)`.

use aimux_core::tool::RawToolCall;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, Tool};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput};
use aimux_core::shared::provider_namespace;
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort, ResponseMetadata, Warning};
use aimux_provider_utils::Resolvable;
use aimux_providers::deepseek::{
    DeepSeekChatLanguageModel, DeepSeekProviderSettings, create_deepseek,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn with_options(mut message: LanguageModelMessage, options: Value) -> LanguageModelMessage {
    let (LanguageModelMessage::System {
        provider_options, ..
    }
    | LanguageModelMessage::User {
        provider_options, ..
    }
    | LanguageModelMessage::Assistant {
        provider_options, ..
    }
    | LanguageModelMessage::Tool {
        provider_options, ..
    }) = &mut message;
    *provider_options = Some(serde_json::from_value(options).unwrap());
    message
}

/// The upstream `TEST_PROMPT`.
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

fn options() -> CallOptions {
    CallOptions::new(test_prompt())
}

fn options_for(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn with_provider_options(mut options: CallOptions, value: Value) -> CallOptions {
    options.provider_options = Some(serde_json::from_value(value).unwrap());
    options
}

fn weather_tool() -> FunctionTool {
    FunctionTool {
        name: "weather".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

fn strict_tool(name: &str, strict: Option<bool>) -> Tool {
    Tool::Function(FunctionTool {
        name: name.to_string(),
        description: None,
        input_schema: json!({ "type": "object", "properties": {} }),
        strict,
        provider_options: None,
        input_examples: None,
    })
}

fn settings(server: &MockServer, base_path: &str) -> DeepSeekProviderSettings {
    DeepSeekProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(format!("{}{base_path}", server.uri())),
        ..Default::default()
    }
}

/// `provider.chat(model_id)` of the upstream tests.
fn chat(server: &MockServer, model_id: &str) -> DeepSeekChatLanguageModel {
    create_deepseek(settings(server, ""))
        .unwrap()
        .chat(model_id)
}

/// `betaProvider.chat(model_id)` of the upstream tests.
fn beta_chat(server: &MockServer, model_id: &str) -> DeepSeekChatLanguageModel {
    create_deepseek(settings(server, "/beta"))
        .unwrap()
        .chat(model_id)
}

async fn json_server(body: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(".*/chat/completions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

fn data_event(chunk: &Value) -> String {
    format!("data: {chunk}\n\n")
}

async fn sse_server(chunks: Vec<String>) -> MockServer {
    let mut body = chunks.concat();
    body.push_str("data: [DONE]\n\n");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(".*/chat/completions$"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    server
}

async fn request_paths(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_string())
        .collect()
}

fn text_response() -> Value {
    json!({
        "id": "00f10ecd-60b3-4707-b5db-e4bcadf7aea1",
        "object": "chat.completion",
        "created": 1764656316,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 13,
            "completion_tokens": 300,
            "total_tokens": 313,
            "prompt_cache_hit_tokens": 0,
            "prompt_cache_miss_tokens": 13
        },
        "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache"
    })
}

/// Generate against a canned text response and return the request body.
async fn generate_body(model_id: &str, options: &CallOptions) -> Value {
    let server = json_server(text_response()).await;
    chat(&server, model_id)
        .do_generate(options)
        .await
        .unwrap()
        .request
        .and_then(|r| r.body)
        .unwrap()
}

async fn generate_result(model_id: &str, options: &CallOptions) -> GenerateResult {
    let server = json_server(text_response()).await;
    chat(&server, model_id).do_generate(options).await.unwrap()
}

/// The wire messages of a generate call.
async fn wire_messages(model_id: &str, prompt: LanguageModelPrompt) -> Vec<Value> {
    generate_body(model_id, &options_for(prompt)).await["messages"]
        .as_array()
        .unwrap()
        .clone()
}

/// Run a generate call that must fail before anything is sent.
async fn generate_error(model_id: &str, options: &CallOptions) -> String {
    let server = json_server(text_response()).await;
    let error = chat(&server, model_id)
        .do_generate(options)
        .await
        .expect_err("the call must be rejected");
    assert!(
        request_paths(&server).await.is_empty(),
        "nothing may be fetched"
    );
    error.to_string()
}

fn warning_values(warnings: &[Warning]) -> Vec<Value> {
    warnings
        .iter()
        .map(|warning| serde_json::to_value(warning).unwrap())
        .collect()
}

fn deprecated(setting: &str) -> Warning {
    Warning::Deprecated {
        setting: setting.to_string(),
        message: format!(
            "{setting} is deprecated by DeepSeek and has been omitted. Remove {setting} from the request."
        ),
    }
}

fn thinking_unsupported(feature: &str) -> Warning {
    Warning::Unsupported {
        feature: feature.to_string(),
        details: Some(format!(
            "{feature} has no effect when DeepSeek thinking is enabled. Set providerOptions.deepseek.thinking.type to 'disabled' to use {feature}."
        )),
    }
}

fn compatibility(feature: &str, details: &str) -> Warning {
    Warning::Compatibility {
        feature: feature.to_string(),
        details: Some(details.to_string()),
    }
}

async fn collect(stream: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = stream.stream;
    while let Some(part) = stream.next().await {
        parts.push(part.expect("stream part"));
    }
    parts
}

fn text_chunks() -> Vec<String> {
    vec![
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat", "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache",
            "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "" }, "finish_reason": null }]
        })),
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat", "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache",
            "choices": [{ "index": 0, "delta": { "content": "Hello" }, "finish_reason": null }]
        })),
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat", "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache",
            "choices": [{ "index": 0, "delta": { "content": ", World!" }, "finish_reason": null }]
        })),
        data_event(&json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1764656316,
            "model": "deepseek-chat", "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache",
            "choices": [{ "index": 0, "delta": { "content": "" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 13, "completion_tokens": 3, "total_tokens": 16,
                "prompt_cache_hit_tokens": 5, "prompt_cache_miss_tokens": 8
            }
        })),
    ]
}

fn finish_metadata(parts: &[StreamPart]) -> Value {
    parts
        .iter()
        .find_map(|part| match part {
            StreamPart::Finish {
                provider_metadata, ..
            } => provider_metadata
                .as_ref()
                .map(|metadata| serde_json::to_value(metadata).unwrap()),
            _ => None,
        })
        .expect("a finish part")
}

fn stream_start_warnings(parts: &[StreamPart]) -> Vec<Value> {
    match &parts[0] {
        StreamPart::StreamStart { warnings } => warning_values(warnings),
        other => panic!("expected stream-start, got {other:?}"),
    }
}

fn logprob(token: &str, logprob: f64, bytes: Value) -> Value {
    json!({
        "token": token, "logprob": logprob, "bytes": bytes,
        "top_logprobs": [{ "token": token, "logprob": logprob, "bytes": bytes }]
    })
}

// ===========================================================================
// describe('model IDs') / describe('supportedUrls')
// ===========================================================================

#[tokio::test]
async fn should_forward_the_model_id() {
    for model_id in ["deepseek-v4-flash", "deepseek-v4-pro"] {
        assert_eq!(generate_body(model_id, &options()).await["model"], model_id);
    }
}

#[tokio::test]
async fn should_natively_support_http_image_urls() {
    let server = MockServer::start().await;
    let urls = chat(&server, "deepseek-chat").supported_urls();
    let patterns = &urls.0["image/*"];
    assert_eq!(patterns.len(), 1);
    assert!(patterns[0].is_match("https://example.com/a.png"));
    assert!(patterns[0].is_match("http://example.com/a.png"));
    assert!(!patterns[0].is_match("data:image/png;base64,AAAA"));
}

// ===========================================================================
// describe('doGenerate')
// ===========================================================================

#[tokio::test]
async fn should_reject_a_response_without_choices() {
    let server = json_server(json!({
        "id": "chatcmpl-empty", "object": "chat.completion", "created": 0,
        "model": "deepseek-chat", "choices": [],
        "usage": { "prompt_tokens": 1, "completion_tokens": 0, "total_tokens": 1 }
    }))
    .await;
    let error = chat(&server, "deepseek-chat")
        .do_generate(&options())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidResponseData(m) if m == "Response did not contain any choices."),
        "{error:?}"
    );
}

#[tokio::test]
async fn should_use_the_chat_completions_path_of_the_default_and_beta_base_urls() {
    let server = json_server(text_response()).await;
    chat(&server, "deepseek-chat")
        .do_generate(&options())
        .await
        .unwrap();
    beta_chat(&server, "deepseek-chat")
        .do_generate(&options())
        .await
        .unwrap();
    assert_eq!(
        request_paths(&server).await,
        ["/chat/completions", "/beta/chat/completions"]
    );
}

// ---- describe('text') ------------------------------------------------------

#[tokio::test]
async fn text_should_send_correct_request_body() {
    let mut options = options_for(vec![
        LanguageModelMessage::System {
            content: ("You are a helpful assistant.").into(),
            provider_options: None,
        },
        LanguageModelMessage::user_text("Hello"),
    ]);
    options.temperature = Some(0.5);
    options.top_p = Some(0.3);
    assert_eq!(
        generate_body("deepseek-chat", &options).await,
        json!({
            "messages": [
                { "content": "You are a helpful assistant.", "role": "system" },
                { "content": "Hello", "role": "user" }
            ],
            "model": "deepseek-chat",
            "temperature": 0.5,
            "top_p": 0.3
        })
    );
}

#[tokio::test]
async fn text_should_omit_deprecated_and_ineffective_sampling_options_in_default_v4_thinking_mode()
{
    let mut options = options();
    options.temperature = Some(0.2);
    options.top_p = Some(0.4);
    options.frequency_penalty = Some(0.5);
    options.presence_penalty = Some(0.6);
    let result = generate_result("deepseek-v4-flash", &options).await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap(),
        json!({ "model": "deepseek-v4-flash", "messages": [{ "role": "user", "content": "Hello" }] })
    );
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[
            deprecated("frequencyPenalty"),
            deprecated("presencePenalty"),
            thinking_unsupported("temperature"),
            thinking_unsupported("topP"),
        ])
    );
}

#[tokio::test]
async fn text_should_preserve_supported_sampling_options_when_v4_thinking_is_disabled() {
    let mut options = with_provider_options(
        options(),
        json!({ "deepseek": { "thinking": { "type": "disabled" } } }),
    );
    options.temperature = Some(0.2);
    options.top_p = Some(0.4);
    options.frequency_penalty = Some(0.5);
    options.presence_penalty = Some(0.6);
    let result = generate_result("deepseek-v4-flash", &options).await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap(),
        json!({
            "model": "deepseek-v4-flash",
            "messages": [{ "role": "user", "content": "Hello" }],
            "temperature": 0.2,
            "top_p": 0.4,
            "thinking": { "type": "disabled" }
        })
    );
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[
            deprecated("frequencyPenalty"),
            deprecated("presencePenalty")
        ])
    );
}

#[tokio::test]
async fn text_should_warn_about_top_k_and_seed_and_not_send_them() {
    // upstream: getArgs pushes `{ type: 'unsupported', feature: 'topK' }` and
    // never puts topK in the body.
    let mut options = options();
    options.top_k = Some(40.0);
    options.seed = Some(7);
    let result = generate_result("deepseek-chat", &options).await;
    let body = result.request.and_then(|r| r.body).unwrap();
    assert!(body.get("top_k").is_none() && body.get("seed").is_none());
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[
            Warning::Unsupported {
                feature: "topK".to_string(),
                details: None
            },
            Warning::Unsupported {
                feature: "seed".to_string(),
                details: None
            },
        ])
    );
}

#[tokio::test]
async fn text_should_send_max_tokens_and_stop_sequences() {
    let mut options = options();
    options.max_output_tokens = Some(64);
    options.stop_sequences = Some(vec!["END".to_string()]);
    let body = generate_body("deepseek-chat", &options).await;
    assert_eq!(body["max_tokens"], 64);
    assert_eq!(body["stop"], json!(["END"]));
    assert!(body.get("max_completion_tokens").is_none());
}

#[tokio::test]
async fn text_should_send_message_names() {
    let name = |value: &str| json!({ "deepseek": { "name": value } });
    let messages = wire_messages(
        "deepseek-chat",
        vec![
            with_options(
                LanguageModelMessage::System {
                    content: ("You are a helpful assistant.").into(),
                    provider_options: None,
                },
                name("guide"),
            ),
            with_options(LanguageModelMessage::user_text("Hello"), name("alice")),
            with_options(
                LanguageModelMessage::Assistant {
                    content: vec![AssistantPart::Text(TextPart {
                        text: ("Hello, Alice.").into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
                name("assistant"),
            ),
            with_options(
                LanguageModelMessage::user_text("How are you?"),
                name("alice"),
            ),
        ],
    )
    .await;
    assert_eq!(
        messages,
        [
            json!({ "content": "You are a helpful assistant.", "name": "guide", "role": "system" }),
            json!({ "content": "Hello", "name": "alice", "role": "user" }),
            json!({ "content": "Hello, Alice.", "name": "assistant", "role": "assistant" }),
            json!({ "content": "How are you?", "name": "alice", "role": "user" }),
        ]
    );
}

#[tokio::test]
async fn text_should_pass_provider_options_user_id_as_user_id() {
    let options = with_provider_options(
        options(),
        json!({ "deepseek": { "userId": "tenant_123-user" } }),
    );
    assert_eq!(
        generate_body("deepseek-chat", &options).await["user_id"],
        "tenant_123-user"
    );
}

#[tokio::test]
async fn text_should_omit_user_id_when_user_id_is_not_provided() {
    assert!(
        generate_body("deepseek-chat", &options())
            .await
            .get("user_id")
            .is_none()
    );
}

#[tokio::test]
async fn text_should_not_send_unknown_provider_options() {
    // upstream: `deepseekLanguageModelChatOptions` is a zod object, which
    // strips keys it does not declare.
    let options =
        with_provider_options(options(), json!({ "deepseek": { "user": "u1", "foo": 1 } }));
    let body = generate_body("deepseek-chat", &options).await;
    assert!(body.get("user").is_none() && body.get("foo").is_none());
}

#[tokio::test]
async fn text_should_reject_strict_tools_on_the_standard_endpoint_before_fetching() {
    let mut options = options();
    options.tools = Some(vec![strict_tool("getWeather", Some(true))]);
    let message = generate_error("deepseek-chat", &options).await;
    assert!(
        message.contains("DeepSeek strict tool calls require a beta base URL ending in `/beta`."),
        "{message}"
    );
}

#[tokio::test]
async fn text_should_send_all_strict_tools_on_the_beta_endpoint() {
    let server = json_server(text_response()).await;
    let mut options = options();
    options.tools = Some(vec![strict_tool("getWeather", Some(true))]);
    let body = beta_chat(&server, "deepseek-chat")
        .do_generate(&options)
        .await
        .unwrap()
        .request
        .and_then(|r| r.body)
        .unwrap();
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "getWeather");
    assert_eq!(body["tools"][0]["function"]["strict"], true);
}

#[tokio::test]
async fn text_should_reject_mixed_strict_tools_in_streaming_requests() {
    let server = sse_server(text_chunks()).await;
    let mut options = options();
    options.tools = Some(vec![
        strict_tool("strictTool", Some(true)),
        strict_tool("nonStrictTool", None),
    ]);
    let Err(error) = beta_chat(&server, "deepseek-chat")
        .do_stream(&options)
        .await
    else {
        panic!("the call must be rejected");
    };
    assert!(
        error.to_string().contains(
            "DeepSeek strict mode requires every function tool in the request to set `strict: true`."
        ),
        "{error}"
    );
    assert!(request_paths(&server).await.is_empty());
}

#[tokio::test]
async fn text_should_reject_invalid_user_id_before_making_an_api_request() {
    for (user_id, expected) in [
        ("".to_string(), "userId must match /^[a-zA-Z0-9_-]+$/"),
        (
            "contains space".to_string(),
            "userId must match /^[a-zA-Z0-9_-]+$/",
        ),
        (
            "a".repeat(513),
            "userId must be at most 512 characters long",
        ),
    ] {
        let options =
            with_provider_options(options(), json!({ "deepseek": { "userId": user_id } }));
        let message = generate_error("deepseek-chat", &options).await;
        assert!(message.contains("invalid provider options"), "{message}");
        assert!(message.contains(expected), "{message}");
    }
}

#[tokio::test]
async fn text_should_extract_text_content() {
    let result = generate_result("deepseek-chat", &options()).await;
    assert_eq!(result.content.len(), 1);
    assert!(
        matches!(&result.content[0], GenerateContent::Text { text, .. } if text == "Hello, World!")
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("stop"));
    assert_eq!(result.usage.input_tokens.total, Some(13));
    assert_eq!(result.usage.output_tokens.total, Some(300));
    assert_eq!(
        result.response.as_ref().and_then(|r| r.id.as_deref()),
        Some("00f10ecd-60b3-4707-b5db-e4bcadf7aea1")
    );
    assert_eq!(
        result.response.as_ref().and_then(|r| r.model_id.as_deref()),
        Some("deepseek-chat")
    );
}

#[tokio::test]
async fn text_should_report_usage_with_prompt_cache_tokens() {
    // upstream convert-to-deepseek-usage.ts: cache hits are the cache-read tokens.
    let mut body = text_response();
    body["usage"] = json!({
        "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
        "prompt_cache_hit_tokens": 70, "prompt_cache_miss_tokens": 30,
        "completion_tokens_details": { "reasoning_tokens": 5 }
    });
    let server = json_server(body).await;
    let usage = chat(&server, "deepseek-chat")
        .do_generate(&options())
        .await
        .unwrap()
        .usage;
    assert_eq!(usage.input_tokens.total, Some(100));
    assert_eq!(usage.input_tokens.cache_read, Some(70));
    assert_eq!(usage.input_tokens.no_cache, Some(30));
    assert_eq!(usage.output_tokens.total, Some(20));
    assert_eq!(usage.output_tokens.reasoning, Some(5));
    assert_eq!(usage.output_tokens.text, Some(15));
    assert_eq!(usage.raw.unwrap()["prompt_cache_hit_tokens"], 70);
}

#[tokio::test]
async fn text_should_report_the_provider_metadata() {
    // upstream doGenerate: promptCacheHitTokens, promptCacheMissTokens,
    // responseObject, choiceIndex, messageRole, systemFingerprint.
    let result = generate_result("deepseek-chat", &options()).await;
    assert_eq!(
        serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
        json!({ "deepseek": {
            "promptCacheHitTokens": 0,
            "promptCacheMissTokens": 13,
            "responseObject": "chat.completion",
            "choiceIndex": 0,
            "messageRole": "assistant",
            "systemFingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache"
        } })
    );
}

#[tokio::test]
async fn text_should_include_the_system_fingerprint_in_provider_metadata() {
    let result = generate_result("deepseek-chat", &options()).await;
    assert_eq!(
        result.provider_metadata.unwrap()["deepseek"]["systemFingerprint"],
        "fp_eaab8d114b_prod0820_fp8_kvcache"
    );
}

#[tokio::test]
async fn text_should_tolerate_a_null_or_missing_system_fingerprint() {
    for fingerprint in [Some(Value::Null), None] {
        let mut body = text_response();
        match fingerprint {
            Some(value) => body["system_fingerprint"] = value,
            None => {
                body.as_object_mut().unwrap().remove("system_fingerprint");
            }
        }
        let server = json_server(body).await;
        let result = chat(&server, "deepseek-chat")
            .do_generate(&options())
            .await
            .unwrap();
        assert!(
            result.provider_metadata.unwrap()["deepseek"]
                .get("systemFingerprint")
                .is_none()
        );
    }
}

// ---- describe('reasoning') ---------------------------------------------------

fn reasoning_response() -> Value {
    json!({
        "id": "r1", "object": "chat.completion", "created": 1764656316, "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "There are 3 r's in strawberry.",
                "reasoning_content": "Let me count the letters."
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 20, "completion_tokens": 30, "total_tokens": 50,
                   "completion_tokens_details": { "reasoning_tokens": 10 } }
    })
}

#[tokio::test]
async fn reasoning_should_send_correct_request_body() {
    let options = with_provider_options(
        options_for(vec![LanguageModelMessage::user_text(
            "How many \"r\"s are in the word \"strawberry\"?",
        )]),
        json!({ "deepseek": { "thinking": { "type": "enabled" } } }),
    );
    assert_eq!(
        generate_body("deepseek-reasoner", &options).await,
        json!({
            "messages": [{ "content": "How many \"r\"s are in the word \"strawberry\"?", "role": "user" }],
            "model": "deepseek-reasoner",
            "thinking": { "type": "enabled" }
        })
    );
}

#[tokio::test]
async fn reasoning_should_extract_reasoning_before_text() {
    let server = json_server(reasoning_response()).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&options())
        .await
        .unwrap();
    assert_eq!(result.content.len(), 2);
    assert!(
        matches!(&result.content[0], GenerateContent::Reasoning(ReasoningOutput { text, .. }) if text == "Let me count the letters.")
    );
    assert!(
        matches!(&result.content[1], GenerateContent::Text { text, .. } if text == "There are 3 r's in strawberry.")
    );
    assert_eq!(result.usage.output_tokens.reasoning, Some(10));
    assert_eq!(result.usage.output_tokens.text, Some(20));
}

#[tokio::test]
async fn reasoning_should_not_read_the_reasoning_field() {
    // upstream deepseekChatResponseSchema only declares `reasoning_content`.
    let mut body = reasoning_response();
    let message = body["choices"][0]["message"].as_object_mut().unwrap();
    let reasoning = message.remove("reasoning_content").unwrap();
    message.insert("reasoning".to_string(), reasoning);
    let server = json_server(body).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&options())
        .await
        .unwrap();
    assert_eq!(result.content.len(), 1);
    assert!(matches!(&result.content[0], GenerateContent::Text { .. }));
}

// ---- describe('logprobs') ----------------------------------------------------

fn logprobs_response() -> Value {
    let mut body = text_response();
    body["choices"][0]["message"] = json!({
        "role": "assistant", "content": "OK", "reasoning_content": "Reasoning"
    });
    body["choices"][0]["logprobs"] = json!({
        "content": [logprob("OK", -0.00002467602, json!([79, 75]))],
        "reasoning_content": [logprob("Reasoning", -0.1, Value::Null)]
    });
    body
}

#[tokio::test]
async fn logprobs_should_send_logprobs_provider_options() {
    let options = with_provider_options(
        options(),
        json!({ "deepseek": { "logprobs": true, "topLogprobs": 1 } }),
    );
    assert_eq!(
        generate_body("deepseek-v4-flash", &options).await,
        json!({
            "logprobs": true,
            "messages": [{ "content": "Hello", "role": "user" }],
            "model": "deepseek-v4-flash",
            "top_logprobs": 1
        })
    );
}

#[tokio::test]
async fn logprobs_should_enable_logprobs_when_top_logprobs_is_set() {
    let options = with_provider_options(options(), json!({ "deepseek": { "topLogprobs": 1 } }));
    let body = generate_body("deepseek-v4-flash", &options).await;
    assert_eq!(body["logprobs"], true);
    assert_eq!(body["top_logprobs"], 1);
}

#[tokio::test]
async fn logprobs_should_extract_content_and_reasoning_logprobs() {
    let server = json_server(logprobs_response()).await;
    let options = with_provider_options(options(), json!({ "deepseek": { "logprobs": true } }));
    let result = chat(&server, "deepseek-v4-flash")
        .do_generate(&options)
        .await
        .unwrap();
    assert_eq!(
        result.provider_metadata.unwrap()["deepseek"]["logprobs"],
        json!({
            "content": [logprob("OK", -0.00002467602, json!([79, 75]))],
            "reasoning_content": [logprob("Reasoning", -0.1, Value::Null)]
        })
    );
}

// ---- describe('top-level reasoning') -------------------------------------------

fn reasoning_options(effort: ReasoningEffort) -> CallOptions {
    CallOptions {
        reasoning: Some(effort),
        ..options()
    }
}

#[tokio::test]
async fn top_level_reasoning_should_map_to_thinking_enabled() {
    let body = generate_body(
        "deepseek-reasoner",
        &reasoning_options(ReasoningEffort::High),
    )
    .await;
    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
    assert_eq!(body["reasoning_effort"], "high");
}

#[tokio::test]
async fn top_level_reasoning_none_should_map_to_thinking_disabled() {
    let body = generate_body(
        "deepseek-reasoner",
        &reasoning_options(ReasoningEffort::None),
    )
    .await;
    assert_eq!(body["thinking"], json!({ "type": "disabled" }));
    assert!(body.get("reasoning_effort").is_none());
}

#[tokio::test]
async fn top_level_reasoning_none_should_keep_the_temperature() {
    // thinking is disabled, so temperature applies and is not warned about.
    let mut options = reasoning_options(ReasoningEffort::None);
    options.temperature = Some(0.4);
    let result = generate_result("deepseek-reasoner", &options).await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["temperature"],
        0.4
    );
    assert!(result.warnings.is_empty());
}

#[tokio::test]
async fn top_level_reasoning_xhigh_should_map_to_reasoning_effort_max() {
    let result = generate_result(
        "deepseek-reasoner",
        &reasoning_options(ReasoningEffort::Xhigh),
    )
    .await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
        "max"
    );
    assert!(warning_values(&result.warnings).contains(&serde_json::to_value(compatibility(
        "reasoning",
        "reasoning \"xhigh\" is not directly supported by this model. mapped to effort \"max\"."
    ))
    .unwrap()));
}

#[tokio::test]
async fn top_level_reasoning_low_should_map_to_reasoning_effort_low_without_a_compatibility_warning()
 {
    let result = generate_result(
        "deepseek-reasoner",
        &reasoning_options(ReasoningEffort::Low),
    )
    .await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
        "low"
    );
    assert!(
        !result
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::Compatibility { feature, .. } if feature == "reasoning"))
    );
}

#[tokio::test]
async fn top_level_reasoning_medium_should_map_to_reasoning_effort_high_with_a_compatibility_warning()
 {
    let result = generate_result(
        "deepseek-reasoner",
        &reasoning_options(ReasoningEffort::Medium),
    )
    .await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
        "high"
    );
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[compatibility(
            "reasoning",
            "reasoning \"medium\" is not directly supported by this model. mapped to effort \"high\"."
        )])
    );
}

#[tokio::test]
async fn top_level_reasoning_minimal_should_map_to_reasoning_effort_low_with_compatibility_warning()
{
    let result = generate_result(
        "deepseek-reasoner",
        &reasoning_options(ReasoningEffort::Minimal),
    )
    .await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
        "low"
    );
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[compatibility(
            "reasoning",
            "reasoning \"minimal\" is not directly supported by this model. mapped to effort \"low\"."
        )])
    );
}

#[tokio::test]
async fn top_level_reasoning_should_map_provider_options_reasoning_effort() {
    for (input, output, warns) in [
        ("low", "low", false),
        ("medium", "high", true),
        ("xhigh", "max", true),
    ] {
        let options = with_provider_options(
            options(),
            json!({ "deepseek": { "reasoningEffort": input } }),
        );
        let result = generate_result("deepseek-reasoner", &options).await;
        assert_eq!(
            result.request.and_then(|r| r.body).unwrap()["reasoning_effort"],
            output
        );
        let expected = if warns {
            vec![compatibility(
                "reasoningEffort",
                &format!(
                    "reasoningEffort \"{input}\" is not a canonical DeepSeek value. mapped to \"{output}\"."
                ),
            )]
        } else {
            vec![]
        };
        assert_eq!(warning_values(&result.warnings), warning_values(&expected));
    }
}

#[tokio::test]
async fn top_level_reasoning_should_map_legacy_thinking_type_adaptive_to_enabled_with_a_compatibility_warning()
 {
    let options = with_provider_options(
        options(),
        json!({ "deepseek": { "thinking": { "type": "adaptive" } } }),
    );
    let result = generate_result("deepseek-reasoner", &options).await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["thinking"],
        json!({ "type": "enabled" })
    );
    assert!(warning_values(&result.warnings).contains(&serde_json::to_value(compatibility(
        "thinking.type",
        "thinking.type \"adaptive\" is not a canonical DeepSeek value. mapped to \"enabled\"."
    ))
    .unwrap()));
}

#[tokio::test]
async fn top_level_reasoning_should_pass_provider_options_reasoning_effort() {
    let options = with_provider_options(
        options(),
        json!({ "deepseek": { "reasoningEffort": "max" } }),
    );
    let body = generate_body("deepseek-reasoner", &options).await;
    assert_eq!(body["reasoning_effort"], "max");
    // When only reasoningEffort is set without thinking, thinking stays unset.
    assert!(body.get("thinking").is_none());
}

#[tokio::test]
async fn top_level_reasoning_should_prefer_provider_options_thinking_over_top_level_reasoning() {
    let options = with_provider_options(
        reasoning_options(ReasoningEffort::None),
        json!({ "deepseek": { "thinking": { "type": "enabled" } } }),
    );
    assert_eq!(
        generate_body("deepseek-reasoner", &options).await["thinking"],
        json!({ "type": "enabled" })
    );
}

#[tokio::test]
async fn top_level_reasoning_should_prefer_provider_options_reasoning_effort_over_top_level_reasoning()
 {
    let options = with_provider_options(
        reasoning_options(ReasoningEffort::High),
        json!({ "deepseek": { "reasoningEffort": "max" } }),
    );
    assert_eq!(
        generate_body("deepseek-reasoner", &options).await["reasoning_effort"],
        "max"
    );
}

#[tokio::test]
async fn top_level_reasoning_should_not_set_thinking_when_reasoning_is_not_specified() {
    assert!(
        generate_body("deepseek-reasoner", &options())
            .await
            .get("thinking")
            .is_none()
    );
}

#[tokio::test]
async fn top_level_reasoning_provider_default_sends_neither_thinking_nor_effort() {
    let body = generate_body(
        "deepseek-reasoner",
        &reasoning_options(ReasoningEffort::ProviderDefault),
    )
    .await;
    assert!(body.get("thinking").is_none() && body.get("reasoning_effort").is_none());
}

// ---- describe('tool call') ---------------------------------------------------

fn tool_call_response() -> Value {
    json!({
        "id": "t1", "object": "chat.completion", "created": 1764656316, "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant", "content": "",
                "reasoning_content": "I should look up the weather.",
                "tool_calls": [{
                    "id": "call_00_abc", "type": "function",
                    "function": { "name": "weather", "arguments": "{\"location\":\"San Francisco\"}" }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 50, "completion_tokens": 20, "total_tokens": 70 }
    })
}

fn tool_options() -> CallOptions {
    let mut options = with_provider_options(
        options(),
        json!({ "deepseek": { "thinking": { "type": "enabled" } } }),
    );
    options.tools = Some(vec![Tool::Function(weather_tool())]);
    options
}

fn weather_wire_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "weather",
            "parameters": {
                "$schema": "http://json-schema.org/draft-07/schema#",
                "additionalProperties": false,
                "properties": { "location": { "type": "string" } },
                "required": ["location"],
                "type": "object"
            }
        }
    })
}

#[tokio::test]
async fn tool_call_should_send_correct_request_body() {
    // The core `tool_choice` is not optional (default `auto`), so it is always
    // sent next to the tools.
    assert_eq!(
        generate_body("deepseek-reasoner", &tool_options()).await,
        json!({
            "messages": [{ "content": "Hello", "role": "user" }],
            "model": "deepseek-reasoner",
            "thinking": { "type": "enabled" },
            "tools": [weather_wire_tool()],
            "tool_choice": "auto"
        })
    );
}

#[tokio::test]
async fn tool_call_should_send_the_tool_choice() {
    use aimux_core::options::ToolChoice;
    let mut options = tool_options();
    options.tool_choice = ToolChoice::Tool {
        tool_name: "weather".to_string(),
    };
    assert_eq!(
        generate_body("deepseek-reasoner", &options).await["tool_choice"],
        json!({ "type": "function", "function": { "name": "weather" } })
    );
    options.tool_choice = ToolChoice::Required;
    assert_eq!(
        generate_body("deepseek-reasoner", &options).await["tool_choice"],
        "required"
    );
}

#[tokio::test]
async fn tool_call_should_warn_about_provider_defined_tools() {
    use aimux_core::tool::ProviderTool;
    let mut options = options();
    options.tools = Some(vec![Tool::Provider(ProviderTool {
        id: "test.search".to_string(),
        name: "search".to_string(),
        args: json!({}),
    })]);
    let result = generate_result("deepseek-chat", &options).await;
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[Warning::Unsupported {
            feature: "provider-defined tool test.search".to_string(),
            details: None,
        }])
    );
    assert!(
        result.request.and_then(|r| r.body).unwrap()["tools"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn tool_call_should_extract_tool_call_content() {
    let server = json_server(tool_call_response()).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&tool_options())
        .await
        .unwrap();
    assert!(
        matches!(&result.content[0], GenerateContent::Reasoning(ReasoningOutput { text, .. }) if text == "I should look up the weather.")
    );
    assert!(matches!(
        &result.content[1],
        GenerateContent::ToolCall(RawToolCall { tool_call_id, tool_name, input, .. })
            if tool_call_id == "call_00_abc" && tool_name == "weather"
                && input == "{\"location\":\"San Francisco\"}"
    ));
    assert_eq!(result.content.len(), 2);
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    assert_eq!(
        result.provider_metadata.unwrap()["deepseek"]["toolCallTypes"],
        json!(["function"])
    );
}

#[tokio::test]
async fn tool_call_should_generate_an_id_when_the_server_sends_none() {
    let mut body = tool_call_response();
    body["choices"][0]["message"]["tool_calls"][0]["id"] = Value::Null;
    let server = json_server(body).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&tool_options())
        .await
        .unwrap();
    assert!(matches!(
        &result.content[1],
        GenerateContent::ToolCall(RawToolCall { tool_call_id, .. }) if !tool_call_id.is_empty()
    ));
}

// ---- describe('json response format') -------------------------------------------

fn json_options(schema: Option<Value>) -> CallOptions {
    let mut options = tool_options();
    options.response_format = Some(ResponseFormat::Json {
        schema,
        name: None,
        description: None,
    });
    options
}

#[tokio::test]
async fn json_response_format_should_send_correct_request_body_without_schema() {
    assert_eq!(
        generate_body("deepseek-reasoner", &json_options(None)).await,
        json!({
            "messages": [
                { "content": "Return JSON.", "role": "system" },
                { "content": "Hello", "role": "user" }
            ],
            "model": "deepseek-reasoner",
            "response_format": { "type": "json_object" },
            "thinking": { "type": "enabled" },
            "tools": [weather_wire_tool()],
            "tool_choice": "auto"
        })
    );
}

#[tokio::test]
async fn json_response_format_should_send_correct_request_body_with_schema() {
    let schema = json!({
        "type": "object",
        "properties": { "elements": { "type": "array", "items": { "type": "object",
            "properties": { "location": { "type": "string" }, "temperature": { "type": "number" },
                            "condition": { "type": "string" } },
            "required": ["location", "temperature", "condition"], "additionalProperties": false } } },
        "required": ["elements"], "additionalProperties": false,
        "$schema": "http://json-schema.org/draft-07/schema#"
    });
    let result = generate_result("deepseek-reasoner", &json_options(Some(schema.clone()))).await;
    let body = result.request.and_then(|r| r.body).unwrap();
    assert_eq!(
        body["messages"][0],
        json!({
            "role": "system",
            "content": format!("Return JSON that conforms to the following schema: {schema}")
        })
    );
    assert_eq!(
        body["messages"][1],
        json!({ "role": "user", "content": "Hello" })
    );
    assert_eq!(body["response_format"], json!({ "type": "json_object" }));
    // upstream: supportsStructuredOutputs is false, so the schema is injected
    // into the system message with a compatibility warning.
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[compatibility(
            "responseFormat JSON schema",
            "JSON response schema is injected into the system message."
        )])
    );
}

#[tokio::test]
async fn json_response_format_should_extract_text_content() {
    let mut body = text_response();
    body["choices"][0]["message"]["content"] = json!("{\"elements\":[]}");
    let server = json_server(body).await;
    let result = chat(&server, "deepseek-reasoner")
        .do_generate(&json_options(None))
        .await
        .unwrap();
    assert!(
        matches!(&result.content[0], GenerateContent::Text { text, .. } if text == "{\"elements\":[]}")
    );
}

// ---- describe('assistant prefix completion') --------------------------------------

fn prefix_prompt(prefix_options: Value) -> LanguageModelPrompt {
    vec![
        LanguageModelMessage::user_text("Complete this sentence."),
        with_options(
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: ("The answer is").into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
            json!({ "deepseek": prefix_options }),
        ),
    ]
}

#[tokio::test]
async fn prefix_should_send_name_and_prefix_on_the_final_assistant_message() {
    let server = json_server(text_response()).await;
    let body = beta_chat(&server, "deepseek-chat")
        .do_generate(&options_for(prefix_prompt(
            json!({ "name": "assistant", "prefix": true }),
        )))
        .await
        .unwrap()
        .request
        .and_then(|r| r.body)
        .unwrap();
    assert_eq!(
        body["messages"],
        json!([
            { "role": "user", "content": "Complete this sentence." },
            { "role": "assistant", "content": "The answer is", "name": "assistant", "prefix": true }
        ])
    );
    assert_eq!(body["model"], "deepseek-chat");
}

#[tokio::test]
async fn prefix_should_reject_prefix_completion_with_the_default_base_url() {
    let message = generate_error(
        "deepseek-chat",
        &options_for(vec![with_options(
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: ("The answer is").into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
            json!({ "deepseek": { "prefix": true } }),
        )]),
    )
    .await;
    assert!(
        message.contains(
            "DeepSeek assistant prefix completion requires a beta base URL ending in `/beta`."
        ),
        "{message}"
    );
}

// ===========================================================================
// describe('doStream')
// ===========================================================================

// A stream whose first event is an error is returned as the `do_stream` error
// (core's operation retry reads it); upstream emits it as an `error` part.
// The classification (status, retryability) is upstream's.
async fn stream_error(data: &Value) -> aimux_core::error::ApiCallError {
    let server = sse_server(vec![data_event(data)]).await;
    match chat(&server, "deepseek-chat").do_stream(&options()).await {
        Err(AiMuxError::ApiCall(error)) => *error,
        other => panic!(
            "expected an API call error, got {:?}",
            other.map(|_| "a stream")
        ),
    }
}

#[tokio::test]
async fn stream_should_preserve_a_provider_error_envelope_in_stream_errors() {
    let data = json!({ "error": {
        "message": "Rate limit reached", "type": "rate_limit_error", "code": "rate_limit_exceeded"
    } });
    let error = stream_error(&data).await;
    assert_eq!(error.message, "Rate limit reached");
    assert_eq!(error.provider_code.as_deref(), Some("rate_limit_exceeded"));
    assert_eq!(error.status_code, Some(429));
    assert!(error.is_retryable);
    // upstream: `data` is the whole error envelope.
    let envelope: Value = serde_json::from_str(error.response_body.as_deref().unwrap()).unwrap();
    assert_eq!(envelope, data);
}

#[tokio::test]
async fn stream_should_classify_insufficient_quota_as_non_retryable() {
    let data = json!({ "error": {
        "message": "You exceeded your current quota.", "type": "rate_limit_error",
        "code": "insufficient_quota"
    } });
    let error = stream_error(&data).await;
    assert_eq!(error.status_code, Some(429));
    assert!(!error.is_retryable);
}

#[tokio::test]
async fn stream_should_preserve_the_provider_type_when_code_is_an_http_status() {
    let data = json!({ "error": {
        "message": "Rate limit reached", "type": "rate_limit_error", "code": "429"
    } });
    let error = stream_error(&data).await;
    assert_eq!(error.provider_code.as_deref(), Some("429"));
    assert_eq!(error.status_code, Some(429));
    assert!(error.is_retryable);
}

// ---- describe('text') ------------------------------------------------------------

async fn stream_parts(
    model_id: &str,
    options: &CallOptions,
    chunks: Vec<String>,
) -> (Value, Vec<StreamPart>) {
    let server = sse_server(chunks).await;
    let result = chat(&server, model_id).do_stream(options).await.unwrap();
    let body = result.request.clone().and_then(|r| r.body).unwrap();
    (body, collect(result).await)
}

#[tokio::test]
async fn stream_text_should_send_model_id_settings_and_input() {
    let mut options = options_for(vec![
        LanguageModelMessage::System {
            content: ("You are a helpful assistant.").into(),
            provider_options: None,
        },
        LanguageModelMessage::user_text("Hello"),
    ]);
    options.temperature = Some(0.5);
    options.top_p = Some(0.3);
    let (body, _) = stream_parts("deepseek-chat", &options, text_chunks()).await;
    assert_eq!(
        body,
        json!({
            "messages": [
                { "content": "You are a helpful assistant.", "role": "system" },
                { "content": "Hello", "role": "user" }
            ],
            "model": "deepseek-chat",
            "stream": true,
            "stream_options": { "include_usage": true },
            "temperature": 0.5,
            "top_p": 0.3
        })
    );
}

#[tokio::test]
async fn stream_text_should_omit_deprecated_and_ineffective_sampling_options_in_default_v4_thinking_mode()
 {
    let mut options = options();
    options.temperature = Some(0.2);
    options.top_p = Some(0.4);
    options.frequency_penalty = Some(0.5);
    options.presence_penalty = Some(0.6);
    let (body, parts) = stream_parts("deepseek-v4-flash", &options, text_chunks()).await;
    assert_eq!(
        body,
        json!({
            "model": "deepseek-v4-flash",
            "messages": [{ "role": "user", "content": "Hello" }],
            "stream": true,
            "stream_options": { "include_usage": true }
        })
    );
    assert_eq!(
        stream_start_warnings(&parts),
        warning_values(&[
            deprecated("frequencyPenalty"),
            deprecated("presencePenalty"),
            thinking_unsupported("temperature"),
            thinking_unsupported("topP"),
        ])
    );
}

#[tokio::test]
async fn stream_text_should_preserve_supported_sampling_options_when_v4_thinking_is_disabled() {
    let mut options = with_provider_options(
        options(),
        json!({ "deepseek": { "thinking": { "type": "disabled" } } }),
    );
    options.temperature = Some(0.2);
    options.top_p = Some(0.4);
    options.frequency_penalty = Some(0.5);
    options.presence_penalty = Some(0.6);
    let (body, parts) = stream_parts("deepseek-v4-flash", &options, text_chunks()).await;
    assert_eq!(
        body,
        json!({
            "model": "deepseek-v4-flash",
            "messages": [{ "role": "user", "content": "Hello" }],
            "temperature": 0.2,
            "top_p": 0.4,
            "thinking": { "type": "disabled" },
            "stream": true,
            "stream_options": { "include_usage": true }
        })
    );
    assert_eq!(
        stream_start_warnings(&parts),
        warning_values(&[
            deprecated("frequencyPenalty"),
            deprecated("presencePenalty")
        ])
    );
}

#[tokio::test]
async fn stream_text_should_send_message_names() {
    let name = |value: &str| json!({ "deepseek": { "name": value } });
    let options = options_for(vec![
        with_options(
            LanguageModelMessage::System {
                content: ("You are a helpful assistant.").into(),
                provider_options: None,
            },
            name("guide"),
        ),
        with_options(LanguageModelMessage::user_text("Hello"), name("alice")),
        with_options(
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: ("Hello, Alice.").into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
            name("assistant"),
        ),
    ]);
    let (body, _) = stream_parts("deepseek-chat", &options, text_chunks()).await;
    assert_eq!(
        body["messages"],
        json!([
            { "content": "You are a helpful assistant.", "name": "guide", "role": "system" },
            { "content": "Hello", "name": "alice", "role": "user" },
            { "content": "Hello, Alice.", "name": "assistant", "role": "assistant" }
        ])
    );
}

#[tokio::test]
async fn stream_text_should_pass_provider_options_user_id_as_user_id() {
    let options = with_provider_options(
        options(),
        json!({ "deepseek": { "userId": "tenant_123-user" } }),
    );
    let (body, _) = stream_parts("deepseek-chat", &options, text_chunks()).await;
    assert_eq!(body["user_id"], "tenant_123-user");
    assert_eq!(body["stream"], true);
}

#[tokio::test]
async fn stream_text_should_stream_text() {
    let (_, parts) = stream_parts("deepseek-chat", &options(), text_chunks()).await;
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    assert!(matches!(
        &parts[1],
        StreamPart::ResponseMetadata(ResponseMetadata { id, model_id, .. })
            if id.as_deref() == Some("c1") && model_id.as_deref() == Some("deepseek-chat")
    ));
    let deltas: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextDelta { id, delta, .. } if id == "txt-0" => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, ["Hello", ", World!"]);
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::TextStart { id, .. } if id == "txt-0"))
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::TextEnd { id, .. } if id == "txt-0"))
    );
    match parts.last().unwrap() {
        StreamPart::Finish {
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(finish_reason.raw.as_deref(), Some("stop"));
            // usage with prompt-cache tokens
            assert_eq!(usage.input_tokens.total, Some(13));
            assert_eq!(usage.input_tokens.cache_read, Some(5));
            assert_eq!(usage.input_tokens.no_cache, Some(8));
            assert_eq!(usage.output_tokens.total, Some(3));
        }
        other => panic!("expected finish, got {other:?}"),
    }
    assert_eq!(
        finish_metadata(&parts),
        json!({ "deepseek": {
            "promptCacheHitTokens": 5,
            "promptCacheMissTokens": 8,
            "responseObject": "chat.completion.chunk",
            "choiceIndex": 0,
            "messageRole": "assistant",
            "systemFingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache"
        } })
    );
}

#[tokio::test]
async fn stream_text_should_include_the_repeated_system_fingerprint_in_provider_metadata() {
    let (_, parts) = stream_parts("deepseek-chat", &options(), text_chunks()).await;
    assert_eq!(
        finish_metadata(&parts)["deepseek"]["systemFingerprint"],
        "fp_eaab8d114b_prod0820_fp8_kvcache"
    );
}

#[tokio::test]
async fn stream_text_should_keep_the_latest_non_null_system_fingerprint() {
    let chunks = vec![
        "data: {\"system_fingerprint\":\"fp_initial\",\"choices\":[{\"delta\":{\"content\":\"OK\"},\"finish_reason\":null}],\"usage\":null}\n\n".to_string(),
        "data: {\"system_fingerprint\":null,\"choices\":[{\"delta\":{},\"finish_reason\":null}],\"usage\":null}\n\n".to_string(),
        "data: {\"system_fingerprint\":\"fp_latest\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":null}\n\n".to_string(),
    ];
    let (_, parts) = stream_parts("deepseek-chat", &options(), chunks).await;
    assert_eq!(
        finish_metadata(&parts)["deepseek"]["systemFingerprint"],
        "fp_latest"
    );
}

#[tokio::test]
async fn stream_text_should_tolerate_a_null_or_missing_system_fingerprint() {
    for fingerprint in [Some(Value::Null), None] {
        let mut chunk = json!({
            "choices": [{ "delta": { "content": "OK" }, "finish_reason": "stop" }],
            "usage": null
        });
        if let Some(value) = fingerprint {
            chunk["system_fingerprint"] = value;
        }
        let (_, parts) = stream_parts("deepseek-chat", &options(), vec![data_event(&chunk)]).await;
        assert!(
            finish_metadata(&parts)["deepseek"]
                .get("systemFingerprint")
                .is_none()
        );
    }
}

// ---- describe('reasoning') -------------------------------------------------------------

#[tokio::test]
async fn stream_reasoning_should_keep_reasoning_active_when_deltas_include_empty_tool_calls() {
    let chunk = |delta: Value, finish: Value| {
        data_event(&json!({
            "id": "chatcmpl-test", "object": "chat.completion.chunk", "created": 1,
            "model": "test-model",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
        }))
    };
    let chunks = vec![
        chunk(
            json!({ "role": "assistant", "content": "", "reasoning_content": "Think ", "tool_calls": [] }),
            Value::Null,
        ),
        chunk(
            json!({ "content": "", "reasoning_content": "more...", "tool_calls": [] }),
            Value::Null,
        ),
        chunk(
            json!({ "content": "Hello", "reasoning_content": "", "tool_calls": [] }),
            json!("stop"),
        ),
    ];
    let (_, parts) = stream_parts("deepseek-reasoner", &options(), chunks).await;
    let reasoning: Vec<String> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ReasoningStart { id, .. } => Some(format!("start {id}")),
            StreamPart::ReasoningDelta { id, delta, .. } => Some(format!("delta {id} {delta}")),
            StreamPart::ReasoningEnd { id, .. } => Some(format!("end {id}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasoning,
        [
            "start reasoning-0",
            "delta reasoning-0 Think ",
            "delta reasoning-0 more...",
            "end reasoning-0"
        ]
    );
}

#[tokio::test]
async fn stream_reasoning_should_map_legacy_thinking_and_generic_reasoning_to_canonical_request_values()
 {
    let options = with_provider_options(
        reasoning_options(ReasoningEffort::Medium),
        json!({ "deepseek": { "thinking": { "type": "adaptive" } } }),
    );
    let (body, parts) = stream_parts("deepseek-reasoner", &options, text_chunks()).await;
    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(
        stream_start_warnings(&parts),
        warning_values(&[
            compatibility(
                "thinking.type",
                "thinking.type \"adaptive\" is not a canonical DeepSeek value. mapped to \"enabled\"."
            ),
            compatibility(
                "reasoning",
                "reasoning \"medium\" is not directly supported by this model. mapped to effort \"high\"."
            ),
        ])
    );
}

#[tokio::test]
async fn stream_reasoning_should_stream_reasoning_before_text() {
    let chunk = |delta: Value, finish: Value| {
        data_event(&json!({
            "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "deepseek-reasoner",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
        }))
    };
    let chunks = vec![
        chunk(
            json!({ "role": "assistant", "content": null, "reasoning_content": "Let me " }),
            Value::Null,
        ),
        chunk(
            json!({ "content": null, "reasoning_content": "think." }),
            Value::Null,
        ),
        chunk(json!({ "content": "Answer" }), Value::Null),
        chunk(json!({ "content": "" }), json!("stop")),
    ];
    let (_, parts) = stream_parts("deepseek-reasoner", &options(), chunks).await;
    let order: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ReasoningStart { .. } => Some("reasoning-start"),
            StreamPart::ReasoningDelta { .. } => Some("reasoning-delta"),
            StreamPart::ReasoningEnd { .. } => Some("reasoning-end"),
            StreamPart::TextStart { .. } => Some("text-start"),
            StreamPart::TextDelta { .. } => Some("text-delta"),
            StreamPart::TextEnd { .. } => Some("text-end"),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        [
            "reasoning-start",
            "reasoning-delta",
            "reasoning-delta",
            "text-start",
            "reasoning-end",
            "text-delta",
            "text-end"
        ]
    );
}

// ---- describe('logprobs') ----------------------------------------------------------------

#[tokio::test]
async fn stream_logprobs_should_send_logprobs_provider_options_and_collect_streamed_logprobs() {
    let chunk = |choice: Value, usage: Value| {
        data_event(&json!({
            "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "deepseek-v4-flash",
            "choices": [choice], "usage": usage
        }))
    };
    let chunks = vec![
        chunk(
            json!({ "index": 0, "delta": { "role": "assistant", "reasoning_content": "Reasoning" },
                    "logprobs": { "reasoning_content": [logprob("Reasoning", -0.1, Value::Null)] },
                    "finish_reason": null }),
            Value::Null,
        ),
        chunk(
            json!({ "index": 0, "delta": { "content": "OK" },
                    "logprobs": { "content": [logprob("OK", -0.00002467602, json!([79, 75]))] },
                    "finish_reason": "stop" }),
            json!({ "prompt_tokens": 9, "completion_tokens": 2, "total_tokens": 11,
                    "prompt_cache_hit_tokens": 0, "prompt_cache_miss_tokens": 9 }),
        ),
    ];
    let options = with_provider_options(
        options(),
        json!({ "deepseek": { "logprobs": true, "topLogprobs": 1 } }),
    );
    let (body, parts) = stream_parts("deepseek-v4-flash", &options, chunks).await;
    assert_eq!(body["logprobs"], true);
    assert_eq!(body["top_logprobs"], 1);
    assert_eq!(
        finish_metadata(&parts),
        json!({ "deepseek": {
            "choiceIndex": 0,
            "logprobs": {
                "content": [logprob("OK", -0.00002467602, json!([79, 75]))],
                "reasoning_content": [logprob("Reasoning", -0.1, Value::Null)]
            },
            "messageRole": "assistant",
            "promptCacheHitTokens": 0,
            "promptCacheMissTokens": 9,
            "responseObject": "chat.completion.chunk"
        } })
    );
}

// ---- describe('tool call') ------------------------------------------------------------------

#[tokio::test]
async fn stream_tool_call_should_stream_tool_call() {
    let chunk = |delta: Value, finish: Value| {
        data_event(&json!({
            "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "deepseek-reasoner",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
        }))
    };
    let chunks = vec![
        chunk(
            json!({ "role": "assistant", "reasoning_content": "Look it up." }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "id": "call_00_abc", "type": "function",
                    "function": { "name": "weather", "arguments": "" } }] }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "{\"location\":" } }] }),
            Value::Null,
        ),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "\"San Francisco\"}" } }] }),
            Value::Null,
        ),
        chunk(json!({ "content": "" }), json!("tool_calls")),
    ];
    let (_, parts) = stream_parts("deepseek-reasoner", &tool_options(), chunks).await;
    assert!(parts.iter().any(|p| matches!(
        p,
        StreamPart::ToolInputStart { id, tool_name, .. } if id == "call_00_abc" && tool_name == "weather"
    )));
    let input: String = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolInputDelta { id, delta, .. } if id == "call_00_abc" => {
                Some(delta.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(input, "{\"location\":\"San Francisco\"}");
    assert!(parts.iter().any(|p| matches!(
        p,
        StreamPart::ToolCall(RawToolCall { tool_call_id, tool_name, input, .. })
            if tool_call_id == "call_00_abc" && tool_name == "weather"
                && input == "{\"location\":\"San Francisco\"}"
    )));
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { .. }))
    );
    match parts.last().unwrap() {
        StreamPart::Finish { finish_reason, .. } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
        }
        other => panic!("expected finish, got {other:?}"),
    }
    assert_eq!(
        finish_metadata(&parts)["deepseek"]["toolCallTypes"],
        json!(["function"])
    );
}

// ---- describe('assistant prefix completion') -----------------------------------------------------

#[tokio::test]
async fn stream_prefix_should_send_prefix_true_on_the_final_assistant_message() {
    let server = sse_server(text_chunks()).await;
    let result = beta_chat(&server, "deepseek-chat")
        .do_stream(&options_for(prefix_prompt(json!({ "prefix": true }))))
        .await
        .unwrap();
    let body = result.request.and_then(|r| r.body).unwrap();
    assert_eq!(
        body["messages"],
        json!([
            { "role": "user", "content": "Complete this sentence." },
            { "role": "assistant", "content": "The answer is", "prefix": true }
        ])
    );
    assert_eq!(body["model"], "deepseek-chat");
    assert_eq!(body["stream"], true);
}

// ===========================================================================
// convert-to-deepseek-chat-messages.test.ts (through the request body)
// ===========================================================================

fn png_data() -> UserPart {
    UserPart::File(FilePart {
        data: FileData::Data {
            data: FileBytes::Base64("AAECAw==".into()),
        },
        media_type: "image/png".into(),
        filename: None,
        provider_options: None,
    })
}

fn file_with_options(mut part: UserPart, options: Value) -> UserPart {
    if let UserPart::File(file) = &mut part {
        file.provider_options = Some(serde_json::from_value(options).unwrap());
    }
    part
}

// ---- describe('message names') -------------------------------------------------------------

#[tokio::test]
async fn convert_should_ignore_a_name_on_a_tool_message_with_an_unsupported_warning() {
    let options = options_for(vec![with_options(
        LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: ("call-1").into(),
                result: json!("sunny"),
                tool_name: None,
                is_error: None,
                preliminary: None,
                dynamic: None,
                provider_options: None,
            })],
            provider_options: None,
        },
        json!({ "deepseek": { "name": "weather_tool" } }),
    )]);
    let result = generate_result("deepseek-chat", &options).await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["messages"],
        json!([{ "role": "tool", "tool_call_id": "call-1", "content": "sunny" }])
    );
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[Warning::Unsupported {
            feature: "message name on tool messages".to_string(),
            details: None,
        }])
    );
}

#[tokio::test]
async fn convert_should_reject_a_non_string_name() {
    let options = options_for(vec![with_options(
        LanguageModelMessage::user_text("Hello"),
        json!({ "deepseek": { "name": 123 } }),
    )]);
    let message = generate_error("deepseek-chat", &options).await;
    assert!(message.contains("invalid provider options"), "{message}");
}

#[tokio::test]
async fn convert_should_serialize_a_name_from_a_custom_provider_options_namespace() {
    let server = json_server(text_response()).await;
    let model = create_deepseek(DeepSeekProviderSettings {
        name: Some("azure".to_string()),
        ..settings(&server, "")
    })
    .unwrap()
    .chat("deepseek-chat");
    let options = options_for(vec![with_options(
        LanguageModelMessage::user_text("Hello"),
        json!({ "azure": { "name": "alice" } }),
    )]);
    let result = model.do_generate(&options).await.unwrap();
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["messages"],
        json!([{ "role": "user", "content": "Hello", "name": "alice" }])
    );
}

// ---- describe('user messages') ---------------------------------------------------------------

#[tokio::test]
async fn convert_should_convert_messages_with_only_a_text_part_to_a_string_content() {
    let messages = wire_messages(
        "deepseek-chat",
        vec![LanguageModelMessage::user_text("Hello")],
    )
    .await;
    assert_eq!(messages, [json!({ "role": "user", "content": "Hello" })]);
}

#[tokio::test]
async fn convert_should_convert_image_data_to_an_image_url_content_part() {
    let messages = wire_messages(
        "deepseek-chat",
        vec![LanguageModelMessage::User {
            content: vec![
                UserPart::Text(TextPart {
                    text: ("Hello").into(),
                    provider_options: None,
                }),
                png_data(),
            ],
            provider_options: None,
        }],
    )
    .await;
    assert_eq!(
        messages,
        [json!({ "role": "user", "content": [
            { "type": "text", "text": "Hello" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAECAw==" } }
        ] })]
    );
}

#[tokio::test]
async fn convert_should_convert_an_image_url_to_an_image_url_content_part() {
    let messages = wire_messages(
        "deepseek-chat",
        vec![LanguageModelMessage::User {
            content: vec![
                UserPart::Text(TextPart {
                    text: ("Hello").into(),
                    provider_options: None,
                }),
                UserPart::File(FilePart {
                    data: FileData::Url {
                        url: ("https://example.com/image.png").into(),
                    },
                    media_type: ("image/png").into(),
                    filename: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }],
    )
    .await;
    assert_eq!(
        messages,
        [json!({ "role": "user", "content": [
            { "type": "text", "text": "Hello" },
            { "type": "image_url", "image_url": { "url": "https://example.com/image.png" } }
        ] })]
    );
}

#[tokio::test]
async fn convert_should_pass_image_detail_to_image_url_content_parts() {
    let messages = wire_messages(
        "deepseek-v4-flash-vision-exp",
        vec![LanguageModelMessage::User {
            content: vec![file_with_options(
                UserPart::File(FilePart {
                    data: FileData::Url {
                        url: ("https://example.com/image.webp").into(),
                    },
                    media_type: ("image/webp").into(),
                    filename: None,
                    provider_options: None,
                }),
                json!({ "deepseek": { "imageDetail": "low" } }),
            )],
            provider_options: None,
        }],
    )
    .await;
    assert_eq!(
        messages,
        [json!({ "role": "user", "content": [
            { "type": "image_url", "image_url": { "url": "https://example.com/image.webp", "detail": "low" } }
        ] })]
    );
}

#[tokio::test]
async fn convert_should_convert_inline_image_data_to_file_data_and_preserve_its_filename() {
    let part = UserPart::File(FilePart {
        data: FileData::Data {
            data: FileBytes::Base64("AAECAw==".to_string()),
        },
        media_type: "image/jpg".to_string(),
        filename: Some("sample.jpg".to_string()),
        provider_options: Some(provider_namespace("deepseek", json!({ "fileData": true }))),
    });
    let messages = wire_messages(
        "deepseek-v4-flash-vision-exp",
        vec![LanguageModelMessage::User {
            content: vec![part],
            provider_options: None,
        }],
    )
    .await;
    assert_eq!(
        messages,
        [json!({ "role": "user", "content": [
            { "type": "file", "file_data": "data:image/jpeg;base64,AAECAw==", "filename": "sample.jpg" }
        ] })]
    );
}

#[tokio::test]
async fn convert_should_reject_image_detail_together_with_file_data() {
    let part = file_with_options(
        png_data(),
        json!({ "deepseek": { "fileData": true, "imageDetail": "high" } }),
    );
    let message = generate_error(
        "deepseek-v4-flash-vision-exp",
        &options_for(vec![LanguageModelMessage::User {
            content: vec![part],
            provider_options: None,
        }]),
    )
    .await;
    assert!(
        message.contains("DeepSeek `imageDetail` cannot be combined with `fileData`."),
        "{message}"
    );
}

#[tokio::test]
async fn convert_should_reject_image_urls_longer_than_8192_characters() {
    let url = format!("https://example.com/{}", "a".repeat(8192));
    let message = generate_error(
        "deepseek-v4-flash-vision-exp",
        &options_for(vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Url { url: (url) },
                media_type: ("image/png").into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }]),
    )
    .await;
    assert!(
        message.contains("DeepSeek image URLs must not exceed 8192 characters."),
        "{message}"
    );
}

#[tokio::test]
async fn convert_should_reject_unsupported_image_formats() {
    let message = generate_error(
        "deepseek-v4-flash-vision-exp",
        &options_for(vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Base64(("AAECAw==").into()),
                },
                media_type: ("image/svg+xml").into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }]),
    )
    .await;
    assert!(
        message.contains("DeepSeek supports JPEG, PNG, GIF, and WebP image inputs."),
        "{message}"
    );
}

#[tokio::test]
async fn convert_should_convert_an_image_provider_reference_to_a_file_content_part() {
    let messages = wire_messages(
        "deepseek-v4-flash-vision-exp",
        vec![LanguageModelMessage::User {
            content: vec![
                UserPart::Text(TextPart {
                    text: ("Hello").into(),
                    provider_options: None,
                }),
                UserPart::File(FilePart {
                    data: FileData::Reference {
                        reference: serde_json::from_value(
                            json!({ "deepseek": "file-api-deepseek", "openai": "file-openai" }),
                        )
                        .unwrap(),
                    },
                    media_type: ("image/png").into(),
                    filename: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }],
    )
    .await;
    assert_eq!(
        messages,
        [json!({ "role": "user", "content": [
            { "type": "text", "text": "Hello" },
            { "type": "file", "file_id": "file-api-deepseek" }
        ] })]
    );
}

#[tokio::test]
async fn convert_should_throw_when_an_image_reference_has_no_deepseek_identifier() {
    let message = generate_error(
        "deepseek-v4-flash-vision-exp",
        &options_for(vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Reference {
                    reference: serde_json::from_value(json!({ "openai": "file-openai" })).unwrap(),
                },
                media_type: ("image/png").into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }]),
    )
    .await;
    assert!(message.contains("deepseek"), "{message}");
}

#[tokio::test]
async fn convert_should_warn_about_unsupported_non_image_file_parts() {
    let result = generate_result(
        "deepseek-chat",
        &options_for(vec![LanguageModelMessage::User {
            content: vec![
                UserPart::Text(TextPart {
                    text: ("Hello").into(),
                    provider_options: None,
                }),
                UserPart::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64(("AAECAw==").into()),
                    },
                    media_type: ("application/pdf").into(),
                    filename: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }]),
    )
    .await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["messages"],
        json!([{ "role": "user", "content": "Hello" }])
    );
    assert_eq!(
        warning_values(&result.warnings),
        warning_values(&[Warning::Unsupported {
            feature: "user message part type: file".to_string(),
            details: None,
        }])
    );
}

// ---- describe('tool calls') ------------------------------------------------------------------

fn tool_turn(reasoning: bool) -> Vec<LanguageModelMessage> {
    let mut assistant = vec![];
    if reasoning {
        assistant.push(AssistantPart::Reasoning(ReasoningPart {
            text: ("I think the tool will return the correct value.").into(),
            signature: None,
            provider_options: None,
        }));
    }
    assistant.push(AssistantPart::ToolCall(ToolCallPart {
        tool_call_id: ("quux").into(),
        tool_name: ("thwomp").into(),
        input: json!({ "foo": "bar123" }),
        provider_executed: None,
        thought_signature: None,
        provider_options: None,
    }));
    vec![
        LanguageModelMessage::Assistant {
            content: assistant,
            provider_options: None,
        },
        LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: ("quux").into(),
                result: json!({ "oof": "321rab" }),
                tool_name: None,
                is_error: None,
                preliminary: None,
                dynamic: None,
                provider_options: None,
            })],
            provider_options: None,
        },
    ]
}

fn wire_tool_call() -> Value {
    json!([{
        "id": "quux", "type": "function",
        "function": { "name": "thwomp", "arguments": "{\"foo\":\"bar123\"}" }
    }])
}

#[tokio::test]
async fn convert_should_stringify_arguments_to_tool_calls() {
    let result = generate_result("deepseek-chat", &options_for(tool_turn(false))).await;
    assert_eq!(
        result.request.and_then(|r| r.body).unwrap()["messages"],
        json!([
            // upstream sends the empty text as `content: ""`, not null.
            { "role": "assistant", "content": "", "tool_calls": wire_tool_call() },
            { "role": "tool", "tool_call_id": "quux", "content": "{\"oof\":\"321rab\"}" }
        ])
    );
    assert!(result.warnings.is_empty());
}

#[tokio::test]
async fn convert_should_handle_text_output_type_in_tool_results() {
    let messages = wire_messages(
        "deepseek-chat",
        vec![
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: ("call-1").into(),
                    tool_name: ("getWeather").into(),
                    input: json!({ "query": "weather" }),
                    provider_executed: None,
                    thought_signature: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: ("call-1").into(),
                    result: json!("It is sunny today"),
                    tool_name: None,
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
        ],
    )
    .await;
    assert_eq!(
        messages[1],
        json!({ "role": "tool", "tool_call_id": "call-1", "content": "It is sunny today" })
    );
}

#[tokio::test]
async fn convert_should_support_reasoning_content_in_tool_calls() {
    let mut prompt = vec![LanguageModelMessage::user_text("Hello")];
    prompt.extend(tool_turn(true));
    let messages = wire_messages("deepseek-chat", prompt).await;
    assert_eq!(
        messages[1],
        json!({
            "role": "assistant", "content": "",
            "reasoning_content": "I think the tool will return the correct value.",
            "tool_calls": wire_tool_call()
        })
    );
}

#[tokio::test]
async fn convert_should_filter_out_reasoning_content_from_turns_before_the_last_user_message() {
    let mut prompt = vec![LanguageModelMessage::user_text("Hello")];
    prompt.extend(tool_turn(true));
    prompt.push(LanguageModelMessage::user_text("Goodbye"));
    let messages = wire_messages("deepseek-chat", prompt).await;
    assert_eq!(
        messages[1],
        json!({ "role": "assistant", "content": "", "tool_calls": wire_tool_call() })
    );
}

// ---- describe('deepseek-v4 thinking mode') -------------------------------------------------------

#[tokio::test]
async fn convert_should_preserve_reasoning_content_from_prior_turns_for_deepseek_v4() {
    let mut prompt = vec![LanguageModelMessage::user_text("Hello")];
    prompt.extend(tool_turn(true));
    prompt.push(LanguageModelMessage::user_text("Goodbye"));
    let messages = wire_messages("deepseek-v4-pro", prompt).await;
    assert_eq!(
        messages[1],
        json!({
            "role": "assistant", "content": "",
            "reasoning_content": "I think the tool will return the correct value.",
            "tool_calls": wire_tool_call()
        })
    );
}

#[tokio::test]
async fn convert_should_preserve_reasoning_content_from_prior_turns_for_the_deepseek_flash_alias() {
    let messages = wire_messages(
        "deepseek-flash",
        vec![
            LanguageModelMessage::user_text("Hello"),
            LanguageModelMessage::Assistant {
                content: vec![
                    AssistantPart::Reasoning(ReasoningPart {
                        text: ("Prior-turn reasoning.").into(),
                        signature: None,
                        provider_options: None,
                    }),
                    AssistantPart::Text(TextPart {
                        text: ("Hi there").into(),
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            },
            LanguageModelMessage::user_text("Again"),
        ],
    )
    .await;
    assert_eq!(
        messages[1],
        json!({ "role": "assistant", "content": "Hi there", "reasoning_content": "Prior-turn reasoning." })
    );
}

#[tokio::test]
async fn convert_should_back_fill_empty_reasoning_content_for_deepseek_v4_assistant_messages_with_no_reasoning_part()
 {
    let messages = wire_messages(
        "deepseek-v4-pro",
        vec![
            LanguageModelMessage::user_text("Hello"),
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: ("Hi there").into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::user_text("Again"),
        ],
    )
    .await;
    assert_eq!(
        messages[1],
        json!({ "role": "assistant", "content": "Hi there", "reasoning_content": "" })
    );
}

// ---- describe('assistant prefix completion') -----------------------------------------------------

#[tokio::test]
async fn convert_should_reject_prefix_completion_on_a_non_assistant_message() {
    let message = generate_error(
        "deepseek-chat",
        &options_for(vec![with_options(
            LanguageModelMessage::user_text("Hello"),
            json!({ "deepseek": { "prefix": true } }),
        )]),
    )
    .await;
    assert!(
        message.contains(
            "DeepSeek assistant prefix completion requires `prefix: true` on an assistant message."
        ),
        "{message}"
    );
}

#[tokio::test]
async fn convert_should_reject_prefix_completion_on_a_non_final_assistant_message() {
    let server = json_server(text_response()).await;
    let prompt = vec![
        with_options(
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: ("The answer is").into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
            json!({ "deepseek": { "prefix": true } }),
        ),
        LanguageModelMessage::user_text("Continue"),
    ];
    let error = beta_chat(&server, "deepseek-chat")
        .do_generate(&options_for(prompt))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains(
            "DeepSeek assistant prefix completion requires the prefixed assistant message to be the final message."
        ),
        "{error}"
    );
}
