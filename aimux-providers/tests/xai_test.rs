//! Rust translations of the AI SDK xAI provider chat-level tests.
//!
//! Sources (TS → Rust):
//! - `packages/xai/src/supports-reasoning-effort.test.ts` → `supports_reasoning_effort` mod
//! - `packages/xai/src/convert-xai-chat-usage.test.ts` → `convert_usage` mod
//! - `packages/xai/src/xai-prepare-tools.test.ts` → `prepare_tools` mod
//! - `packages/xai/src/convert-to-xai-chat-messages.test.ts` → `convert_messages` mod
//! - `packages/xai/src/xai-error.test.ts` → `error_handling` mod
//! - `packages/xai/src/xai-chat-language-model.test.ts` → `do_generate` / `do_stream` / `reasoning` mods
//! - `packages/xai/src/xai-provider.test.ts` → `provider` mod
//!
//! Each test uses `wiremock` to spin up a mock HTTP server, configures a JSON
//! or SSE response, creates an `XaiModel` via `XAIProvider`, calls
//! `do_generate` / `do_stream`, and asserts on the result.

use aimux_core::tool::RawToolCall;
use std::collections::HashMap;

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ToolChoice};
use aimux_core::result::{GenerateContent, ReasoningOutput, Source};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::FinishReasonUnified;

use aimux_providers::xai::convert::supports_reasoning_effort;
use aimux_providers::{XAIConfig, XAIProvider};

// ── shared helpers ───────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> Vec<LanguageModelPromptMessage> {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// `CallOptions` with only `prompt` set.
fn default_options(prompt: Vec<LanguageModelPromptMessage>) -> CallOptions {
    CallOptions::new(prompt)
}

/// A standard non-streaming chat-completion JSON body returning "Hello".
fn text_completion_body() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}

/// The xai-text fixture response (with reasoning_content and detailed usage).
fn xai_text_fixture() -> Value {
    json!({
        "id": "2af5c888-e886-6dcb-7844-95f8fe010b00",
        "object": "chat.completion",
        "created": 1770774046,
        "model": "grok-3-mini",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello",
                "reasoning_content": "First, the user said: \"Say a single word.\" That's straightforward. They want me to respond with just one word.\n\nResponse: I'll go with \"Hello\" as it's a common greeting and keeps it simple.",
                "refusal": null
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 12,
            "completion_tokens": 1,
            "total_tokens": 241,
            "prompt_tokens_details": {
                "text_tokens": 12,
                "audio_tokens": 0,
                "image_tokens": 0,
                "cached_tokens": 2
            },
            "completion_tokens_details": {
                "reasoning_tokens": 228,
                "audio_tokens": 0,
                "accepted_prediction_tokens": 0,
                "rejected_prediction_tokens": 0
            }
        },
        "system_fingerprint": "fp_2a885414fb"
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
            Err(e) => panic!("stream error: {e:?}"),
        }
    }
    parts
}

/// Extract text deltas from a list of stream parts.
fn text_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

/// Extract reasoning deltas from a list of stream parts.
fn reasoning_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ReasoningDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

/// Build a provider pointed at the mock server.
fn make_provider(server: &MockServer) -> XAIProvider {
    let config = XAIConfig::new("test-api-key").with_base_url(server.uri());
    XAIProvider::new(config)
}

/// Provider options with a single xai key.
fn xai_options(key: &str, value: Value) -> Option<HashMap<String, Value>> {
    let mut m = HashMap::new();
    m.insert("xai".to_string(), json!({ key: value }));
    Some(m)
}

/// Build provider options from a JSON object.
fn xai_provider_options(opts: Value) -> Option<HashMap<String, Value>> {
    let mut m = HashMap::new();
    m.insert("xai".to_string(), opts);
    Some(m)
}

