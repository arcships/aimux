//! Rust translations of the AI SDK Groq provider tests.
//!
//! Sources (TS → Rust):
//! - `packages/groq/src/convert-to-groq-chat-messages.test.ts` → message
//!   conversion tests (checked via do_generate request body)
//! - `packages/groq/src/convert-groq-usage.test.ts` → usage conversion tests
//!   (checked via do_generate result.usage)
//! - `packages/groq/src/groq-prepare-tools.test.ts` → tool preparation tests
//!   (checked via do_generate request body)
//! - `packages/groq/src/groq-chat-language-model.test.ts` → doGenerate/doStream
//!   behaviour tests
//! - `packages/groq/src/groq-chat-language-model-options.test.ts` → Zod schema
//!   validation (TypeScript-specific; not directly translatable)
//!
//! Every test builds the model through the Groq package
//! (`create_groq(..).chat(model)`), so it exercises `GroqChatLanguageModel`.
//!
//! A `partial:` marker names an upstream case the test covers only in part,
//! with the file that has the full port or the reason it cannot match.

use aimux_core::tool::RawToolCall;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice, ResponseFormat, Tool};
use aimux_core::provider::Provider;
use aimux_core::result::{GenerateContent, ReasoningOutput};
use aimux_core::shared::{FileBytes, FileData, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};

use aimux_providers::groq::{GroqChatLanguageModel, GroqProviderSettings, create_groq, groq};

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

/// `CallOptions` with only `prompt` set.
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// A Groq chat model pointed at the mock server.
fn model_at(server: &MockServer, model_id: &str) -> GroqChatLanguageModel {
    create_groq(GroqProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some("test-api-key".to_string()),
        ..Default::default()
    })
    .expect("groq provider should build")
    .chat(model_id)
}

fn browser_search_model_ids() -> Vec<&'static str> {
    include_str!("../src/groq/browser_search_models.rs")
        .split('"')
        .filter(|value| value.starts_with("openai/"))
        .collect()
}

/// Build the default test model pointed at the mock server.
fn make_provider(server: &MockServer) -> GroqChatLanguageModel {
    model_at(server, "gemma2-9b-it")
}

/// A standard non-streaming chat-completion JSON body returning "Hello, World!".
fn text_completion_body() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "gemma2-9b-it",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}

/// `__fixtures__/groq-text.json`, as upstream reads it.
fn groq_text_body() -> Value {
    serde_json::from_str(include_str!("../src/groq/tests/fixtures/groq-text.json"))
        .expect("fixture is JSON")
}

/// `__fixtures__/groq-reasoning.json`, as upstream reads it.
fn groq_reasoning_body() -> Value {
    serde_json::from_str(include_str!(
        "../src/groq/tests/fixtures/groq-reasoning.json"
    ))
    .expect("fixture is JSON")
}

/// `__fixtures__/groq-tool-call.json`, as upstream reads it.
fn groq_tool_call_body() -> Value {
    serde_json::from_str(include_str!(
        "../src/groq/tests/fixtures/groq-tool-call.json"
    ))
    .expect("fixture is JSON")
}

/// Build a single SSE `data: <json>\n\n` event string.
fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

/// Concatenate SSE events and append the `[DONE]` sentinel.
fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Collect every `StreamPart` from a `StreamResult` into a `Vec`.
async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(_) => break,
        }
    }
    parts
}

/// Mount a JSON mock response on the server.
async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE mock response on the server.
async fn mock_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

/// Get the first received request body as JSON.
async fn first_request_body(server: &MockServer) -> Value {
    let requests = server
        .received_requests()
        .await
        .expect("no requests received");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("invalid JSON body");
    body
}

async fn failed_call(status: u16, body: Value) -> aimux_core::error::ApiCallError {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    match make_provider(&server)
        .do_generate(&default_options(test_prompt()))
        .await
    {
        Err(aimux_core::AiMuxError::ApiCall(error)) => *error,
        other => panic!("expected an API call error, got {other:?}"),
    }
}

fn groq_at(server: &MockServer) -> aimux_providers::groq::GroqProvider {
    create_groq(GroqProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some("test-api-key".to_string()),
        ..Default::default()
    })
    .unwrap()
}

fn options_with(provider_options: Value) -> CallOptions {
    let mut options = default_options(test_prompt());
    options.provider_options = Some(serde_json::from_value(provider_options).unwrap());
    options
}

// ════════════════════════════════════════════════════════════════════════════
// convert-to-groq-chat-messages.test.ts
// ════════════════════════════════════════════════════════════════════════════
mod convert_to_groq_chat_messages {
    use super::*;

    mod user_messages {
        use super::*;

        /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › user messages › should convert messages with image parts
        #[tokio::test]
        async fn image_parts_from_base64() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                content: vec![
                    UserPart::Text(TextPart {
                        text: "Hello".into(),
                        provider_options: None,
                    }),
                    UserPart::File(FilePart {
                        data: FileData::Data {
                            data: FileBytes::Base64("AAECAw==".into()),
                        },
                        media_type: "image/png".into(),
                        filename: None,
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];
            model.do_generate(&default_options(prompt)).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["messages"],
                json!([{
                    "content": [
                        { "text": "Hello", "type": "text" },
                        { "image_url": { "url": "data:image/png;base64,AAECAw==" }, "type": "image_url" }
                    ],
                    "role": "user"
                }])
            );
        }

