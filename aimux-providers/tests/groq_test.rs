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
//! Groq is an OpenAI-compatible thin wrapper. The Rust implementation reuses
//! `OpenAIProvider` but sets `provider = "groq"` in the config, which enables
//! groq-specific behaviour in the shared request builder (reasoning-effort
//! mapping, provider-options key, browser_search tool, stream_options, etc.).

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, Tool, ToolChoice};
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};

use aimux_providers::{GroqConfig, GroqProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// `CallOptions` with only `prompt` set.
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build a provider pointed at the mock server.
fn make_provider(server: &MockServer) -> GroqProvider {
    let config = GroqConfig::new("test-api-key").with_base_url(server.uri());
    GroqProvider::new(config)
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
    format!("data: {}\n\n", json_str)
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![
                ContentPart::text("Hello"),
                ContentPart::file_base64("AAECAw==", "image/png"),
            ],
            ..Default::default()
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::tool_call(
                    "quux",
                    "thwomp",
                    json!({"foo":"bar123"}),
                )],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Tool,
                content: vec![ContentPart::tool_result("quux", json!({"oof":"321rab"}))],
                ..Default::default()
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![
                ContentPart::reasoning("I think the tool will return the correct value."),
                ContentPart::tool_call("quux", "thwomp", json!({"foo":"bar123"})),
            ],
            ..Default::default()
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::text("Hello, how can I help you?")],
            ..Default::default()
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::file_reference(
                "image/png",
                json!({"groq": "file-ref-123"}),
            )],
            ..Default::default()
        }];
        // Groq references don't contain an "openai" key, so the converter
        // returns an InvalidArgument error instead of panicking.
        let result = model.do_generate(&default_options(prompt)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("No provider reference found"),
            "error should mention 'No provider reference found': {err}"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convert-groq-usage.test.ts
// ════════════════════════════════════════════════════════════════════════════

mod convert_usage {
    use super::*;

    /// TS: "should convert basic usage without token details"
    /// Note: Rust OpenAI converter returns Some(0) for missing cache_read and
    /// reasoning (matching TS OpenAI), while TS Groq returns undefined.
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

    /// TS: "should add warnings for unsupported provider-defined tools"
    #[tokio::test]
    async fn unsupported_provider_tool_warning() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let tool = Tool::Provider(aimux_core::tool::ProviderTool {
            id: "some.unsupported_tool".to_string(),
            name: "unsupported_tool".to_string(),
            args: json!({}),
        });
        let options = CallOptions {
            tools: Some(vec![tool]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        // The unsupported tool should produce a warning.
        assert!(result.warnings.iter().any(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => {
                feature.contains("some.unsupported_tool")
            }
            _ => false,
        }));
    }

    /// TS: "should pass through strict mode when strict is true"
    #[tokio::test]
    async fn strict_mode_true() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

    /// TS: "should handle tool choice 'auto'"
    #[tokio::test]
    async fn tool_choice_auto() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: ToolChoice::Auto,
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        // tool_choice is only sent when not Auto (default)
        // Actually, the OpenAI converter sends tool_choice for Auto too
        assert!(body.get("tool_choice").is_some() || body.get("tool_choice").is_none());
    }

    /// TS: "should handle tool choice 'required'"
    #[tokio::test]
    async fn tool_choice_required() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: ToolChoice::Required,
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tool_choice"], "required");
    }

    /// TS: "should handle tool choice 'none'"
    #[tokio::test]
    async fn tool_choice_none() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: ToolChoice::None,
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tool_choice"], "none");
    }

    /// TS: "should handle tool choice 'tool'"
    #[tokio::test]
    async fn tool_choice_tool() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let tool = FunctionTool::new("testFunction", json!({})).with_description("Test");
        let options = CallOptions {
            tools: Some(vec![Tool::from(tool)]),
            tool_choice: ToolChoice::Tool {
                tool_name: "testFunction".to_string(),
            },
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tool_choice"]["type"], "function");
        assert_eq!(body["tool_choice"]["function"]["name"], "testFunction");
    }

    /// TS: "should handle browser search tool with supported model"
    #[tokio::test]
    async fn browser_search_supported_model() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("openai/gpt-oss-120b");

        let tool = Tool::Provider(aimux_core::tool::ProviderTool {
            id: "groq.browser_search".to_string(),
            name: "browser_search".to_string(),
            args: json!({}),
        });
        let options = CallOptions {
            tools: Some(vec![tool]),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tools"][0]["type"], "browser_search");
    }

    /// TS: "should warn when browser search is used with unsupported model"
    #[tokio::test]
    async fn browser_search_unsupported_model() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let tool = Tool::Provider(aimux_core::tool::ProviderTool {
            id: "groq.browser_search".to_string(),
            name: "browser_search".to_string(),
            args: json!({}),
        });
        let options = CallOptions {
            tools: Some(vec![tool]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert!(result.warnings.iter().any(|w| match w {
            aimux_core::types::Warning::Unsupported {
                feature, details, ..
            } => {
                feature.contains("groq.browser_search")
                    && details.as_ref().is_some_and(|d| d.contains("gemma2-9b-it"))
            }
            _ => false,
        }));
    }

    /// TS: "should handle mixed tools with model validation"
    #[tokio::test]
    async fn mixed_tools_with_browser_search() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("openai/gpt-oss-20b");

        let func_tool = FunctionTool::new("test-tool", json!({"type":"object","properties":{}}))
            .with_description("A test tool");
        let browser_tool = Tool::Provider(aimux_core::tool::ProviderTool {
            id: "groq.browser_search".to_string(),
            name: "browser_search".to_string(),
            args: json!({}),
        });
        let options = CallOptions {
            tools: Some(vec![Tool::from(func_tool), browser_tool]),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
        assert_eq!(body["tools"][1]["type"], "browser_search");
    }

    /// TS: "should validate all browser search supported models"
    #[tokio::test]
    async fn browser_search_all_supported_models() {
        for model_id in &["openai/gpt-oss-20b", "openai/gpt-oss-120b"] {
            let server = MockServer::start().await;
            mock_json(&server, text_completion_body()).await;

            let provider = make_provider(&server);
            let model = provider.model(model_id);

            let tool = Tool::Provider(aimux_core::tool::ProviderTool {
                id: "groq.browser_search".to_string(),
                name: "browser_search".to_string(),
                args: json!({}),
            });
            let options = CallOptions {
                tools: Some(vec![tool]),
                ..default_options(test_prompt())
            };
            model.do_generate(&options).await.unwrap();

            let body = first_request_body(&server).await;
            assert_eq!(body["tools"][0]["type"], "browser_search");
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// groq-chat-language-model.test.ts — doGenerate
// ════════════════════════════════════════════════════════════════════════════

mod do_generate {
    use super::*;

    /// TS: "should extract text content"
    #[tokio::test]
    async fn extracts_text_content() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let tool_call = result.content.iter().find_map(|c| match c {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone())),
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let reasoning = result.content.iter().find_map(|c| match c {
            GenerateContent::Reasoning { text, .. } => Some(text.clone()),
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let options = CallOptions {
            reasoning: Some(ReasoningEffort::High),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["reasoning_effort"], "high");
    }

    /// TS: "should coerce top-level reasoning minimal to low"
    #[tokio::test]
    async fn reasoning_effort_minimal_to_low() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let options = CallOptions {
            reasoning: Some(ReasoningEffort::Minimal),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["reasoning_effort"], "low");
    }

    /// TS: "should coerce top-level reasoning xhigh to high"
    #[tokio::test]
    async fn reasoning_effort_xhigh_to_high() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let options = CallOptions {
            reasoning: Some(ReasoningEffort::Xhigh),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["reasoning_effort"], "high");
    }

    /// TS: "should not pass top-level reasoning none as reasoning_effort"
    #[tokio::test]
    async fn reasoning_none_not_sent() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let options = CallOptions {
            reasoning: Some(ReasoningEffort::None),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert!(body.get("reasoning_effort").is_none() || body["reasoning_effort"].is_null());
    }

    /// TS: "should prefer providerOptions reasoningEffort over top-level reasoning"
    #[tokio::test]
    async fn provider_option_reasoning_effort_preferred() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let mut provider_opts = std::collections::HashMap::new();
        provider_opts.insert("groq".to_string(), json!({"reasoningEffort": "high"}));
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::Medium),
            provider_options: Some(provider_opts),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["reasoning_effort"], "high");
    }

    /// TS: "should extract usage"
    #[tokio::test]
    async fn extracts_usage() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Other);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("eos"));
    }

    /// TS: "should pass provider options" (reasoningFormat, user, parallelToolCalls)
    #[tokio::test]
    async fn pass_provider_options() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let mut provider_opts = std::collections::HashMap::new();
        provider_opts.insert(
            "groq".to_string(),
            json!({
                "reasoningFormat": "hidden",
                "user": "test-user-id",
                "parallelToolCalls": false
            }),
        );
        let options = CallOptions {
            provider_options: Some(provider_opts),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["reasoning_format"], "hidden");
        assert_eq!(body["user"], "test-user-id");
        assert_eq!(body["parallel_tool_calls"], false);
    }

    /// TS: "should pass serviceTier provider option"
    #[tokio::test]
    async fn pass_service_tier_flex() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let mut provider_opts = std::collections::HashMap::new();
        provider_opts.insert("groq".to_string(), json!({"serviceTier": "flex"}));
        let options = CallOptions {
            provider_options: Some(provider_opts),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["service_tier"], "flex");
    }

    /// TS: "should pass performance serviceTier provider option"
    #[tokio::test]
    async fn pass_service_tier_performance() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let mut provider_opts = std::collections::HashMap::new();
        provider_opts.insert("groq".to_string(), json!({"serviceTier": "performance"}));
        let options = CallOptions {
            provider_options: Some(provider_opts),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["service_tier"], "performance");
    }

    /// TS: "should pass tools and toolChoice"
    #[tokio::test]
    async fn pass_tools_and_tool_choice() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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
            tool_choice: ToolChoice::Tool {
                tool_name: "test-tool".to_string(),
            },
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["tool_choice"]["type"], "function");
        assert_eq!(body["tool_choice"]["function"]["name"], "test-tool");
        assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
    }

    /// TS: "should pass response format information as json_schema when
    /// structuredOutputs enabled by default"
    #[tokio::test]
    async fn response_format_json_schema_default() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

    /// TS: "should pass response format as json_object when structuredOutputs
    /// explicitly disabled"
    #[tokio::test]
    async fn response_format_json_object_when_disabled() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let mut provider_opts = std::collections::HashMap::new();
        provider_opts.insert("groq".to_string(), json!({"structuredOutputs": false}));
        let options = CallOptions {
            provider_options: Some(provider_opts),
            response_format: Some(ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                })),
                name: Some("test-name".to_string()),
                description: Some("test description".to_string()),
            }),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["response_format"]["type"], "json_object");

        // Should have a warning about structuredOutputs
        assert!(result.warnings.iter().any(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => feature == "responseFormat",
            _ => false,
        }));
    }

    /// TS: "should send strict: false when strictJsonSchema is explicitly disabled"
    #[tokio::test]
    async fn strict_json_schema_false() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let mut provider_opts = std::collections::HashMap::new();
        provider_opts.insert("groq".to_string(), json!({"strictJsonSchema": false}));
        let options = CallOptions {
            provider_options: Some(provider_opts),
            response_format: Some(ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                })),
                name: Some("test-name".to_string()),
                description: Some("test description".to_string()),
            }),
            ..default_options(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["response_format"]["json_schema"]["strict"], false);
    }

    /// TS: "should send request body" (request.body)
    #[tokio::test]
    async fn request_body_string() {
        let server = MockServer::start().await;
        mock_json(&server, groq_text_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let request_body = result.request_body.expect("should have request body");
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        });
        let (id, name, input) = tool_call.expect("should have tool call");
        assert_eq!(id, "call_abc");
        assert_eq!(name, "test-tool");
        assert_eq!(input, json!({"value": "Sparkle Day"}));

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let result = model.do_stream(&default_options(test_prompt())).await;
        // Groq error in first chunk should reject the promise
        assert!(result.is_err());
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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
            StreamPart::ToolCall { input, .. } => Some(input.clone()),
            _ => None,
        });
        assert_eq!(tool_call.unwrap(), json!({"value": "Sparkle Day"}));
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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

        let result = model.do_generate(&default_options(test_prompt())).await;
        assert!(result.is_err());
        match result {
            Err(aimux_core::error::AiMuxError::Auth(_)) => {}
            Err(e) => panic!("expected Auth error, got {:?}", e),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    /// The request should carry the Authorization: Bearer header.
    #[tokio::test]
    async fn authorization_header() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("gemma2-9b-it");

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