// ════════════════════════════════════════════════════════════════════════════
// supportsReasoningEffort — direct function tests
// (supports-reasoning-effort.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod supports_reasoning_effort_tests {
    use super::*;

    /// TS: should return true for grok-4.3
    #[test]
    fn true_for_grok_4_3() {
        assert!(supports_reasoning_effort("grok-4.3"));
    }

    /// TS: should return true for grok-latest
    #[test]
    fn true_for_grok_latest() {
        assert!(supports_reasoning_effort("grok-latest"));
    }

    /// TS: should return true for grok-4.20-multi-agent
    #[test]
    fn true_for_grok_4_20_multi_agent() {
        assert!(supports_reasoning_effort("grok-4.20-multi-agent"));
    }

    /// TS: should return true for grok-4.20-multi-agent-0309
    #[test]
    fn true_for_grok_4_20_multi_agent_0309() {
        assert!(supports_reasoning_effort("grok-4.20-multi-agent-0309"));
    }

    /// TS: should return true for grok-3-mini
    #[test]
    fn true_for_grok_3_mini() {
        assert!(supports_reasoning_effort("grok-3-mini"));
    }

    /// TS: should return false for grok-4.20-reasoning
    #[test]
    fn false_for_grok_4_20_reasoning() {
        assert!(!supports_reasoning_effort("grok-4.20-reasoning"));
    }

    /// TS: should return false for grok-4.20-non-reasoning
    #[test]
    fn false_for_grok_4_20_non_reasoning() {
        assert!(!supports_reasoning_effort("grok-4.20-non-reasoning"));
    }

    /// TS: should return false for grok-4.20-0309-reasoning
    #[test]
    fn false_for_grok_4_20_0309_reasoning() {
        assert!(!supports_reasoning_effort("grok-4.20-0309-reasoning"));
    }

    /// TS: should return false for grok-4.20-0309-non-reasoning
    #[test]
    fn false_for_grok_4_20_0309_non_reasoning() {
        assert!(!supports_reasoning_effort("grok-4.20-0309-non-reasoning"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convertXaiChatUsage — tested through do_generate
// (convert-xai-chat-usage.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod convert_usage {
    use super::*;

    /// TS: should convert basic usage without reasoning tokens
    #[tokio::test]
    async fn basic_usage_without_reasoning() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hi" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 1,
                    "total_tokens": 13
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(12));
        assert_eq!(result.usage.input_tokens.no_cache, Some(12));
        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(1));
        assert_eq!(result.usage.output_tokens.text, Some(1));
        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
    }

    /// TS: should convert usage with reasoning tokens (xai reports separately)
    #[tokio::test]
    async fn usage_with_reasoning_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hi" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 1,
                    "total_tokens": 241,
                    "completion_tokens_details": {
                        "reasoning_tokens": 228
                    }
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        // xAI reports reasoning separately: total = completion + reasoning
        assert_eq!(result.usage.output_tokens.total, Some(229));
        assert_eq!(result.usage.output_tokens.text, Some(1));
        assert_eq!(result.usage.output_tokens.reasoning, Some(228));
        assert_eq!(result.usage.input_tokens.total, Some(12));
    }

    /// TS: should convert usage with cached input tokens
    #[tokio::test]
    async fn usage_with_cached_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hi" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 2,
                    "total_tokens": 438,
                    "prompt_tokens_details": {
                        "text_tokens": 12,
                        "audio_tokens": 0,
                        "image_tokens": 0,
                        "cached_tokens": 3
                    },
                    "completion_tokens_details": {
                        "reasoning_tokens": 424
                    }
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.cache_read, Some(3));
        assert_eq!(result.usage.input_tokens.no_cache, Some(9)); // 12 - 3
        assert_eq!(result.usage.input_tokens.total, Some(12));
        assert_eq!(result.usage.output_tokens.reasoning, Some(424));
        assert_eq!(result.usage.output_tokens.text, Some(2));
        assert_eq!(result.usage.output_tokens.total, Some(426)); // 2 + 424
    }

    /// TS: should handle cached_tokens exceeding prompt_tokens (non-inclusive)
    #[tokio::test]
    async fn cached_tokens_exceeding_prompt_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hi" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 4142,
                    "completion_tokens": 254,
                    "total_tokens": 8724,
                    "prompt_tokens_details": {
                        "cached_tokens": 4328
                    }
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        // Non-inclusive: total = prompt + cached, noCache = prompt
        assert_eq!(result.usage.input_tokens.cache_read, Some(4328));
        assert_eq!(result.usage.input_tokens.no_cache, Some(4142));
        assert_eq!(result.usage.input_tokens.total, Some(8470)); // 4142 + 4328
    }

    /// TS: should handle null token details
    #[tokio::test]
    async fn null_token_details() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hi" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 200,
                    "total_tokens": 300,
                    "prompt_tokens_details": null,
                    "completion_tokens_details": null
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.input_tokens.no_cache, Some(100));
        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(200));
        assert_eq!(result.usage.output_tokens.total, Some(200));
    }

    /// TS: should handle zero reasoning tokens
    #[tokio::test]
    async fn zero_reasoning_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hi" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 50,
                    "completion_tokens": 100,
                    "total_tokens": 150,
                    "completion_tokens_details": {
                        "reasoning_tokens": 0
                    }
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(100));
        assert_eq!(result.usage.output_tokens.total, Some(100));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// prepareTools — tested through do_generate request body and warnings
