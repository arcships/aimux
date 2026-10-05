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

mod common;

use aimux_core::tool::RawToolCall;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, Tool};
use aimux_core::result::{GenerateContent, ReasoningOutput};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};

use aimux_provider_utils::Resolvable;
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
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        ..Default::default()
    })
    .expect("groq provider should build")
    .chat(model_id)
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

/// The groq-text fixture (inline).
fn groq_text_body() -> Value {
    json!({
        "id": "chatcmpl-09d64d2a-ed1c-4473-829f-78db43f45d13",
        "object": "chat.completion",
        "created": 1770770798,
        "model": "llama-3.3-70b-versatile",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Luminaria holiday response text." },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 45,
            "completion_tokens": 607,
            "total_tokens": 652
        }
    })
}

/// The groq-reasoning fixture (inline).
fn groq_reasoning_body() -> Value {
    json!({
        "id": "chatcmpl-73cf8a54-d54e-400c-88b8-603d1a346d96",
        "object": "chat.completion",
        "created": 1770770833,
        "model": "qwen/qwen3-32b",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "The word \"strawberry\" contains 3 R's.",
                "reasoning": "Okay, so the user is asking how many times the letter r appears in the word strawberry."
            },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 17,
            "completion_tokens": 649,
            "total_tokens": 666,
            "completion_tokens_details": {
                "reasoning_tokens": 570
            }
        }
    })
}

/// The groq-tool-call fixture (inline).
fn groq_tool_call_body() -> Value {
    json!({
        "id": "chatcmpl-1fd017fc-60b8-44eb-a736-375b8e1bc3e7",
        "object": "chat.completion",
        "created": 1770770815,
        "model": "llama-3.3-70b-versatile",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "ax9fskhev",
                    "type": "function",
                    "function": { "name": "weather", "arguments": "{}" }
                }]
            },
            "logprobs": null,
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 218,
            "completion_tokens": 15,
            "total_tokens": 233
        }
    })
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

// ════════════════════════════════════════════════════════════════════════════
// convert-to-groq-chat-messages.test.ts
// ════════════════════════════════════════════════════════════════════════════

mod convert_messages {
    use super::*;

    // ── user messages ──

    /// TS: "should convert messages with image parts"
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
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "user");
        assert_eq!(msg["content"][0]["type"], "text");
        assert_eq!(msg["content"][0]["text"], "Hello");
        assert_eq!(msg["content"][1]["type"], "image_url");
        assert_eq!(
            msg["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAECAw=="
        );
    }

    /// TS: "should convert messages with only a text part to a string content"
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
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello");
    }

    // ── tool calls ──

    /// TS: "should stringify arguments to tool calls"
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
                    thought_signature: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "quux".into(),
                    result: json!({"oof":"321rab"}),
                    tool_name: None,
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
        ];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        // Assistant message
        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][0]["content"], "");
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "quux");
        assert_eq!(body["messages"][0]["tool_calls"][0]["type"], "function");
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["name"],
            "thwomp"
        );
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["arguments"],
            r#"{"foo":"bar123"}"#
        );
        // Tool message
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["tool_call_id"], "quux");
        assert_eq!(body["messages"][1]["content"], r#"{"oof":"321rab"}"#);
    }

    /// TS: "should send reasoning if present"
    #[tokio::test]
    async fn reasoning_in_assistant_message() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Assistant {
            content: vec![
                AssistantPart::Reasoning(ReasoningPart {
                    text: "I think the tool will return the correct value.".into(),
                    signature: None,
                    provider_options: None,
                }),
                AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "quux".into(),
                    tool_name: "thwomp".into(),
                    input: json!({"foo":"bar123"}),
                    provider_executed: None,
                    thought_signature: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "");
        assert_eq!(
            msg["reasoning"],
            "I think the tool will return the correct value."
        );
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "thwomp");
    }

    /// TS: "should not include reasoning field when no reasoning content is present"
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
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "Hello, how can I help you?");
        assert!(msg.get("reasoning").is_none() || msg["reasoning"].is_null());
    }

    /// TS: "should throw for file parts with provider references"
    /// Groq does not support provider file references.
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

// ════════════════════════════════════════════════════════════════════════════
// convert-groq-usage.test.ts
// ════════════════════════════════════════════════════════════════════════════

mod convert_usage {
    use super::*;

    /// TS: "should convert basic usage without token details"
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