        /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › user messages › should convert messages with only a text part to a string content
        #[tokio::test]
        async fn single_text_becomes_string() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["messages"],
                json!([{ "content": "Hello", "role": "user" }])
            );
        }
    }

    mod tool_calls {
        use super::*;

        /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should stringify arguments to tool calls
        #[tokio::test]
        async fn tool_call_arguments_stringified() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let prompt: LanguageModelPrompt = vec![
                LanguageModelMessage::Assistant {
                    content: vec![AssistantPart::ToolCall(ToolCallPart {
                        tool_call_id: "quux".into(),
                        tool_name: "thwomp".into(),
                        input: json!({"foo":"bar123"}),
                        provider_executed: None,
                        provider_options: None,
                    })],
                    provider_options: None,
                },
                LanguageModelMessage::Tool {
                    content: vec![ToolPart::ToolResult(ToolResultPart {
                        tool_call_id: "quux".into(),
                        output: ToolResultOutput::Json {
                            value: json!({"oof":"321rab"}),
                            provider_options: None,
                        },
                        tool_name: "thwomp".into(),
                        provider_options: None,
                    })],
                    provider_options: None,
                },
            ];
            model.do_generate(&default_options(prompt)).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["messages"],
                json!([
                    {
                        "content": "",
                        "role": "assistant",
                        "tool_calls": [{
                            "function": { "arguments": r#"{"foo":"bar123"}"#, "name": "thwomp" },
                            "id": "quux",
                            "type": "function"
                        }]
                    },
                    { "content": r#"{"oof":"321rab"}"#, "role": "tool", "tool_call_id": "quux" }
                ])
            );
        }

        /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should send reasoning if present
        #[tokio::test]
        async fn reasoning_in_assistant_message() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Assistant {
                content: vec![
                    AssistantPart::Reasoning(ReasoningPart {
                        text: "I think the tool will return the correct value.".into(),
                        provider_options: None,
                    }),
                    AssistantPart::ToolCall(ToolCallPart {
                        tool_call_id: "quux".into(),
                        tool_name: "thwomp".into(),
                        input: json!({"foo":"bar123"}),
                        provider_executed: None,
                        provider_options: None,
                    }),
                ],
                provider_options: None,
            }];
            model.do_generate(&default_options(prompt)).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["messages"],
                json!([{
                    "content": "",
                    "reasoning": "I think the tool will return the correct value.",
                    "role": "assistant",
                    "tool_calls": [{
                        "function": { "arguments": r#"{"foo":"bar123"}"#, "name": "thwomp" },
                        "id": "quux",
                        "type": "function"
                    }]
                }])
            );
        }

        /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should not include reasoning field when no reasoning content is present
        #[tokio::test]
        async fn no_reasoning_field_when_absent() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: "Hello, how can I help you?".into(),
                    provider_options: None,
                })],
                provider_options: None,
            }];
            model.do_generate(&default_options(prompt)).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["messages"],
                json!([{ "content": "Hello, how can I help you?", "role": "assistant" }])
            );
        }

        /// Groq does not support provider file references.
        /// partial: `should throw for file parts with provider references`; upstream checks the message `'file parts with provider references' functionality not supported`, and aimux's `UnsupportedFunctionality` carries only the functionality name (the message field is tracked in #275).
        #[tokio::test]
        async fn file_reference_unsupported() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
                content: vec![UserPart::File(FilePart {
                    data: FileData::Reference {
                        reference: [("groq".into(), "file-ref-123".into())].into(),
                    },
                    media_type: "image/png".into(),
                    filename: None,
                    provider_options: None,
                })],
                provider_options: None,
            }];
            // upstream convert-to-groq-chat-messages.ts: a provider reference
            // throws UnsupportedFunctionalityError.
            let result = model.do_generate(&default_options(prompt)).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(matches!(
                err,
                aimux_core::AiMuxError::UnsupportedFunctionality(_)
            ));
            assert!(
                err.to_string()
                    .contains("file parts with provider references"),
                "error should name the unsupported functionality: {err}"
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convert-groq-usage.test.ts
// ════════════════════════════════════════════════════════════════════════════
mod convert_groq_usage {
    use super::*;

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should convert basic usage without token details
    #[tokio::test]
    async fn basic_usage() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 20, "completion_tokens": 10 }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 20, "noCache": 20 },
                "outputTokens": { "total": 10, "text": 10 },
                "raw": { "prompt_tokens": 20, "completion_tokens": 10 }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should extract reasoning tokens from completion_tokens_details
    #[tokio::test]
    async fn reasoning_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 79,
                    "completion_tokens": 40,
                    "completion_tokens_details": { "reasoning_tokens": 21 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 79, "noCache": 79 },
                "outputTokens": { "total": 40, "text": 19, "reasoning": 21 },
                "raw": {
                    "prompt_tokens": 79,
                    "completion_tokens": 40,
                    "completion_tokens_details": { "reasoning_tokens": 21 }
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle zero reasoning tokens
    #[tokio::test]
    async fn zero_reasoning_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "completion_tokens_details": { "reasoning_tokens": 0 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 20, "noCache": 20 },
                "outputTokens": { "total": 10, "text": 10, "reasoning": 0 },
                "raw": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "completion_tokens_details": { "reasoning_tokens": 0 }
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle all tokens being reasoning tokens
    #[tokio::test]
    async fn all_reasoning_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 50,
                    "completion_tokens_details": { "reasoning_tokens": 50 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 20, "noCache": 20 },
                "outputTokens": { "total": 50, "text": 0, "reasoning": 50 },
                "raw": {
                    "prompt_tokens": 20,
                    "completion_tokens": 50,
                    "completion_tokens_details": { "reasoning_tokens": 50 }
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should map cached_tokens to cacheRead and subtract from noCache
    #[tokio::test]
    async fn cached_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 4641,
                    "completion_tokens": 1817,
                    "prompt_tokens_details": { "cached_tokens": 4608 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 4641, "noCache": 33, "cacheRead": 4608 },
                "outputTokens": { "total": 1817, "text": 1817 },
                "raw": {
                    "prompt_tokens": 4641,
                    "completion_tokens": 1817,
                    "prompt_tokens_details": { "cached_tokens": 4608 }
                }
            })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should treat zero cached_tokens as a cache miss (cacheRead 0)
    #[tokio::test]
    async fn zero_cached_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "prompt_tokens_details": { "cached_tokens": 0 }
                }
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage.input_tokens).unwrap(),
            json!({ "total": 20, "noCache": 20, "cacheRead": 0 })
        );
    }

    /// upstream: @ai-sdk/groq src/convert-groq-usage.test.ts › convertGroqUsage › should handle missing prompt_tokens and completion_tokens
    #[tokio::test]
    async fn missing_tokens() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gemma2-9b-it",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "" },
                    "finish_reason": "stop"
                }],
                "usage": {}
            }),
        )
        .await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": { "total": 0, "noCache": 0 },
                "outputTokens": { "total": 0, "text": 0 },
                "raw": {}
            })
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// groq-prepare-tools.test.ts
// ════════════════════════════════════════════════════════════════════════════
mod groq_prepare_tools {
    use super::*;