// (xai-prepare-tools.test.ts)
// ════════════════════════════════════════════════════════════════════════════

// ════════════════════════════════════════════════════════════════════════════
// convertToXaiChatMessages — tested through do_generate request body
// (convert-to-xai-chat-messages.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod convert_messages {
    use super::*;

    /// TS: should throw error when provider reference is missing xai key
    #[tokio::test]
    async fn missing_xai_reference_panics() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let prompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::file_reference(
                "image/png",
                json!({"openai": "file-xyz789"}),
            )],
            ..Default::default()
        }];
        let result = model.do_generate(&default_options(prompt)).await;
        assert!(
            result.is_err(),
            "should error when xai reference is missing"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — response parsing and request body
// (xai-chat-language-model.test.ts → describe('doGenerate'))
// ════════════════════════════════════════════════════════════════════════════

mod do_generate {
    use super::*;

    /// TS: should be instantiated correctly
    #[tokio::test]
    async fn instantiated_correctly() {
        let provider = make_provider(&MockServer::start().await);
        let model = provider.model("grok-3");
        assert_eq!(model.model_id(), "grok-3");
        assert_eq!(model.provider(), "xai.chat");
    }

    /// TS: should extract text content
    #[tokio::test]
    async fn extract_text_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hello from object" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "Hello from object"),
            other => panic!("expected Text, got {other:?}"),
        }
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    }

    /// TS: should extract tool call content
    #[tokio::test]
    async fn extract_tool_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_93562515",
                            "function": {
                                "name": "weather",
                                "arguments": "{\"location\":\"San Francisco\"}"
                            },
                            "type": "function"
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": { "prompt_tokens": 291, "total_tokens": 506, "completion_tokens": 26 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let tool_call = result.content.iter().find_map(|c| match c {
            GenerateContent::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        });
        let (id, name, input) = tool_call.expect("should have ToolCall");
        assert_eq!(id, "call_93562515");
        assert_eq!(name, "weather");
        assert_eq!(input, r#"{"location":"San Francisco"}"#);
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    }

    /// TS: should extract usage (from xai-text fixture)
    #[tokio::test]
    async fn extract_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(xai_text_fixture()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.cache_read, Some(2));
        assert_eq!(result.usage.input_tokens.no_cache, Some(10));
        assert_eq!(result.usage.input_tokens.total, Some(12));
        assert_eq!(result.usage.output_tokens.reasoning, Some(228));
        assert_eq!(result.usage.output_tokens.text, Some(1));
        assert_eq!(result.usage.output_tokens.total, Some(229));
    }

    /// TS: should pass the model and the messages
    #[tokio::test]
    async fn pass_model_and_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "grok-3");
        assert_eq!(body["messages"][0]["content"], "Hello");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    /// TS: should pass tools and toolChoice
    #[tokio::test]
    async fn pass_tools_and_tool_choice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let options = CallOptions {
            tools: Some(vec![Tool::from(FunctionTool {
                name: "test-tool".to_string(),
                description: None,
                input_schema: json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#"
                }),
                strict: None,
                provider_options: None,
                input_examples: None,
            })]),
            tool_choice: ToolChoice::Tool {
                tool_name: "test-tool".to_string(),
            },
            ..default_options(test_prompt())
        };
        let _ = model.do_generate(&options).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["tool_choice"]["function"]["name"], "test-tool");
        assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
        // additionalProperties should be removed
        assert!(
            body["tools"][0]["function"]["parameters"]
                .get("additionalProperties")
                .is_none()
        );
    }

    /// TS: should handle missing usage in response
    #[tokio::test]
    async fn missing_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "no-usage-test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hello", "tool_calls": null },
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(0));
        assert_eq!(result.usage.input_tokens.no_cache, Some(0));
        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(0));
        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
    }

    /// TS: should extract citations as sources
    #[tokio::test]
    async fn extract_citations() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "citations-test",
                "object": "chat.completion",
                "created": 1699472111,
                "model": "grok-3",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Here are the latest developments in AI.", "tool_calls": null },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
                "citations": [
                    "https://example.com/article1",
                    "https://example.com/article2"
                ]
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let options = CallOptions {
            provider_options: xai_provider_options(json!({
                "searchParameters": { "mode": "auto", "returnCitations": true }
            })),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let sources: Vec<_> = result
            .content
            .iter()
            .filter_map(|c| match c {
                GenerateContent::Source(Source {
                    url, source_type, ..
                }) => Some((url.clone(), source_type.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(sources.len(), 2);
        assert_eq!(
            sources[0].0.as_deref(),
            Some("https://example.com/article1")
        );
        assert_eq!(sources[0].1, "url");
        assert_eq!(
            sources[1].0.as_deref(),
            Some("https://example.com/article2")
        );
    }

    /// TS: should avoid duplication when there is a trailing assistant message
    #[tokio::test]
    async fn avoid_duplication_trailing_assistant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(xai_text_fixture()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let prompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::text("prefix ")],
                ..Default::default()
            },
        ];
        let result = model.do_generate(&default_options(prompt)).await.unwrap();

        // Should have text "Hello" (not "prefix ") and reasoning content
        let text = result.content.iter().find_map(|c| match c {
            GenerateContent::Text { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(text.as_deref(), Some("Hello"));

        let has_reasoning = result
            .content
            .iter()
            .any(|c| matches!(c, GenerateContent::Reasoning(_)));
        assert!(has_reasoning, "should have reasoning content");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — streaming response parsing
// (xai-chat-language-model.test.ts → describe('doStream'))
// ════════════════════════════════════════════════════════════════════════════

mod do_stream {
    use super::*;

    /// TS: should stream text content
    #[tokio::test]
    async fn stream_text() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"grok-3","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"grok-3","choices":[{"index":0,"delta":{"content":", World!"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"grok-3","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":30,"total_tokens":34}}"#,
            ),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        assert_eq!(
            text_deltas(&parts),
            vec!["Hello".to_string(), ", World!".to_string()]
        );

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { finish_reason, .. }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    /// TS: should stream tool call content
    #[tokio::test]
    async fn stream_tool_call() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"de9d896d","model":"grok-3-mini","choices":[{"index":0,"delta":{"tool_calls":[{"id":"call_55117580","function":{"name":"weather","arguments":"{\"location\":\"San Francisco\"}"}}]}}]}"#,
            ),
            &sse_event(
                r#"{"id":"de9d896d","model":"grok-3-mini","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            ),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let tool_call = parts.iter().find_map(|p| match p {
            StreamPart::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        });
        let (id, name, input) = tool_call.expect("should have ToolCall");
        assert_eq!(id, "call_55117580");
        assert_eq!(name, "weather");
        assert_eq!(input, r#"{"location":"San Francisco"}"#);
    }

    /// TS: should pass the messages (stream request body)
    #[tokio::test]
    async fn stream_request_body() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"grok-3","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-1","model":"grok-3","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":30,"total_tokens":34}}"#,
            ),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let _ = collect_stream(result).await;

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["model"], "grok-3");
        assert_eq!(body["messages"][0]["content"], "Hello");
    }

    /// TS: should handle missing usage in streaming response
    #[tokio::test]
    async fn stream_missing_usage() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"no-usage","model":"grok-3","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"no-usage","model":"grok-3","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"no-usage","model":"grok-3","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            ),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let finish = parts.iter().find_map(|p| match p {
            StreamPart::Finish { usage, .. } => Some(usage.clone()),
            _ => None,
        });
        let usage = finish.expect("should have Finish");
        assert_eq!(usage.input_tokens.total, Some(0));
        assert_eq!(usage.output_tokens.total, Some(0));
    }

    /// TS: should stream citations as sources
    #[tokio::test]
    async fn stream_citations() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"c8e45f92","model":"grok-3","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"c8e45f92","model":"grok-3","choices":[{"index":0,"delta":{"content":"Latest AI news"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"c8e45f92","model":"grok-3","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":30,"total_tokens":34},"citations":["https://example.com/source1","https://example.com/source2"]}"#,
            ),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let options = CallOptions {
            provider_options: xai_provider_options(json!({
                "searchParameters": { "mode": "auto", "returnCitations": true }
            })),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        let sources: Vec<_> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::Source(Source { url, .. }) => url.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0], "https://example.com/source1");
        assert_eq!(sources[1], "https://example.com/source2");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Reasoning models — reasoning_effort, reasoning content, streaming