        assert_eq!(result.usage.input_tokens.total, Some(20));
        assert_eq!(result.usage.input_tokens.no_cache, Some(20));
        assert_eq!(result.usage.output_tokens.total, Some(10));
        assert_eq!(result.usage.output_tokens.text, Some(10));
    }

    /// TS: "should extract reasoning tokens from completion_tokens_details"
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

        assert_eq!(result.usage.input_tokens.total, Some(79));
        assert_eq!(result.usage.output_tokens.total, Some(40));
        assert_eq!(result.usage.output_tokens.reasoning, Some(21));
        assert_eq!(result.usage.output_tokens.text, Some(19)); // 40 - 21
    }

    /// TS: "should handle zero reasoning tokens"
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

        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(10));
    }

    /// TS: "should handle all tokens being reasoning tokens"
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

        assert_eq!(result.usage.output_tokens.total, Some(50));
        assert_eq!(result.usage.output_tokens.text, Some(0)); // 50 - 50
        assert_eq!(result.usage.output_tokens.reasoning, Some(50));
    }

    /// TS: "should map cached_tokens to cacheRead and subtract from noCache"
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

        assert_eq!(result.usage.input_tokens.total, Some(4641));
        assert_eq!(result.usage.input_tokens.no_cache, Some(33)); // 4641 - 4608
        assert_eq!(result.usage.input_tokens.cache_read, Some(4608));
    }

    /// TS: "should treat zero cached_tokens as a cache miss (cacheRead 0)"
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

        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.input_tokens.no_cache, Some(20));
    }

    /// TS: "should handle missing prompt_tokens and completion_tokens"
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

        assert_eq!(result.usage.input_tokens.total, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(0));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// groq-prepare-tools.test.ts
// ════════════════════════════════════════════════════════════════════════════

mod prepare_tools {
    use super::*;

    /// TS: "should correctly prepare function tools"
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
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "testFunction");
        assert_eq!(
            body["tools"][0]["function"]["description"],
            "A test function"
        );
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
    }

    /// TS: "should pass through strict mode when strict is true"
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
        assert_eq!(body["tools"][0]["function"]["strict"], true);
    }

    /// TS: "should pass through strict mode when strict is false"
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
        assert_eq!(body["tools"][0]["function"]["strict"], false);
    }

    /// TS: "should not include strict when strict is undefined"
    #[tokio::test]
    async fn strict_mode_undefined() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let tool = FunctionTool::new("testFunction", json!({"type": "object", "properties": {}}))
            .with_description("A test function");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert!(body["tools"][0]["function"].get("strict").is_none());
    }
}

// ════════════════════════════════════════════════════════════════════════════
// groq-chat-language-model.test.ts — doGenerate
// ════════════════════════════════════════════════════════════════════════════

mod do_generate {
    use super::*;

    /// upstream: "should reject a response without choices"
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
            matches!(&error, aimux_core::AiMuxError::InvalidResponseData(m) if m == "Response did not contain any choices."),
            "{error:?}"
        );
    }

    /// TS: "should extract text content"
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

    /// TS: "should send correct request body"
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
        assert_eq!(body["model"], "gemma2-9b-it");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello");
    }

    /// TS: "should extract tool call content"
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

    /// TS: "should extract reasoning content"
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

    /// TS: "should map top-level reasoning to reasoning_effort"
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

    /// upstream: "should coerce top-level reasoning minimal to low"
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

    /// upstream: "should coerce top-level reasoning xhigh to high"
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

    /// upstream: "should map top-level reasoning none to reasoning_effort for Qwen 3.6"
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

    /// upstream: "should omit unsupported top-level reasoning none and warn"
    #[tokio::test]
    async fn reasoning_none_omitted_and_warns_for_other_models() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = model_at(&server, "openai/gpt-oss-120b");

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

    /// TS: "should extract usage"
    #[tokio::test]
    async fn extracts_usage() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(45));
        assert_eq!(result.usage.output_tokens.total, Some(607));
    }

    /// TS: "should support partial usage" (only prompt_tokens, no completion_tokens)
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

        assert_eq!(result.usage.input_tokens.total, Some(20));
        assert_eq!(result.usage.output_tokens.total, Some(0));
    }

    /// TS: "should extract cached input tokens"
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

        assert_eq!(result.usage.input_tokens.total, Some(20));
        assert_eq!(result.usage.input_tokens.cache_read, Some(15));
        assert_eq!(result.usage.input_tokens.no_cache, Some(5)); // 20 - 15
    }

    /// TS: "should extract reasoning tokens from completion_tokens_details"
    #[tokio::test]
    async fn reasoning_tokens_from_details() {
        let server = MockServer::start().await;
        mock_json(&server, groq_reasoning_body()).await;

        let model = make_provider(&server);

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(17));
        assert_eq!(result.usage.output_tokens.total, Some(649));
        assert_eq!(result.usage.output_tokens.reasoning, Some(570));
        assert_eq!(result.usage.output_tokens.text, Some(79)); // 649 - 570
    }

    /// TS: "should support unknown finish reason"
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

    /// TS: "should pass response format information as json_schema when
    /// structuredOutputs enabled by default"
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
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["name"], "test-name");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    }

    /// TS: "should send request body" (request.body)
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
        assert_eq!(request_body["model"], "gemma2-9b-it");
        assert_eq!(request_body["messages"][0]["content"], "Hello");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// groq-chat-language-model.test.ts — doStream
// ════════════════════════════════════════════════════════════════════════════

mod do_stream {
    use super::*;

    /// TS: "should stream text"
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

    /// TS: "should send correct streaming request body"
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
        assert_eq!(req_body["model"], "gemma2-9b-it");
        assert_eq!(req_body["stream"], true);
        // Groq should NOT send stream_options
        assert!(req_body.get("stream_options").is_none());
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