    mod prepare_tools {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should correctly prepare function tools
        #[tokio::test]
        async fn function_tools() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = FunctionTool {
                name: "testFunction".to_string(),
                description: Some("A test function".to_string()),
                input_schema: json!({"type": "object", "properties": {}}),
                strict: None,
                provider_options: None,
                input_examples: None,
            };
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["tools"],
                json!([{
                    "type": "function",
                    "function": {
                        "name": "testFunction",
                        "description": "A test function",
                        "parameters": { "type": "object", "properties": {} }
                    }
                }])
            );
            assert!(body.get("tool_choice").is_none());
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should add warnings for unsupported provider-defined tools
        #[tokio::test]
        async fn unsupported_provider_tool_warning() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = Tool::Provider(aimux_core::tool::ProviderTool {
                id: "some.unsupported_tool".parse().unwrap(),
                name: "unsupported_tool".to_string(),
                args: serde_json::Map::new(),
                supports_deferred_results: None,
            });
            let options = CallOptions {
                tools: Some(vec![tool]),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            // Upstream's prepared `tools: []`: no tool reaches the request.
            assert!(body.get("tools").is_none_or(|tools| *tools == json!([])));
            assert!(body.get("tool_choice").is_none());
            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([{
                    "feature": "provider-defined tool some.unsupported_tool",
                    "type": "unsupported"
                }])
            );
        }

        mod strict_mode_for_function_tools {
            use super::*;

            /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode when strict is true
            #[tokio::test]
            async fn strict_mode_true() {
                let server = MockServer::start().await;
                mock_json(&server, text_completion_body()).await;

                let model = make_provider(&server);

                let tool = FunctionTool {
                    name: "testFunction".to_string(),
                    description: Some("A test function".to_string()),
                    input_schema: json!({"type": "object", "properties": {}}),
                    strict: Some(true),
                    provider_options: None,
                    input_examples: None,
                };
                let options = CallOptions {
                    tools: Some(vec![Tool::from(tool)]),
                    ..default_options(test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(
                    body["tools"],
                    json!([{
                        "type": "function",
                        "function": {
                            "name": "testFunction",
                            "description": "A test function",
                            "parameters": { "type": "object", "properties": {} },
                            "strict": true
                        }
                    }])
                );
            }

            /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › strict mode for function tools › should pass through strict mode when strict is false
            #[tokio::test]
            async fn strict_mode_false() {
                let server = MockServer::start().await;
                mock_json(&server, text_completion_body()).await;

                let model = make_provider(&server);

                let tool = FunctionTool {
                    name: "testFunction".to_string(),
                    description: Some("A test function".to_string()),
                    input_schema: json!({"type": "object", "properties": {}}),
                    strict: Some(false),
                    provider_options: None,
                    input_examples: None,
                };
                let options = CallOptions {
                    tools: Some(vec![Tool::from(tool)]),
                    ..default_options(test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(
                    body["tools"],
                    json!([{
                        "type": "function",
                        "function": {
                            "name": "testFunction",
                            "description": "A test function",
                            "parameters": { "type": "object", "properties": {} },
                            "strict": false
                        }
                    }])
                );
            }

            /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › strict mode for function tools › should not include strict when strict is undefined
            #[tokio::test]
            async fn strict_mode_undefined() {
                let server = MockServer::start().await;
                mock_json(&server, text_completion_body()).await;

                let model = make_provider(&server);

                let tool =
                    FunctionTool::new("testFunction", json!({"type": "object", "properties": {}}))
                        .with_description("A test function");
                let options = CallOptions {
                    tools: Some(vec![Tool::from(tool)]),
                    ..default_options(test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(
                    body["tools"],
                    json!([{
                        "type": "function",
                        "function": {
                            "name": "testFunction",
                            "description": "A test function",
                            "parameters": { "type": "object", "properties": {} }
                        }
                    }])
                );
            }
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should handle tool choice "auto"
        #[tokio::test]
        async fn tool_choice_auto() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                tool_choice: Some(LanguageModelV4ToolChoice::Auto),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            // upstream groq-prepare-tools.ts: `case 'auto': tool_choice: 'auto'`
            assert_eq!(body["tool_choice"], "auto");
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should handle tool choice "required"
        #[tokio::test]
        async fn tool_choice_required() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                tool_choice: Some(LanguageModelV4ToolChoice::Required),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["tool_choice"], "required");
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should handle tool choice "none"
        #[tokio::test]
        async fn tool_choice_none() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                tool_choice: Some(LanguageModelV4ToolChoice::None),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["tool_choice"], "none");
        }

        /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › should handle tool choice "tool"
        #[tokio::test]
        async fn tool_choice_tool() {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let model = make_provider(&server);

            let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                tool_choice: Some(LanguageModelV4ToolChoice::Tool {
                    tool_name: "testFunction".to_string(),
                }),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body["tool_choice"],
                json!({ "type": "function", "function": { "name": "testFunction" } })
            );
        }

        mod browser_search_tool {
            use super::*;

            /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should handle browser search tool with supported model
            #[tokio::test]
            async fn browser_search_supported_model() {
                let server = MockServer::start().await;
                mock_json(&server, text_completion_body()).await;

                let model = model_at(&server, browser_search_model_ids()[1]);

                let tool = Tool::Provider(aimux_core::tool::ProviderTool {
                    id: "groq.browser_search".parse().unwrap(),
                    name: "browser_search".to_string(),
                    args: serde_json::Map::new(),
                    supports_deferred_results: None,
                });
                let options = CallOptions {
                    tools: Some(vec![tool]),
                    ..default_options(test_prompt())
                };
                let result = model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(body["tools"], json!([{ "type": "browser_search" }]));
                assert!(result.warnings.is_empty());
            }

            /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should warn when browser search is used with unsupported model
            #[tokio::test]
            async fn browser_search_unsupported_model() {
                let server = MockServer::start().await;
                mock_json(&server, text_completion_body()).await;

                let model = make_provider(&server);

                let tool = Tool::Provider(aimux_core::tool::ProviderTool {
                    id: "groq.browser_search".parse().unwrap(),
                    name: "browser_search".to_string(),
                    args: serde_json::Map::new(),
                    supports_deferred_results: None,
                });
                let options = CallOptions {
                    tools: Some(vec![tool]),
                    ..default_options(test_prompt())
                };
                let result = model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert!(body.get("tools").is_none_or(|tools| *tools == json!([])));
                assert_eq!(
                    serde_json::to_value(&result.warnings).unwrap(),
                    json!([{
                        "details": "Browser search is only supported on the following models: openai/gpt-oss-20b, openai/gpt-oss-120b. Current model: gemma2-9b-it",
                        "feature": "provider-defined tool groq.browser_search",
                        "type": "unsupported"
                    }])
                );
            }

            /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should handle mixed tools with model validation
            #[tokio::test]
            async fn mixed_tools_with_browser_search() {
                let server = MockServer::start().await;
                mock_json(&server, text_completion_body()).await;

                let model = model_at(&server, browser_search_model_ids()[0]);

                let func_tool =
                    FunctionTool::new("test-tool", json!({"type":"object","properties":{}}))
                        .with_description("A test tool");
                let browser_tool = Tool::Provider(aimux_core::tool::ProviderTool {
                    id: "groq.browser_search".parse().unwrap(),
                    name: "browser_search".to_string(),
                    args: serde_json::Map::new(),
                    supports_deferred_results: None,
                });
                let options = CallOptions {
                    tools: Some(vec![Tool::from(func_tool), browser_tool]),
                    ..default_options(test_prompt())
                };
                let result = model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(
                    body["tools"],
                    json!([
                        {
                            "type": "function",
                            "function": {
                                "name": "test-tool",
                                "description": "A test tool",
                                "parameters": { "type": "object", "properties": {} }
                            }
                        },
                        { "type": "browser_search" }
                    ])
                );
                assert!(result.warnings.is_empty());
            }

            /// upstream: @ai-sdk/groq src/groq-prepare-tools.test.ts › prepareTools › browser search tool › should validate all browser search supported models
            #[tokio::test]
            async fn browser_search_all_supported_models() {
                let model_ids = browser_search_model_ids();
                assert_eq!(model_ids, ["openai/gpt-oss-20b", "openai/gpt-oss-120b"]);
                for model_id in model_ids {
                    let server = MockServer::start().await;
                    mock_json(&server, text_completion_body()).await;

                    let model = model_at(&server, model_id);

                    let tool = Tool::Provider(aimux_core::tool::ProviderTool {
                        id: "groq.browser_search".parse().unwrap(),
                        name: "browser_search".to_string(),
                        args: serde_json::Map::new(),
                        supports_deferred_results: None,
                    });
                    let options = CallOptions {
                        tools: Some(vec![tool]),
                        ..default_options(test_prompt())
                    };
                    let result = model.do_generate(&options).await.unwrap();

                    let body = first_request_body(&server).await;
                    assert_eq!(body["tools"], json!([{ "type": "browser_search" }]));
                    assert!(result.warnings.is_empty());
                }
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// groq-chat-language-model.test.ts
// ════════════════════════════════════════════════════════════════════════════
mod groq_chat_language_model {
    use super::*;

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should reject a response without choices
        #[tokio::test]
        async fn rejects_a_response_without_choices() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "chatcmpl-empty",
                    "object": "chat.completion",
                    "created": 1711115037,
                    "model": "gemma2-9b-it",
                    "choices": [],
                    "usage": {"prompt_tokens": 4, "total_tokens": 4, "completion_tokens": 0}
                }),
            )
            .await;
            let model = make_provider(&server);

            let error = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap_err();
            assert!(
                matches!(&error, aimux_core::AiMuxError::InvalidResponseData { message: m, .. } if m == "Response did not contain any choices."),
                "{error:?}"
            );
        }

        mod text {
            use super::*;

            /// partial: `should extract text content`; full port in src/groq/tests/chat_language_model.rs.
            #[tokio::test]
            async fn extracts_text_content() {
                let server = MockServer::start().await;
                mock_json(&server, groq_text_body()).await;

                let model = make_provider(&server);

                let result = model
                    .do_generate(&default_options(test_prompt()))
                    .await
                    .unwrap();

                let text = result.content.iter().find_map(|c| match c {
                    GenerateContent::Text { text, .. } => Some(text.clone()),
                    _ => None,
                });
                assert!(text.is_some());
                assert!(text.unwrap().contains("Luminaria"));
            }

            /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › text › should send correct request body
            #[tokio::test]
            async fn correct_request_body() {
                let server = MockServer::start().await;
                mock_json(&server, groq_text_body()).await;

                let model = make_provider(&server);

                model
                    .do_generate(&default_options(test_prompt()))
                    .await
                    .unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(
                    body,
                    json!({
                        "messages": [{ "content": "Hello", "role": "user" }],
                        "model": "gemma2-9b-it"
                    })
                );
            }
        }

        mod tool_call {
            use super::*;

            /// partial: `should extract tool call content`; full port in src/groq/tests/chat_language_model.rs.
            #[tokio::test]
            async fn extracts_tool_call() {
                let server = MockServer::start().await;
                mock_json(&server, groq_tool_call_body()).await;

                let model = make_provider(&server);

                let result = model
                    .do_generate(&default_options(test_prompt()))
                    .await
                    .unwrap();

                let tool_call = result.content.iter().find_map(|c| match c {
                    GenerateContent::ToolCall(RawToolCall {
                        tool_call_id,
                        tool_name,
                        ..
                    }) => Some((tool_call_id.clone(), tool_name.clone())),
                    _ => None,
                });
                let (id, name) = tool_call.expect("should have tool call");
                assert_eq!(id, "ax9fskhev");
                assert_eq!(name, "weather");
                assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
            }
        }

        mod reasoning {
            use super::*;

            /// partial: `should extract reasoning content`; full port in src/groq/tests/chat_language_model.rs.
            #[tokio::test]
            async fn extracts_reasoning() {
                let server = MockServer::start().await;
                mock_json(&server, groq_reasoning_body()).await;

                let model = make_provider(&server);

                let result = model
                    .do_generate(&default_options(test_prompt()))
                    .await
                    .unwrap();

                let reasoning = result.content.iter().find_map(|c| match c {
                    GenerateContent::Reasoning(ReasoningOutput { text, .. }) => Some(text.clone()),
                    _ => None,
                });
                assert!(reasoning.is_some());
                assert!(reasoning.unwrap().contains("strawberry"));
            }
        }

        mod top_level_reasoning {
            use super::*;

            /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should map top-level reasoning to reasoning_effort
            #[tokio::test]
            async fn reasoning_effort_high() {
                let server = MockServer::start().await;
                mock_json(&server, groq_text_body()).await;

                let model = make_provider(&server);

                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::High),
                    ..default_options(test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(body["reasoning_effort"], "high");
            }

            /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should coerce top-level reasoning minimal to low
            #[tokio::test]
            async fn reasoning_effort_minimal_coerced_to_low() {
                let server = MockServer::start().await;
                mock_json(&server, groq_text_body()).await;

                let model = make_provider(&server);

                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::Minimal),
                    ..default_options(test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(body["reasoning_effort"], "low");
            }

            /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should coerce top-level reasoning xhigh to high
            #[tokio::test]
            async fn reasoning_effort_xhigh_coerced_to_high() {
                let server = MockServer::start().await;
                mock_json(&server, groq_text_body()).await;

                let model = make_provider(&server);

                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::Xhigh),
                    ..default_options(test_prompt())
                };
                model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(body["reasoning_effort"], "high");
            }

            /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should map top-level reasoning none to reasoning_effort for Qwen 3.6
            #[tokio::test]
            async fn reasoning_none_maps_for_qwen() {
                let server = MockServer::start().await;
                mock_json(&server, groq_text_body()).await;

                let model = model_at(&server, "qwen/qwen3.6-27b");

                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::None),
                    ..default_options(test_prompt())
                };
                let result = model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(body["reasoning_effort"], "none");
                assert!(result.warnings.is_empty());
            }

            /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should omit unsupported top-level reasoning none and warn
            #[tokio::test]
            async fn reasoning_none_omitted_and_warns_for_other_models() {
                let server = MockServer::start().await;
                mock_json(&server, groq_text_body()).await;

                let model = model_at(&server, browser_search_model_ids()[1]);

                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::None),
                    ..default_options(test_prompt())
                };
                let result = model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert!(body.get("reasoning_effort").is_none());
                assert!(result.warnings.iter().any(|w| matches!(
                    w,
                    aimux_core::types::Warning::Unsupported { feature, details }
                        if feature == "reasoning"
                            && details.as_deref()
                                == Some("reasoning \"none\" is not supported by this model.")
                )));
            }

            /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › top-level reasoning › should prefer providerOptions reasoningEffort over top-level reasoning
            #[tokio::test]
            async fn provider_option_reasoning_effort_preferred() {
                let server = MockServer::start().await;
                mock_json(&server, groq_text_body()).await;

                let model = model_at(&server, "openai/gpt-oss-120b");

                let provider_opts =
                    provider_namespace("groq", json!({"reasoningEffort": "high"})).unwrap();
                let options = CallOptions {
                    reasoning: Some(ReasoningEffort::Medium),
                    provider_options: Some(provider_opts),
                    ..default_options(test_prompt())
                };
                let result = model.do_generate(&options).await.unwrap();

                let body = first_request_body(&server).await;
                assert_eq!(body["reasoning_effort"], "high");
                assert!(result.warnings.is_empty());
            }
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should extract usage
        #[tokio::test]
        async fn extracts_usage() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.usage).unwrap(),
                json!({
                    "inputTokens": { "total": 45, "noCache": 45 },
                    "outputTokens": { "total": 607, "text": 607 },
                    "raw": { "completion_tokens": 607, "prompt_tokens": 45, "total_tokens": 652 }
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should support partial usage
        #[tokio::test]
        async fn partial_usage() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "created": 1711115037,
                    "model": "gemma2-9b-it",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": "" },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 20, "total_tokens": 20 }
                }),
            )
            .await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.usage).unwrap(),
                json!({
                    "inputTokens": { "total": 20, "noCache": 20 },
                    "outputTokens": { "total": 0, "text": 0 },
                    "raw": { "prompt_tokens": 20, "total_tokens": 20 }
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should extract cached input tokens
        #[tokio::test]
        async fn cached_input_tokens() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "created": 1711115037,
                    "model": "gemma2-9b-it",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": "" },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 20,
                        "total_tokens": 25,
                        "completion_tokens": 5,
                        "prompt_tokens_details": { "cached_tokens": 15 }
                    }
                }),
            )
            .await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.usage).unwrap(),
                json!({
                    "inputTokens": { "total": 20, "noCache": 5, "cacheRead": 15 },
                    "outputTokens": { "total": 5, "text": 5 },
                    "raw": {
                        "completion_tokens": 5,
                        "prompt_tokens": 20,
                        "prompt_tokens_details": { "cached_tokens": 15 },
                        "total_tokens": 25
                    }
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should extract reasoning tokens from completion_tokens_details
        #[tokio::test]
        async fn reasoning_tokens_from_details() {
            let server = MockServer::start().await;
            mock_json(&server, groq_reasoning_body()).await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.usage).unwrap(),
                json!({
                    "inputTokens": { "total": 17, "noCache": 17 },
                    "outputTokens": { "total": 649, "text": 79, "reasoning": 570 },
                    "raw": {
                        "completion_tokens": 649,
                        "completion_tokens_details": { "reasoning_tokens": 570 },
                        "prompt_tokens": 17,
                        "total_tokens": 666
                    }
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should support unknown finish reason
        #[tokio::test]
        async fn unknown_finish_reason() {
            let server = MockServer::start().await;
            mock_json(
                &server,
                json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "created": 1711115037,
                    "model": "gemma2-9b-it",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": "" },
                        "finish_reason": "eos"
                    }],
                    "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
                }),
            )
            .await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
            assert_eq!(result.finish_reason.raw.as_deref(), Some("eos"));
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass provider options
        #[tokio::test]
        async fn pass_provider_options() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let provider_opts = provider_namespace(
                "groq",
                json!({
                    "reasoningFormat": "hidden",
                    "user": "test-user-id",
                    "parallelToolCalls": false
                }),
            )
            .unwrap();
            let options = CallOptions {
                provider_options: Some(provider_opts),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it",
                    "parallel_tool_calls": false,
                    "reasoning_format": "hidden",
                    "user": "test-user-id"
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass serviceTier provider option
        #[tokio::test]
        async fn pass_service_tier_flex() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let provider_opts = provider_namespace("groq", json!({"serviceTier": "flex"})).unwrap();
            let options = CallOptions {
                provider_options: Some(provider_opts),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it",
                    "service_tier": "flex"
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass performance serviceTier provider option
        #[tokio::test]
        async fn pass_service_tier_performance() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let provider_opts =
                provider_namespace("groq", json!({"serviceTier": "performance"})).unwrap();
            let options = CallOptions {
                provider_options: Some(provider_opts),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it",
                    "service_tier": "performance"
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass tools and toolChoice
        #[tokio::test]
        async fn pass_tools_and_tool_choice() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let tool = FunctionTool::new(
                "test-tool",
                json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#"
                }),
            );
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                tool_choice: Some(LanguageModelV4ToolChoice::Tool {
                    tool_name: "test-tool".to_string(),
                }),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it",
                    "tool_choice": { "function": { "name": "test-tool" }, "type": "function" },
                    "tools": [{
                        "function": {
                            "name": "test-tool",
                            "parameters": {
                                "$schema": "http://json-schema.org/draft-07/schema#",
                                "additionalProperties": false,
                                "properties": { "value": { "type": "string" } },
                                "required": ["value"],
                                "type": "object"
                            }
                        },
                        "type": "function"
                    }]
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass response format information as json_schema when structuredOutputs enabled by default
        #[tokio::test]
        async fn response_format_json_schema_default() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let options = CallOptions {
                response_format: Some(ResponseFormat::Json {
                    schema: Some(json!({
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#"
                    })),
                    name: Some("test-name".to_string()),
                    description: Some("test description".to_string()),
                }),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it",
                    "response_format": {
                        "json_schema": {
                            "description": "test description",
                            "name": "test-name",
                            "schema": {
                                "$schema": "http://json-schema.org/draft-07/schema#",
                                "additionalProperties": false,
                                "properties": { "value": { "type": "string" } },
                                "required": ["value"],
                                "type": "object"
                            },
                            "strict": true
                        },
                        "type": "json_schema"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should pass response format information as json_object when structuredOutputs explicitly disabled
        #[tokio::test]
        async fn response_format_json_object_when_disabled() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let provider_opts =
                provider_namespace("groq", json!({"structuredOutputs": false})).unwrap();
            let options = CallOptions {
                provider_options: Some(provider_opts),
                response_format: Some(ResponseFormat::Json {
                    schema: Some(json!({
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#"
                    })),
                    name: Some("test-name".to_string()),
                    description: Some("test description".to_string()),
                }),
                ..default_options(test_prompt())
            };
            let result = model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it",
                    "response_format": { "type": "json_object" }
                })
            );
            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([{
                    "details": "JSON response format schema is only supported with structuredOutputs",
                    "feature": "responseFormat",
                    "type": "unsupported"
                }])
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should send strict: false when strictJsonSchema is explicitly disabled
        #[tokio::test]
        async fn strict_json_schema_false() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let provider_opts =
                provider_namespace("groq", json!({"strictJsonSchema": false})).unwrap();
            let options = CallOptions {
                provider_options: Some(provider_opts),
                response_format: Some(ResponseFormat::Json {
                    schema: Some(json!({
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#"
                    })),
                    name: Some("test-name".to_string()),
                    description: Some("test description".to_string()),
                }),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(
                body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it",
                    "response_format": {
                        "json_schema": {
                            "description": "test description",
                            "name": "test-name",
                            "schema": {
                                "$schema": "http://json-schema.org/draft-07/schema#",
                                "additionalProperties": false,
                                "properties": { "value": { "type": "string" } },
                                "required": ["value"],
                                "type": "object"
                            },
                            "strict": false
                        },
                        "type": "json_schema"
                    }
                })
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doGenerate › should send request body
        #[tokio::test]
        async fn request_body_string() {
            let server = MockServer::start().await;
            mock_json(&server, groq_text_body()).await;

            let model = make_provider(&server);

            let result = model
                .do_generate(&default_options(test_prompt()))
                .await
                .unwrap();

            let request_body = result
                .request
                .and_then(|request| request.body)
                .expect("should have request body");
            assert_eq!(
                serde_json::to_string(&request_body).unwrap(),
                r#"{"model":"gemma2-9b-it","messages":[{"role":"user","content":"Hello"}]}"#
            );
        }
    }

    mod do_stream {
        use super::*;

        mod text {
            use super::*;

            /// partial: `should stream text`; full port in src/groq/tests/chat_language_model.rs.
            #[tokio::test]
            async fn streams_text() {
                let server = MockServer::start().await;
                let body = sse_body(&[
                    &sse_event(
                        r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
                    ),
                    &sse_event(
                        r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
                    ),
                    &sse_event(
                        r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
                    ),
                ]);
                mock_sse(&server, body).await;

                let model = make_provider(&server);

                let result = model
                    .do_stream(&default_options(test_prompt()))
                    .await
                    .unwrap();
                let parts = collect_stream(result).await;

                // Should have stream-start, response-metadata, text-start, text-delta x2, text-end, finish
                let text_deltas: Vec<String> = parts
                    .iter()
                    .filter_map(|p| match p {
                        StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(text_deltas, vec!["Hello", " world"]);

                // Check finish reason
                let finish = parts.iter().find_map(|p| match p {
                    StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
                    _ => None,
                });
                assert!(finish.is_some());
                assert_eq!(finish.unwrap().unified, FinishReasonUnified::Stop);
            }
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › should send correct streaming request body
        #[tokio::test]
        async fn streaming_request_body() {
            let server = MockServer::start().await;
            let body = sse_body(&[
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
                ),
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
                ),
            ]);
            mock_sse(&server, body).await;

            let model = make_provider(&server);

            model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap();

            let req_body = first_request_body(&server).await;
            assert_eq!(
                req_body,
                json!({
                    "messages": [{ "content": "Hello", "role": "user" }],
                    "model": "gemma2-9b-it",
                    "stream": true
                })
            );
        }

        /// partial: `should stream tool call deltas when tool call arguments are passed in the first chunk`; full port in src/groq/tests/chat_language_model.rs.
        #[tokio::test]
        async fn streams_tool_call_deltas() {
            let server = MockServer::start().await;
            let body = sse_body(&[
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"test-tool","arguments":"{"}}]},"finish_reason":null}]}"#,
                ),
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"value\":"}}]},"finish_reason":null}]}"#,
                ),
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"Sparkle Day\"}"}}]},"finish_reason":null}]}"#,
                ),
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"x_groq":{"usage":{"prompt_tokens":18,"completion_tokens":439,"total_tokens":457}}}"#,
                ),
            ]);
            mock_sse(&server, body).await;

            let model = make_provider(&server);

            let tool = FunctionTool::new(
                "test-tool",
                json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                }),
            );
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                ..default_options(test_prompt())
            };
            let result = model.do_stream(&options).await.unwrap();
            let parts = collect_stream(result).await;

            // Should have a ToolCall part
            let tool_call = parts.iter().find_map(|p| match p {
                StreamPart::ToolCall(RawToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    ..
                }) => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
                _ => None,
            });
            let (id, name, input) = tool_call.expect("should have tool call");
            assert_eq!(id, "call_abc");
            assert_eq!(name, "test-tool");
            assert_eq!(input, Value::String(r#"{"value":"Sparkle Day"}"#.into()));

            // Should have tool-calls finish reason
            let finish = parts.iter().find_map(|p| match p {
                StreamPart::Finish { finish_reason, .. } => Some(finish_reason),
                _ => None,
            });
            assert_eq!(finish.unwrap().unified, FinishReasonUnified::ToolCalls);
        }

        /// partial: `should handle error stream parts`; upstream snapshots the JavaScript `APICallError` object, and aimux's error has its own serialized shape, so the test checks its message, status and retryability.
        #[tokio::test]
        async fn handles_error_stream() {
            let server = MockServer::start().await;
            let body = sse_body(&[&sse_event(
                r#"{"error":{"message":"Rate limit reached","type":"rate_limit_error"}}"#,
            )]);
            mock_sse(&server, body).await;

            let model = make_provider(&server);

            let result = model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap();
            let parts = collect_stream(result).await;
            assert_eq!(parts.len(), 3);
            assert!(
                matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty())
            );
            match &parts[1] {
                StreamPart::Error {
                    error: aimux_core::AiMuxError::ApiCall(error),
                } => {
                    assert_eq!(error.message, "Rate limit reached");
                    assert_eq!(error.status_code, Some(429));
                    assert!(error.is_retryable);
                }
                other => panic!("expected Error, got {other:?}"),
            }
            match &parts[2] {
                StreamPart::Finish {
                    finish_reason,
                    usage,
                    ..
                } => {
                    assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
                    assert_eq!(finish_reason.raw, None);
                    assert_eq!(
                        serde_json::to_value(usage).unwrap(),
                        serde_json::to_value(aimux_core::types::Usage::default()).unwrap()
                    );
                }
                other => panic!("expected Finish, got {other:?}"),
            }
        }

        /// partial: `should stream tool call that is sent in one chunk`; full port in src/groq/tests/chat_language_model.rs.
        #[tokio::test]
        async fn streams_tool_call_one_chunk() {
            let server = MockServer::start().await;
            let body = sse_body(&[
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"test-tool","arguments":"{\"value\":\"Sparkle Day\"}"}}]},"finish_reason":null}]}"#,
                ),
                &sse_event(
                    r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"x_groq":{"usage":{"prompt_tokens":18,"completion_tokens":439,"total_tokens":457}}}"#,
                ),
            ]);
            mock_sse(&server, body).await;

            let model = make_provider(&server);

            let tool = FunctionTool::new(
                "test-tool",
                json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}),
            );
            let options = CallOptions {
                tools: Some(vec![Tool::from(tool)]),
                ..default_options(test_prompt())
            };
            let result = model.do_stream(&options).await.unwrap();
            let parts = collect_stream(result).await;

            let tool_call = parts.iter().find_map(|p| match p {
                StreamPart::ToolCall(RawToolCall { input, .. }) => Some(input.clone()),
                _ => None,
            });
            assert_eq!(
                tool_call.unwrap(),
                Value::String(r#"{"value":"Sparkle Day"}"#.into())
            );
        }

        mod reasoning {
            use super::*;

            /// upstream: @ai-sdk/groq src/groq-chat-language-model.test.ts › doStream › reasoning › should keep reasoning active when deltas include empty tool calls
            #[tokio::test]
            #[rustfmt::skip]
            async fn keeps_reasoning_active_with_empty_tool_calls() {
                let server = MockServer::start().await;
                let body = sse_body(&[
                    &sse_event(
                        r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning":"Think ","tool_calls":[]},"finish_reason":null}]}"#,
                    ),
                    &sse_event(
                        r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"content":"","reasoning":"more...","tool_calls":[]},"finish_reason":null}]}"#,
                    ),
                    &sse_event(
                        r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"content":"Hello","reasoning":"","tool_calls":[]},"finish_reason":"stop"}]}"#,
                    ),
                ]);
                mock_sse(&server, body).await;
                let model = make_provider(&server);

                let result = model
                    .do_stream(&default_options(test_prompt()))
                    .await
                    .unwrap();
                let parts = collect_stream(result).await;
                let reasoning: Vec<Value> = parts
                    .iter()
                    .map(|part| serde_json::to_value(part).unwrap())
                    .filter(|part| part["type"].as_str().is_some_and(|kind| kind.starts_with("reasoning-")))
                    .collect();
                assert_eq!(
                    reasoning,
                    [
                        json!({ "type": "reasoning-start", "id": "reasoning-0" }),
                        json!({ "type": "reasoning-delta", "id": "reasoning-0", "delta": "Think " }),
                        json!({ "type": "reasoning-delta", "id": "reasoning-0", "delta": "more..." }),
                        json!({ "type": "reasoning-end", "id": "reasoning-0" }),
                    ]
                );
            }
        }

        /// partial: `should handle unparsable stream parts`; upstream snapshots the JavaScript `JSONParseError` and its `SyntaxError` text, which aimux's parse error does not reproduce, so the test checks the part types and the finish part.
        #[tokio::test]
        async fn unparsable_chunk_is_an_error_part_and_finish_reason_error() {
            let server = MockServer::start().await;
            mock_sse(
                &server,
                "data: {unparsable}\n\ndata: [DONE]\n\n".to_string(),
            )
            .await;
            let model = make_provider(&server);

            let result = model
                .do_stream(&default_options(test_prompt()))
                .await
                .unwrap();
            let parts = collect_stream(result).await;

            assert_eq!(parts.len(), 3, "{parts:?}");
            assert!(
                matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty())
            );
            assert!(matches!(parts[1], StreamPart::Error { .. }), "{parts:?}");
            match &parts[2] {
                StreamPart::Finish {
                    finish_reason,
                    usage,
                    provider_metadata,
                } => {
                    assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
                    assert_eq!(finish_reason.raw, None);
                    assert_eq!(
                        serde_json::to_value(usage).unwrap(),
                        serde_json::to_value(aimux_core::types::Usage::default()).unwrap()
                    );
                    assert!(provider_metadata.is_none());
                }
                other => panic!("expected Finish, got {other:?}"),
            }
        }
    }

    mod do_stream_with_raw_chunks {
        use super::*;

        /// partial: `should stream raw chunks when includeRawChunks is true`; full port in src/groq/tests/chat_language_model.rs.
        #[tokio::test]
        async fn streams_raw_chunks() {
            let server = MockServer::start().await;
            let chunks = [
                r#"{"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
                r#"{"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
                r#"{"id":"chatcmpl-789","object":"chat.completion.chunk","created":1234567890,"model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"x_groq":{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}}"#,
            ];
            mock_sse(
                &server,
                sse_body(
                    &chunks
                        .map(sse_event)
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                ),
            )
            .await;
            let model = make_provider(&server);

            let options = CallOptions {
                include_raw_chunks: Some(true),
                ..default_options(test_prompt())
            };
            let result = model.do_stream(&options).await.unwrap();
            let parts = collect_stream(result).await;

            let raw: Vec<&Value> = parts
                .iter()
                .filter_map(|p| match p {
                    StreamPart::Raw { raw_value } => Some(raw_value),
                    _ => None,
                })
                .collect();
            assert_eq!(raw.len(), 3);
            for (raw, chunk) in raw.iter().zip(chunks) {
                assert_eq!(**raw, serde_json::from_str::<Value>(chunk).unwrap());
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Tests without an upstream case
// ════════════════════════════════════════════════════════════════════════════
/// `groqFailedResponseHandler` (groq@4.0.52 `groq-error.ts:4-16`): the
/// message is `error.message`, and `error.type` is the provider code.
#[tokio::test]
async fn a_failed_response_reads_groqs_error_schema() {
    let error = failed_call(
        400,
        json!({ "error": {
            "message": "The model `x` does not exist",
            "type": "invalid_request_error",
            "code": "model_not_found"
        } }),
    )
    .await;
    assert_eq!(error.message, "The model `x` does not exist");
    assert_eq!(
        error.provider_code.as_deref(),
        Some("invalid_request_error")
    );
    assert_eq!(error.status_code, Some(400));
    assert!(!error.is_retryable);
}

/// A body outside the schema falls back to the status text, as
/// `createJsonErrorResponseHandler` does when the schema does not parse.
#[tokio::test]
async fn a_failed_response_outside_the_schema_uses_the_status_text() {
    let error = failed_call(503, json!({ "message": "top-level only" })).await;
    assert_eq!(error.message, "Service Unavailable");
    assert_eq!(error.provider_code, None);
    assert!(error.is_retryable);
}

/// RFC-0016 M9: warnings computed while building the request body reach
/// `StreamStart` (previously hard-coded empty). Groq does not support
/// `top_k` → Unsupported warning.
#[tokio::test]
async fn stream_start_carries_body_build_warnings() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let model = make_provider(&server);

    let options = CallOptions {
        top_k: Some(0.5),
        ..default_options(test_prompt())
    };
    let result = model.do_stream(&options).await.unwrap();
    let parts = collect_stream(result).await;

    match &parts[0] {
        StreamPart::StreamStart { warnings } => {
            assert!(
                warnings.iter().any(|w| matches!(
                    w,
                    aimux_core::types::Warning::Unsupported { feature, .. }
                        if feature == "topK"
                )),
                "expected topK Unsupported warning in StreamStart, got {warnings:?}"
            );
        }
        other => panic!("expected StreamStart first, got {other:?}"),
    }
}

/// An error chunk after a content chunk becomes an Error part with its status; the stream then ends with an Error finish.
#[tokio::test]
async fn error_chunk_is_an_error_part_with_status() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(r#"{"error":{"message":"Rate limit reached","type":"rate_limit_error"}}"#),
    ]);
    mock_sse(&server, body).await;
    let model = make_provider(&server);

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();
    let parts = collect_stream(result).await;

    let error = parts
        .iter()
        .find_map(|p| match p {
            StreamPart::Error { error } => Some(error),
            _ => None,
        })
        .expect("an error part");
    match error {
        aimux_core::AiMuxError::ApiCall(call) => {
            assert_eq!(call.message, "Rate limit reached");
            assert_eq!(call.status_code, Some(429));
            assert!(error.is_retryable());
        }
        other => panic!("expected ApiCall, got {other:?}"),
    }
    match parts.last().unwrap() {
        StreamPart::Finish { finish_reason, .. } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
        }
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// Usage streamed in x_groq.usage is reported on the finish part, raw object included.
#[tokio::test]
async fn streams_usage_from_x_groq() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-1","model":"gemma2-9b-it","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"x_groq":{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let model = make_provider(&server);

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();
    let parts = collect_stream(result).await;

    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { usage, .. } => Some(usage.clone()),
        _ => None,
    });
    let usage = finish.expect("should have finish");
    assert_eq!(usage.input_tokens.total, Some(10));
    assert_eq!(usage.output_tokens.total, Some(5));
    // RFC-0016 M10: the raw object from the x_groq.usage sub-path is
    // preserved verbatim (vendor fields included).
    let raw = usage.raw.as_ref().expect("usage.raw must be populated");
    assert_eq!(raw["prompt_tokens"], json!(10));
    assert_eq!(raw["total_tokens"], json!(15));
}