// (xai-chat-language-model.test.ts → describe('reasoning models'))
// ════════════════════════════════════════════════════════════════════════════

mod reasoning {
    use super::*;

    /// TS: should extract reasoning content
    #[tokio::test]
    async fn extract_reasoning_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(xai_text_fixture()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3-mini");
        let options = CallOptions {
            provider_options: xai_options("reasoningEffort", json!("low")),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        // Should have text content and reasoning content
        let has_text = result
            .content
            .iter()
            .any(|c| matches!(c, GenerateContent::Text { text, .. } if text == "Hello"));
        assert!(has_text, "should have text 'Hello'");

        let reasoning = result.content.iter().find_map(|c| match c {
            GenerateContent::Reasoning(ReasoningOutput { text, .. }) => Some(text.clone()),
            _ => None,
        });
        assert!(reasoning.is_some(), "should have reasoning content");
        assert!(reasoning.unwrap().starts_with("First, the user said"));
    }

    /// TS: should extract reasoning tokens from usage
    #[tokio::test]
    async fn extract_reasoning_tokens_from_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(xai_text_fixture()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3-mini");
        let options = CallOptions {
            provider_options: xai_options("reasoningEffort", json!("high")),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.usage.output_tokens.reasoning, Some(228));
        assert_eq!(result.usage.output_tokens.text, Some(1));
        assert_eq!(result.usage.output_tokens.total, Some(229));
    }

    /// TS: should handle reasoning streaming
    #[tokio::test]
    async fn reasoning_streaming() {
        let server = MockServer::start().await;
        let chunks = [
            r#"{"id":"7327b9f5","model":"grok-3-mini","choices":[{"index":0,"delta":{"reasoning_content":"First","role":"assistant"}}]}"#,
            r#"{"id":"7327b9f5","model":"grok-3-mini","choices":[{"index":0,"delta":{"reasoning_content":","}}]}"#,
            r#"{"id":"7327b9f5","model":"grok-3-mini","choices":[{"index":0,"delta":{"reasoning_content":" the"}}]}"#,
            r#"{"id":"7327b9f5","model":"grok-3-mini","choices":[{"index":0,"delta":{"reasoning_content":" user"}}]}"#,
            r#"{"id":"7327b9f5","model":"grok-3-mini","choices":[{"index":0,"delta":{"reasoning_content":" said"}}]}"#,
            r#"{"id":"7327b9f5","model":"grok-3-mini","choices":[{"index":0,"delta":{"content":"Hello"}}]}"#,
            r#"{"id":"7327b9f5","model":"grok-3-mini","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        ];
        let body = sse_body(
            &chunks
                .iter()
                .map(|c| sse_event(c))
                .collect::<Vec<_>>()
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3-mini");
        let options = CallOptions {
            provider_options: xai_options("reasoningEffort", json!("low")),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        let deltas = reasoning_deltas(&parts);
        assert_eq!(deltas, vec!["First", ",", " the", " user", " said"]);

        // Should also have text delta
        let text = text_deltas(&parts);
        assert_eq!(text, vec!["Hello"]);
    }

    /// TS: should deduplicate repetitive reasoning deltas
    #[tokio::test]
    async fn dedup_reasoning_deltas() {
        let server = MockServer::start().await;
        let chunks = [
            r#"{"id":"grok-4-test","model":"grok-4-0709","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            r#"{"id":"grok-4-test","model":"grok-4-0709","choices":[{"index":0,"delta":{"reasoning_content":"Thinking... "},"finish_reason":null}]}"#,
            r#"{"id":"grok-4-test","model":"grok-4-0709","choices":[{"index":0,"delta":{"reasoning_content":"Thinking... "},"finish_reason":null}]}"#,
            r#"{"id":"grok-4-test","model":"grok-4-0709","choices":[{"index":0,"delta":{"reasoning_content":"Thinking... "},"finish_reason":null}]}"#,
            r#"{"id":"grok-4-test","model":"grok-4-0709","choices":[{"index":0,"delta":{"reasoning_content":"Actually calculating now..."},"finish_reason":null}]}"#,
            r#"{"id":"grok-4-test","model":"grok-4-0709","choices":[{"index":0,"delta":{"content":"The answer is 42."},"finish_reason":null}]}"#,
            r#"{"id":"grok-4-test","model":"grok-4-0709","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":15,"completion_tokens":20,"total_tokens":35,"completion_tokens_details":{"reasoning_tokens":10}}}"#,
        ];
        let body = sse_body(
            &chunks
                .iter()
                .map(|c| sse_event(c))
                .collect::<Vec<_>>()
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3-mini");
        let options = CallOptions {
            provider_options: xai_options("reasoningEffort", json!("low")),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        // "Thinking... " should only appear once (dedup)
        let deltas = reasoning_deltas(&parts);
        assert_eq!(deltas, vec!["Thinking... ", "Actually calculating now..."]);

        // Should have text delta
        let text = text_deltas(&parts);
        assert_eq!(text, vec!["The answer is 42."]);

        // Should have reasoning tokens in finish usage
        let finish = parts.iter().find_map(|p| match p {
            StreamPart::Finish { usage, .. } => Some(usage.clone()),
            _ => None,
        });
        let usage = finish.expect("should have Finish");
        assert_eq!(usage.output_tokens.reasoning, Some(10));
        assert_eq!(usage.output_tokens.text, Some(20));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Error handling — 200-status errors, chat completions error shape
// (xai-error.test.ts, xai-chat-language-model.test.ts → error handling)
// ════════════════════════════════════════════════════════════════════════════

mod error_handling {
    use super::*;

    /// TS: extracts message from chat completions error shape (400)
    #[tokio::test]
    async fn chat_completions_error_400() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": {
                    "message": "Invalid value: temperature must be between 0 and 2",
                    "type": "invalid_request_error",
                    "code": "invalid_value"
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model.do_generate(&default_options(test_prompt())).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Invalid value: temperature must be between 0 and 2"),
            "got: {msg}"
        );
    }

    /// TS: extracts message and code from responses api error shape
    #[tokio::test]
    async fn responses_error_shape() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "code": "Client specified an invalid argument",
                "error": "Invalid request content: Each message must have at least one content element."
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model.do_generate(&default_options(test_prompt())).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Client specified an invalid argument"),
            "got: {msg}"
        );
        assert!(msg.contains("Invalid request content"), "got: {msg}");
    }

    /// TS: should throw APICallError when xai returns error with 200 status (doGenerate)
    #[tokio::test]
    async fn error_with_200_status_do_generate() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": "The service is currently unavailable",
                "error": "Timed out waiting for first token"
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model.do_generate(&default_options(test_prompt())).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Timed out waiting for first token"),
            "got: {msg}"
        );
    }

    /// TS: should throw APICallError when xai returns error with 200 status (doStream)
    #[tokio::test]
    async fn error_with_200_status_do_stream() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": "The service is currently unavailable",
                "error": "Timed out waiting for first token"
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model.do_stream(&default_options(test_prompt())).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Timed out waiting for first token"),
            "got: {msg}"
        );
    }