    /// TS: "should stream tool call deltas when tool call arguments are passed
    /// in the first chunk"
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

    /// TS: "should handle error stream parts"
    #[tokio::test]
    async fn handles_error_stream() {
        let server = MockServer::start().await;
        let body = sse_body(&[&sse_event(
            r#"{"error":{"message":"The server had an error processing your request. Sorry about that!","type":"invalid_request_error"}}"#,
        )]);
        mock_sse(&server, body).await;

        let model = make_provider(&server);

        let result = model.do_stream(&default_options(test_prompt())).await;
        // upstream emits an error part and finishes; this crate's convention
        // (as in the Mistral, Cohere and OpenAI-compatible models) is to reject
        // when the very first event is an error, inside Core's retry boundary.
        assert!(result.is_err());
    }

    /// upstream: "should handle error stream parts" (an error chunk after the
    /// first event, which stays in the stream)
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

    /// TS: "should stream tool call that is sent in one chunk"
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

    /// TS: "should stream usage from x_groq.usage"
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

    /// upstream: "should keep reasoning active when deltas include empty tool calls"
    #[tokio::test]
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
        let reasoning: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
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

    /// upstream: "should handle unparsable stream parts"
    #[tokio::test]
    async fn unparsable_chunk_is_an_error_part_and_finish_reason_error() {
        let server = MockServer::start().await;
        mock_sse(
            &server,
            "data: {unparsable}\n\ndata: [DONE]\n\n".to_string(),
        )
        .await;
        let model = make_provider(&server);

        let options = CallOptions {
            include_raw_chunks: Some(true),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        assert!(matches!(parts[0], StreamPart::StreamStart { .. }));
        assert!(matches!(&parts[1], StreamPart::Raw { raw_value } if raw_value.is_null()));
        assert!(matches!(parts[2], StreamPart::Error { .. }), "{parts:?}");
        match &parts[3] {
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
                assert_eq!(finish_reason.raw, None);
                assert_eq!(usage.input_tokens.total, None);
            }
            other => panic!("expected Finish, got {other:?}"),
        }
        assert_eq!(parts.len(), 4);
    }

    /// upstream: "should stream raw chunks when includeRawChunks is true"
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

// ════════════════════════════════════════════════════════════════════════════
// groq-chat-language-model.test.ts — auth and headers
// ════════════════════════════════════════════════════════════════════════════

mod auth {
    use super::*;

    /// TS: A 401 response maps to an auth error.
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
}

// ════════════════════════════════════════════════════════════════════════════
// The package: identity, namespace, metadata, usage, cassettes
// ════════════════════════════════════════════════════════════════════════════

mod package {
    use super::*;

    use serial_test::serial;

    use aimux_core::AiMuxError;
    use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
    use aimux_core::provider::Provider;

    fn groq_at(server: &MockServer) -> aimux_providers::groq::GroqProvider {
        create_groq(GroqProviderSettings {
            base_url: Some(server.uri()),
            api_key: Some(Resolvable::Value("test-api-key".to_string())),
            ..Default::default()
        })
        .unwrap()
    }

    fn options_with(provider_options: Value) -> CallOptions {
        let mut options = default_options(test_prompt());
        options.provider_options = Some(serde_json::from_value(provider_options).unwrap());
        options
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
    fn a_custom_name_changes_identity_only() {
        let provider = create_groq(GroqProviderSettings {
            name: Some("groq_eu".to_string()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(provider.chat("m").provider(), "groq_eu.chat");
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

        let body = result.request.and_then(|request| request.body).unwrap();
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
                .and_then(|request| request.body.as_ref())
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
        let body = result.request.and_then(|request| request.body).unwrap();
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

    /// The recorded Groq exchanges (`tests/cassettes/groq`) replayed through
    /// the package: generate and stream both parse, and usage is read.
    #[tokio::test]
    async fn recorded_groq_cassettes_replay_through_the_package() {
        let server = MockServer::start().await;
        let n = common::replay::mount_cassettes(&server, "tests/cassettes/groq").await;
        assert!(n > 0, "no groq cassettes");
        let model = create_groq(GroqProviderSettings {
            base_url: Some(format!("{}/openai/v1", server.uri())),
            api_key: Some(Resolvable::Value("test-key".to_string())),
            ..Default::default()
        })
        .unwrap()
        .chat("llama-3.3-70b-versatile");

        let result = generate_text(&model, "Hello", GenerateTextOptions::default())
            .await
            .expect("generate_text should succeed with cassette replay");
        assert!(!result.text.is_empty() || !result.tool_calls.is_empty());
        assert!(result.usage.input_tokens.total.is_some());

        let result = stream_text(&model, "Hello", GenerateTextOptions::default())
            .await
            .expect("stream_text should succeed");
        let mut stream = result.stream;
        let mut finished = false;
        while let Some(part) = stream.next().await {
            if let StreamPart::Finish { .. } = part.expect("stream part") {
                finished = true;
            }
        }
        assert!(finished, "stream should finish");
    }
}