/// A 401 response maps to an auth error.
#[tokio::test]
async fn auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error": {"message": "Invalid API key"}})),
        )
        .mount(&server)
        .await;

    let model = make_provider(&server);

    let result = model.do_generate(&default_options(test_prompt())).await;
    assert!(result.is_err());
    match result {
        Err(ref e) if e.status_code() == Some(401) => {}
        Err(e) => panic!("expected a 401, got {e:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

/// The request should carry the Authorization: Bearer header.
#[tokio::test]
async fn authorization_header() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let model = make_provider(&server);

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let auth = requests[0]
        .headers
        .get("authorization")
        .expect("missing authorization header");
    assert_eq!(auth, "Bearer test-api-key");
}

#[test]
fn every_entry_point_reports_groq_chat() {
    assert_eq!(
        groq().chat("llama-3.3-70b-versatile").provider(),
        "groq.chat"
    );
    assert_eq!(groq().call("m").provider(), "groq.chat");
    assert_eq!(groq().language_model("m").unwrap().provider(), "groq.chat");
    // The registry entry point and the preset build the same identity.
    let model =
        aimux_providers::create_provider("groq", aimux_providers::PresetSettings::default())
            .unwrap()
            .language_model("m")
            .unwrap();
    assert_eq!(model.provider(), "groq.chat");
}

#[test]
fn embedding_and_image_models_are_no_such_model() {
    let provider = groq();
    for result in [
        provider.embedding_model("e").map(|_| ()),
        provider.image_model("i").map(|_| ()),
    ] {
        assert!(matches!(result, Err(AiMuxError::NoSuchModel { .. })));
    }
}

#[tokio::test]
async fn only_the_groq_namespace_is_read() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;
    let model = groq_at(&server).chat("gemma2-9b-it");

    let result = model
        .do_generate(&options_with(json!({
            "groq": {"user": "u1", "reasoningFormat": "parsed", "serviceTier": "flex",
                     "parallelToolCalls": false},
            "openai": {"user": "ignored"},
            "openaiCompatible": {"reasoningEffort": "low"},
        })))
        .await
        .unwrap();

    let body = result.request.and_then(|r| r.body).unwrap();
    assert_eq!(body["user"], "u1");
    assert_eq!(body["reasoning_format"], "parsed");
    assert_eq!(body["service_tier"], "flex");
    assert_eq!(body["parallel_tool_calls"], json!(false));
    // upstream: parseProviderOptions({ provider: 'groq' }) reads that namespace only
    assert!(body.get("reasoning_effort").is_none());
}