    /// TS: 401 maps to `ApiCall` (401)
    #[tokio::test]
    async fn status_401_maps_to_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {
                    "message": "Incorrect API key provided",
                    "type": "invalid_request_error"
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let result = model.do_generate(&default_options(test_prompt())).await;

        assert!(
            matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.status_code == Some(401) && m.message == "Incorrect API key provided")
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Provider configuration — base URL, headers, auth
// (xai-provider.test.ts → chat-related tests)
// ════════════════════════════════════════════════════════════════════════════

mod provider {
    use super::*;

    /// TS: should construct a chat model with correct configuration
    #[tokio::test]
    async fn chat_model_config() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");

        assert_eq!(model.model_id(), "grok-3");
        assert_eq!(model.provider(), "xai.chat");
    }

    /// TS: should use custom baseURL
    #[tokio::test]
    async fn custom_base_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let config = XAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = XAIProvider::new(config);
        let model = provider.model("grok-3");
        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/chat/completions");
    }

    /// TS: should pass headers (provider + request)
    #[tokio::test]
    async fn pass_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let mut headers = HashMap::new();
        headers.insert(
            "Custom-Request-Header".to_string(),
            "request-header-value".to_string(),
        );
        let options = CallOptions {
            headers: Some(headers),
            ..default_options(test_prompt())
        };
        let _ = model.do_generate(&options).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer test-api-key")
        );
        assert_eq!(
            requests[0]
                .headers
                .get("custom-request-header")
                .and_then(|v| v.to_str().ok()),
            Some("request-header-value")
        );
    }

    /// TS: request uses correct URL and auth header
    #[tokio::test]
    async fn request_url_and_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-2");
        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/chat/completions");
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer test-api-key")
        );
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "grok-2");
    }

    /// TS: should warn for unsupported parameters (topK, frequencyPenalty, etc.)
    #[tokio::test]
    async fn warn_for_unsupported_params() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.model("grok-3");
        let options = CallOptions {
            top_k: Some(40.0),
            frequency_penalty: Some(0.5),
            presence_penalty: Some(0.3),
            stop_sequences: Some(vec!["stop".to_string()]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let features: Vec<_> = result
            .warnings
            .iter()
            .filter_map(|w| match w {
                aimux_core::types::Warning::Unsupported { feature, .. } => Some(feature.clone()),
                _ => None,
            })
            .collect();
        assert!(features.contains(&"topK".to_string()));
        assert!(features.contains(&"frequencyPenalty".to_string()));
        assert!(features.contains(&"presencePenalty".to_string()));
        assert!(features.contains(&"stopSequences".to_string()));
    }
}