#[tokio::test]
async fn streaming_asks_for_no_stream_options_and_reports_x_groq_usage() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"c","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"x_groq":{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cached_tokens":4}}}}"#,
        ),
    ]);
    mock_sse(&server, body).await;
    let model = groq_at(&server).chat("m");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();
    assert!(
        result
            .request
            .as_ref()
            .and_then(|r| r.body.as_ref())
            .unwrap()
            .get("stream_options")
            .is_none()
    );
    let parts = collect_stream(result).await;

    let (usage, metadata) = parts
        .iter()
        .find_map(|p| match p {
            StreamPart::Finish {
                usage,
                provider_metadata,
                ..
            } => Some((usage.clone(), provider_metadata.clone())),
            _ => None,
        })
        .expect("a finish part");
    assert_eq!(usage.input_tokens.total, Some(10));
    assert_eq!(usage.input_tokens.cache_read, Some(4));
    assert_eq!(usage.input_tokens.no_cache, Some(6));
    assert_eq!(usage.output_tokens.total, Some(5));
    // upstream: the finish part carries no providerMetadata
    assert!(metadata.is_none());
}

#[tokio::test]
async fn the_token_limit_is_max_tokens() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;
    let model = groq_at(&server).chat("gemma2-9b-it");
    let mut options = default_options(test_prompt());
    options.max_output_tokens = Some(64);
    options.top_k = Some(40.0);
    let result = model.do_generate(&options).await.unwrap();
    let body = result.request.and_then(|r| r.body).unwrap();
    // upstream: getArgs sends `max_tokens: maxOutputTokens`
    assert_eq!(body["max_tokens"], 64);
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("top_k").is_none(), "Groq has no top_k");
    assert!(
        result
            .warnings
            .iter()
            .any(|w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. } if feature == "topK"))
    );
}

#[serial]
#[tokio::test]
async fn the_key_is_read_from_the_environment_per_request_not_at_creation() {
    let saved = std::env::var("GROQ_API_KEY").ok();
    unsafe { std::env::remove_var("GROQ_API_KEY") };
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;
    let provider = create_groq(GroqProviderSettings {
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .expect("no key is read at creation");
    let model = provider.chat("m");

    let error = model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AiMuxError::LoadApiKey { env_var, .. } if env_var == "GROQ_API_KEY"),
        "{error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());

    unsafe { std::env::set_var("GROQ_API_KEY", "env-key") };
    model
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer env-key"
    );
    unsafe {
        match saved {
            Some(v) => std::env::set_var("GROQ_API_KEY", v),
            None => std::env::remove_var("GROQ_API_KEY"),
        }
    }
}

/// aimux's own: `usage.raw` keeps a `null` usage field as `null`, as
/// `convertGroqUsage` returns the parsed usage object (convert-groq-usage.ts:53);
/// keys outside the usage schema are dropped (groq-chat-language-model.ts:564-580).
#[tokio::test]
async fn usage_raw_keeps_null_fields() {
    let server = MockServer::start().await;
    let mut body = groq_text_body();
    body["usage"] = json!({
        "prompt_tokens": 45,
        "completion_tokens": 607,
        "total_tokens": 652,
        "prompt_tokens_details": { "cached_tokens": null },
        "completion_tokens_details": null,
        "queue_time": 0.01
    });
    mock_json(&server, body).await;

    let result = make_provider(&server)
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(
        result.usage.raw.map(Value::Object),
        Some(json!({
            "prompt_tokens": 45,
            "completion_tokens": 607,
            "total_tokens": 652,
            "prompt_tokens_details": { "cached_tokens": null },
            "completion_tokens_details": null
        }))
    );
}
