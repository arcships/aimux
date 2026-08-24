//! Rust translations of the AI SDK OpenAI Responses API tests.
//!
//! Sources (TS -> Rust):
//! - `packages/openai/src/responses/openai-responses-language-model.test.ts`
//!   `describe('doGenerate')` request-body + response-parsing cases
//! - `packages/openai/src/responses/openai-responses-language-model.test.ts`
//!   `describe('doStream')` streaming cases
//!
//! Covers the core paths:
//! 1. Request building: input array, instructions, store, previous_response_id,
//!    reasoning (effort/summary), response_format (json_schema/json_object)
//! 2. Stream parsing: response.created -> output_item.added -> output_text.delta
//!    -> output_text.done -> output_item.done -> response.completed
//! 3. function tool calls (function_call_arguments.delta/done)
//! 4. reasoning summary streaming
//!
//! Each test uses `wiremock` to spin up a mock HTTP server, configures a JSON
//! or SSE response, creates an `OpenAIResponsesModel` pointing at the mock,
//! calls `do_generate` / `do_stream`, and asserts on the result.

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, Tool, ToolChoice};
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::{OpenAIConfig, OpenAIProvider};

// -- helpers -----------------------------------------------------------------

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// Build `CallOptions` with everything unset except `prompt`.
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// A simple function tool named `weather`.
fn weather_tool() -> FunctionTool {
    FunctionTool {
        name: "weather".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false,
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

/// Standard mock for a JSON responses-api response.
async fn mock_json_response(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Standard mock for an SSE streaming response.
async fn mock_sse_response(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

/// Build an SSE event string from a JSON value.
fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

/// Concatenate SSE events and append `[DONE]`.
fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Collect all `StreamPart`s from a `StreamResult` into a `Vec`.
async fn collect_stream(result: StreamResult) -> Vec<StreamPart> {
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

/// Extract the first recorded request body as a JSON value.
async fn first_request_body(server: &MockServer) -> Value {
    let requests = server
        .received_requests()
        .await
        .expect("no requests received");
    serde_json::from_slice(&requests[0].body).expect("invalid JSON body")
}

/// A basic text response body (single message output item).
fn text_response_body() -> Value {
    json!({
        "id": "resp_67c97c0203188190a025beb4a75242bc",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "error": null,
        "incomplete_details": null,
        "model": "gpt-4o-2024-07-18",
        "output": [
            {
                "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": "answer text",
                        "annotations": []
                    }
                ]
            }
        ],
        "usage": {
            "input_tokens": 345,
            "input_tokens_details": {
                "cached_tokens": 234,
                "cache_write_tokens": 45
            },
            "output_tokens": 538,
            "output_tokens_details": {
                "reasoning_tokens": 123
            }
        },
        "reasoning": { "effort": null, "summary": null, "context": "current_turn" }
    })
}

// ============================================================================
// doGenerate -- request body construction
// (openai-responses-language-model.test.ts -> describe('doGenerate'))
// ============================================================================

mod do_generate_request {
    use super::*;

    // -- should send model id, settings, and input --

    /// TS: "should send model id, settings, and input"
    #[tokio::test]
    async fn should_send_model_id_settings_and_input() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let prompt = vec![
            LanguageModelPromptMessage {
                role: Role::System,
                content: vec![ContentPart::text("You are a helpful assistant.")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
        ];
        let options = CallOptions {
            temperature: Some(0.5),
            top_p: Some(0.3),
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "maxToolCalls": 10 }));
                m
            }),
            ..CallOptions::new(prompt)
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][0]["content"], "You are a helpful assistant.");
        assert_eq!(body["input"][1]["role"], "user");
        assert_eq!(body["input"][1]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][0]["text"], "Hello");
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["top_p"], 0.3);
        assert_eq!(body["max_tool_calls"], 10);
    }

    // -- should send response format json schema --

    /// TS: "should send response format json schema"
    #[tokio::test]
    async fn should_send_response_format_json_schema() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            response_format: Some(aimux_core::options::ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"],
                    "additionalProperties": false,
                })),
                name: Some("response".to_string()),
                description: Some("A response".to_string()),
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["text"]["format"]["strict"], true);
        assert_eq!(body["text"]["format"]["name"], "response");
        assert_eq!(body["text"]["format"]["description"], "A response");
        assert_eq!(body["text"]["format"]["schema"]["type"], "object");
    }

    // -- should send response format json object --

    /// TS: "should send response format json object"
    #[tokio::test]
    async fn should_send_response_format_json_object() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            response_format: Some(aimux_core::options::ResponseFormat::Json {
                schema: None,
                name: None,
                description: None,
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["text"]["format"]["type"], "json_object");
    }

    // -- should remove unsupported settings for o1 --

    /// TS: "should remove unsupported settings for o1"
    #[tokio::test]
    async fn should_remove_unsupported_settings_for_o1() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("o1");

        let prompt = vec![
            LanguageModelPromptMessage {
                role: Role::System,
                content: vec![ContentPart::text("You are a helpful assistant.")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
        ];
        let options = CallOptions {
            temperature: Some(0.5),
            top_p: Some(0.3),
            ..CallOptions::new(prompt)
        };

        let result = model.do_generate(&options).await.expect("should succeed");

        let body = first_request_body(&server).await;
        assert_eq!(body["model"], "o1");
        // system message uses "developer" role for reasoning models
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][0]["content"], "You are a helpful assistant.");
        assert_eq!(body["input"][1]["role"], "user");
        assert_eq!(body["input"][1]["content"][0]["type"], "input_text");
        // temperature and top_p should be absent
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());

        // warnings for temperature and topP
        assert_eq!(result.warnings.len(), 2);
    }

    // -- should send store = false for reasoning model --

    /// TS: "should send store = false provider option and opt into
    /// reasoning.encrypted_content for reasoning models"
    #[tokio::test]
    async fn should_send_store_false_reasoning_model() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-5-mini");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "store": false }));
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["model"], "gpt-5-mini");
        assert_eq!(body["store"], false);
        assert_eq!(body["include"][0], "reasoning.encrypted_content");
    }

    // -- should send store = false for non-reasoning model --

    /// TS: "should send store = false provider option and not opt into
    /// reasoning.encrypted_content for non-reasoning models"
    #[tokio::test]
    async fn should_send_store_false_non_reasoning_model() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "store": false }));
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["store"], false);
        assert!(body.get("include").is_none());
    }

    // -- should send store = true --

    /// TS: "should send store = true provider option without
    /// reasoning.encrypted_content"
    #[tokio::test]
    async fn should_send_store_true() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "store": true }));
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["store"], true);
        assert!(body.get("include").is_none());
    }

    // -- should send previous response id --

    /// TS: "should send previous response id provider option"
    #[tokio::test]
    async fn should_send_previous_response_id() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "openai".to_string(),
                    json!({ "previousResponseId": "resp_123" }),
                );
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["previous_response_id"], "resp_123");
    }

    // -- should warn when both conversation and previousResponseId --

    /// TS: "should warn when both conversation and previousResponseId are
    /// provided"
    #[tokio::test]
    async fn should_warn_when_conversation_and_previous_response_id() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "openai".to_string(),
                    json!({ "conversation": "conv_123", "previousResponseId": "resp_123" }),
                );
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");

        let body = first_request_body(&server).await;
        assert_eq!(body["conversation"], "conv_123");
        assert_eq!(body["previous_response_id"], "resp_123");

        assert_eq!(result.warnings.len(), 1);
        match &result.warnings[0] {
            aimux_core::types::Warning::Unsupported { feature, .. } => {
                assert_eq!(feature, "conversation");
            }
            other => panic!("expected Unsupported warning, got {other:?}"),
        }
    }

    // -- should send reasoningEffort and reasoningSummary --

    /// TS: "should send reasoningEffort and reasoningSummary provider options"
    #[tokio::test]
    async fn should_send_reasoning_effort_and_summary() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("o3-mini");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "openai".to_string(),
                    json!({ "reasoningEffort": "low", "reasoningSummary": "auto" }),
                );
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["model"], "o3-mini");
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(body["reasoning"]["summary"], "auto");
    }

    // -- should send reasoning with detailed summary when only effort is set --

    /// TS: "should send xhigh reasoningEffort for codex-max model" (verifies
    /// that `summary` defaults to "detailed" when effort is non-"none")
    #[tokio::test]
    async fn should_default_reasoning_summary_to_detailed() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-5.1-codex-max");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "reasoningEffort": "xhigh" }));
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["reasoning"]["effort"], "xhigh");
        assert_eq!(body["reasoning"]["summary"], "detailed");
    }

    // -- should send parallelToolCalls --

    /// TS: "should send parallelToolCalls provider option"
    #[tokio::test]
    async fn should_send_parallel_tool_calls() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "parallelToolCalls": false }));
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["parallel_tool_calls"], false);
    }

    // -- should send user --

    /// TS: "should send user provider option"
    #[tokio::test]
    async fn should_send_user_option() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "user": "user_123" }));
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["user"], "user_123");
    }

    // -- should send conversation --

    /// TS: "should send conversation provider option"
    #[tokio::test]
    async fn should_send_conversation() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "conversation": "conv_123" }));
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["conversation"], "conv_123");
    }

    // -- should send function tools --

    /// TS: tools are prepared into the Responses `tools` array.
    #[tokio::test]
    async fn should_send_function_tools() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            tools: Some(vec![Tool::from(weather_tool())]),
            tool_choice: ToolChoice::Auto,
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.is_empty());

        let body = first_request_body(&server).await;
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "weather");
        assert_eq!(body["tools"][0]["parameters"]["type"], "object");
    }

    // -- should warn about unsupported topK --

    /// TS: topK is unsupported for the Responses API.
    #[tokio::test]
    async fn should_warn_about_topk() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            top_k: Some(40.0),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");
        assert!(result.warnings.iter().any(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => feature == "topK",
            _ => false,
        }));
    }
}

// ============================================================================
// doGenerate -- response parsing
// ============================================================================

mod do_generate_response {
    use super::*;

    // -- should generate text --

    /// TS: "should generate text"
    #[tokio::test]
    async fn should_generate_text() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "answer text"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // -- should extract usage --

    /// TS: "should extract usage"
    #[tokio::test]
    async fn should_extract_usage() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        assert_eq!(result.usage.input_tokens.total, Some(345));
        assert_eq!(result.usage.input_tokens.cache_read, Some(234));
        assert_eq!(result.usage.input_tokens.cache_write, Some(45));
        assert_eq!(result.usage.input_tokens.no_cache, Some(66));
        assert_eq!(result.usage.output_tokens.total, Some(538));
        assert_eq!(result.usage.output_tokens.reasoning, Some(123));
        assert_eq!(result.usage.output_tokens.text, Some(415));
    }

    // -- should extract response id metadata --

    /// TS: "should extract response id metadata"
    #[tokio::test]
    async fn should_extract_response_id_metadata() {
        let server = MockServer::start().await;
        mock_json_response(&server, text_response_body()).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        assert_eq!(
            result.response.id.as_deref(),
            Some("resp_67c97c0203188190a025beb4a75242bc")
        );
        assert_eq!(
            result.response.model_id.as_deref(),
            Some("gpt-4o-2024-07-18")
        );

        // providerMetadata should contain responseId and reasoningContext.
        let pm = result
            .provider_metadata
            .as_ref()
            .and_then(|v| v.get("openai"))
            .expect("provider_metadata.openai should exist");
        assert_eq!(pm["responseId"], "resp_67c97c0203188190a025beb4a75242bc");
        assert_eq!(pm["reasoningContext"], "current_turn");
    }

    // -- should throw error when no output --

    /// TS: "should throw a descriptive error when the response has no output"
    #[tokio::test]
    async fn should_throw_when_no_output() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "resp_no_output",
                "object": "response",
                "created_at": 1741257730,
                "status": "incomplete",
                "error": null,
                "incomplete_details": { "reason": "content_filter" },
                "model": "gpt-4o-2024-07-18",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 0
                }
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let err = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect_err("should error");

        let msg = err.to_string();
        assert!(
            msg.contains("no output"),
            "error should mention 'no output', got: {msg}"
        );
        assert!(
            msg.contains("content_filter"),
            "error should mention 'content_filter', got: {msg}"
        );
    }

    // -- should generate tool-call (function_call) --

    /// TS: function_call output items map to tool-call content.
    #[tokio::test]
    async fn should_generate_tool_call() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "resp_fc",
                "object": "response",
                "created_at": 1741257730,
                "status": "completed",
                "error": null,
                "incomplete_details": null,
                "model": "gpt-4o-2024-07-18",
                "output": [
                    {
                        "type": "function_call",
                        "id": "fc_123",
                        "call_id": "call_abc",
                        "name": "weather",
                        "arguments": "{\"location\":\"San Francisco\"}"
                    }
                ],
                "usage": {
                    "input_tokens": 50,
                    "output_tokens": 10
                }
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            tools: Some(vec![Tool::from(weather_tool())]),
            tool_choice: ToolChoice::Auto,
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_generate(&options).await.expect("should succeed");

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                assert_eq!(tool_call_id, "call_abc");
                assert_eq!(tool_name, "weather");
                assert_eq!(
                    input,
                    &Value::String(r#"{"location":"San Francisco"}"#.into())
                );
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        // finish reason should be tool-calls (hasFunctionCall = true, no incomplete_details)
        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    }

    // -- should generate reasoning content --

    /// TS: reasoning output items map to reasoning content.
    #[tokio::test]
    async fn should_generate_reasoning() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            json!({
                "id": "resp_rs",
                "object": "response",
                "created_at": 1741257730,
                "status": "completed",
                "error": null,
                "incomplete_details": null,
                "model": "o3-mini",
                "output": [
                    {
                        "type": "reasoning",
                        "id": "rs_123",
                        "summary": [
                            { "type": "summary_text", "text": "thinking..." }
                        ]
                    },
                    {
                        "id": "msg_1",
                        "type": "message",
                        "status": "completed",
                        "role": "assistant",
                        "content": [
                            { "type": "output_text", "text": "answer", "annotations": [] }
                        ]
                    }
                ],
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 20,
                    "output_tokens_details": { "reasoning_tokens": 15 }
                }
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("o3-mini");

        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        assert_eq!(result.content.len(), 2);
        match &result.content[0] {
            GenerateContent::Reasoning { text, .. } => assert_eq!(text, "thinking..."),
            other => panic!("expected Reasoning, got {other:?}"),
        }
        match &result.content[1] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "answer"),
            other => panic!("expected Text, got {other:?}"),
        }
        assert_eq!(result.usage.output_tokens.reasoning, Some(15));
        assert_eq!(result.usage.output_tokens.text, Some(5));
    }
}

// ============================================================================
// doStream -- streaming
// (openai-responses-language-model.test.ts -> describe('doStream'))
// ============================================================================

mod do_stream {
    use super::*;

    // -- should stream text deltas --

    /// TS: "should stream text deltas"
    ///
    /// Verifies the main streaming path:
    /// response.created -> output_item.added (message) -> output_text.delta ->
    /// output_item.done (message) -> response.completed.
    #[tokio::test]
    async fn should_stream_text_deltas() {
        let server = MockServer::start().await;
        let chunks = sse_body(&[
            &sse_event(
                r#"{"type":"response.created","response":{"id":"resp_1","created_at":1741269019,"model":"gpt-4o-2024-07-18"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"Hello,"}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":" World!"}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[]}}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_1","created_at":1741269019,"model":"gpt-4o-2024-07-18","incomplete_details":null,"usage":{"input_tokens":543,"input_tokens_details":{"cached_tokens":234},"output_tokens":478,"output_tokens_details":{"reasoning_tokens":123}}}}"#,
            ),
        ]);
        mock_sse_response(&server, &chunks).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        // Expected sequence:
        // StreamStart, ResponseMetadata, TextStart, TextDelta("Hello,"),
        // TextDelta(" World!"), TextEnd, Finish
        assert_eq!(parts.len(), 7);

        assert!(matches!(
            &parts[0],
            StreamPart::StreamStart { warnings } if warnings.is_empty()
        ));

        match &parts[1] {
            StreamPart::ResponseMetadata { id, model_id, .. } => {
                assert_eq!(id.as_deref(), Some("resp_1"));
                assert_eq!(model_id.as_deref(), Some("gpt-4o-2024-07-18"));
            }
            other => panic!("expected ResponseMetadata, got {other:?}"),
        }

        match &parts[2] {
            StreamPart::TextStart { id, .. } => assert_eq!(id, "msg_1"),
            other => panic!("expected TextStart, got {other:?}"),
        }

        match &parts[3] {
            StreamPart::TextDelta { id, delta, .. } => {
                assert_eq!(id, "msg_1");
                assert_eq!(delta, "Hello,");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }

        match &parts[4] {
            StreamPart::TextDelta { id, delta, .. } => {
                assert_eq!(id, "msg_1");
                assert_eq!(delta, " World!");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }

        match &parts[5] {
            StreamPart::TextEnd { id, .. } => assert_eq!(id, "msg_1"),
            other => panic!("expected TextEnd, got {other:?}"),
        }

        match &parts[6] {
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
                assert_eq!(usage.input_tokens.total, Some(543));
                assert_eq!(usage.input_tokens.cache_read, Some(234));
                assert_eq!(usage.output_tokens.total, Some(478));
                assert_eq!(usage.output_tokens.reasoning, Some(123));
                assert_eq!(usage.output_tokens.text, Some(355));
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // -- should stream tool calls --

    /// TS: "should send streaming tool calls"
    ///
    /// Verifies the function-call streaming path:
    /// output_item.added (function_call) -> function_call_arguments.delta ->
    /// function_call_arguments.done -> output_item.done (function_call).
    #[tokio::test]
    async fn should_stream_tool_calls() {
        let server = MockServer::start().await;
        let chunks = sse_body(&[
            &sse_event(
                r#"{"type":"response.created","response":{"id":"resp_tc","created_at":1741362087,"model":"gpt-4o-2024-07-18"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_added","name":"weather","arguments":"","status":"completed"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"{\"location\":"}"#,
            ),
            &sse_event(
                r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"\"Rome\"}"}"#,
            ),
            &sse_event(
                r#"{"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":0,"arguments":"{\"location\":\"Rome\"}"}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_done","name":"weather","arguments":"{\"location\":\"Rome\"}","status":"completed"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_tc","created_at":1741362087,"model":"gpt-4o-2024-07-18","incomplete_details":null,"usage":{"input_tokens":0,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0}}}}"#,
            ),
        ]);
        mock_sse_response(&server, &chunks).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let options = CallOptions {
            tools: Some(vec![Tool::from(weather_tool())]),
            tool_choice: ToolChoice::Auto,
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_stream(&options).await.expect("should succeed");
        let parts = collect_stream(result).await;

        // StreamStart, ResponseMetadata, ToolInputStart, ToolInputDelta x2,
        // ToolInputEnd, ToolCall, Finish
        assert_eq!(parts.len(), 8);

        // ToolInputStart uses the call_id from the added item.
        match &parts[2] {
            StreamPart::ToolInputStart { id, tool_name, .. } => {
                assert_eq!(id, "call_added");
                assert_eq!(tool_name, "weather");
            }
            other => panic!("expected ToolInputStart, got {other:?}"),
        }

        // ToolInputDelta uses the ongoing tool call's id (from added item).
        match &parts[3] {
            StreamPart::ToolInputDelta { id, delta, .. } => {
                assert_eq!(id, "call_added");
                assert!(delta.contains("location"));
            }
            other => panic!("expected ToolInputDelta, got {other:?}"),
        }
        match &parts[4] {
            StreamPart::ToolInputDelta { id, delta, .. } => {
                assert_eq!(id, "call_added");
                assert!(delta.contains("Rome"));
            }
            other => panic!("expected ToolInputDelta, got {other:?}"),
        }

        // ToolInputEnd uses the call_id from the done item.
        match &parts[5] {
            StreamPart::ToolInputEnd { id, .. } => assert_eq!(id, "call_done"),
            other => panic!("expected ToolInputEnd, got {other:?}"),
        }

        // ToolCall uses the call_id and arguments from the done item.
        match &parts[6] {
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                assert_eq!(tool_call_id, "call_done");
                assert_eq!(tool_name, "weather");
                assert_eq!(input, &Value::String(r#"{"location":"Rome"}"#.into()));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }

        // Finish should have tool-calls finish reason.
        match &parts[7] {
            StreamPart::Finish { finish_reason, .. } => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::ToolCalls);
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // -- should stream reasoning summary --

    /// TS: reasoning summary streaming.
    ///
    /// Verifies the reasoning streaming path:
    /// output_item.added (reasoning) -> reasoning_summary_part.added ->
    /// reasoning_summary_text.delta -> reasoning_summary_part.done ->
    /// output_item.done (reasoning).
    #[tokio::test]
    async fn should_stream_reasoning_summary() {
        let server = MockServer::start().await;
        let chunks = sse_body(&[
            &sse_event(
                r#"{"type":"response.created","response":{"id":"resp_rs","created_at":1741269019,"model":"o3-mini-2025-01-31"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.reasoning_summary_part.added","item_id":"rs_1","summary_index":0}"#,
            ),
            &sse_event(
                r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"thinking through the steps"}"#,
            ),
            &sse_event(
                r#"{"type":"response.reasoning_summary_part.done","item_id":"rs_1","summary_index":0}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"thinking through the steps"}]}}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_rs","created_at":1741269019,"model":"o3-mini-2025-01-31","incomplete_details":null,"usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":20,"output_tokens_details":{"reasoning_tokens":20}}}}"#,
            ),
        ]);
        mock_sse_response(&server, &chunks).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("o3-mini");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        // StreamStart, ResponseMetadata, ReasoningStart, ReasoningDelta,
        // ReasoningEnd, Finish
        // (reasoning_summary_part.done with store=false defaults to
        // can-conclude; the ReasoningEnd is emitted at output_item.done)
        assert_eq!(parts.len(), 6);

        match &parts[2] {
            StreamPart::ReasoningStart { id, .. } => {
                assert_eq!(id, "rs_1:0");
            }
            other => panic!("expected ReasoningStart, got {other:?}"),
        }

        match &parts[3] {
            StreamPart::ReasoningDelta { id, delta, .. } => {
                assert_eq!(id, "rs_1:0");
                assert_eq!(delta, "thinking through the steps");
            }
            other => panic!("expected ReasoningDelta, got {other:?}"),
        }

        match &parts[4] {
            StreamPart::ReasoningEnd { id, .. } => assert_eq!(id, "rs_1:0"),
            other => panic!("expected ReasoningEnd, got {other:?}"),
        }

        match &parts[5] {
            StreamPart::Finish { finish_reason, .. } => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            }
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // -- should carry encrypted reasoning content in stream metadata --

    /// Regression: the streaming reducer captured `encrypted_content` into
    /// state and then discarded it (`let _ = encrypted;`) — every
    /// ReasoningEnd carried `provider_metadata: None`, so encrypted
    /// reasoning could not cross a turn with store=false.
    #[tokio::test]
    async fn should_stream_reasoning_encrypted_content_in_metadata() {
        let server = MockServer::start().await;
        let chunks = sse_body(&[
            &sse_event(
                r#"{"type":"response.created","response":{"id":"resp_enc","created_at":1741269019,"model":"o3-mini-2025-01-31"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","encrypted_content":"enc-blob-123"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"thinking"}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","encrypted_content":"enc-blob-123","summary":[{"type":"summary_text","text":"thinking"}]}}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_enc","created_at":1741269019,"model":"o3-mini-2025-01-31","incomplete_details":null,"usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":20,"output_tokens_details":{"reasoning_tokens":20}}}}"#,
            ),
        ]);
        mock_sse_response(&server, &chunks).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("o3-mini");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        let start = parts
            .iter()
            .find(|p| matches!(p, StreamPart::ReasoningStart { .. }))
            .expect("ReasoningStart");
        match start {
            StreamPart::ReasoningStart {
                provider_metadata: Some(m),
                ..
            } => {
                assert_eq!(m["openai"]["itemId"], json!("rs_1"));
                assert_eq!(
                    m["openai"]["reasoningEncryptedContent"],
                    json!("enc-blob-123")
                );
            }
            other => panic!("ReasoningStart must carry metadata, got {other:?}"),
        }

        let end = parts
            .iter()
            .find(|p| matches!(p, StreamPart::ReasoningEnd { .. }))
            .expect("ReasoningEnd");
        match end {
            StreamPart::ReasoningEnd {
                id,
                provider_metadata: Some(m),
            } => {
                assert_eq!(id, "rs_1:0");
                assert_eq!(m["openai"]["itemId"], json!("rs_1"));
                assert_eq!(
                    m["openai"]["reasoningEncryptedContent"],
                    json!("enc-blob-123"),
                    "encrypted_content must ride ReasoningEnd so consume() can round-trip it"
                );
            }
            other => panic!("ReasoningEnd must carry metadata, got {other:?}"),
        }
    }

    /// Same path with no summary text at all: the ReasoningEnd must still
    /// carry the metadata so `consume()` can keep the part in
    /// response_messages.
    #[tokio::test]
    async fn should_stream_reasoning_encrypted_content_without_summary() {
        let server = MockServer::start().await;
        let chunks = sse_body(&[
            &sse_event(
                r#"{"type":"response.created","response":{"id":"resp_enc2","created_at":1741269019,"model":"o3-mini-2025-01-31"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_9","type":"reasoning","encrypted_content":"enc-no-summary"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_9","type":"reasoning","encrypted_content":"enc-no-summary","summary":[]}}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_enc2","created_at":1741269019,"model":"o3-mini-2025-01-31","incomplete_details":null,"usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":20,"output_tokens_details":{"reasoning_tokens":20}}}}"#,
            ),
        ]);
        mock_sse_response(&server, &chunks).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("o3-mini");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        let end = parts
            .iter()
            .find(|p| matches!(p, StreamPart::ReasoningEnd { .. }))
            .expect("ReasoningEnd even without any summary text");
        match end {
            StreamPart::ReasoningEnd {
                provider_metadata: Some(m),
                ..
            } => {
                assert_eq!(m["openai"]["itemId"], json!("rs_9"));
                assert_eq!(
                    m["openai"]["reasoningEncryptedContent"],
                    json!("enc-no-summary")
                );
            }
            other => panic!("ReasoningEnd must carry metadata even without summary, got {other:?}"),
        }
    }

    // -- should send finish reason for incomplete response --

    /// TS: "should send finish reason for incomplete response"
    ///
    /// Verifies that `response.incomplete` with `incomplete_details.reason =
    /// "max_output_tokens"` maps to finish reason `length`.
    #[tokio::test]
    async fn should_send_finish_reason_for_incomplete() {
        let server = MockServer::start().await;
        let chunks = sse_body(&[
            &sse_event(
                r#"{"type":"response.created","response":{"id":"resp_inc","created_at":1741269019,"model":"gpt-4o-2024-07-18"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"Hello,"}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","status":"incomplete","role":"assistant","content":[]}}"#,
            ),
            &sse_event(
                r#"{"type":"response.incomplete","response":{"id":"resp_inc","created_at":1741347648,"model":"gpt-4o-2024-07-18","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":0,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0}}}}"#,
            ),
        ]);
        mock_sse_response(&server, &chunks).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }))
            .expect("should have a Finish part");
        match finish {
            StreamPart::Finish { finish_reason, .. } => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Length);
                assert_eq!(finish_reason.raw.as_deref(), Some("max_output_tokens"));
            }
            _ => unreachable!(),
        }
    }

    // -- should stream text with store=true (reasoning-end at summary_part.done) --

    /// Verifies that when `store=true`, reasoning summary parts are concluded
    /// immediately at `reasoning_summary_part.done` (not deferred to
    /// `output_item.done`).
    #[tokio::test]
    async fn should_conclude_reasoning_at_summary_done_when_store_true() {
        let server = MockServer::start().await;
        let chunks = sse_body(&[
            &sse_event(
                r#"{"type":"response.created","response":{"id":"resp_rs","created_at":1741269019,"model":"o3-mini-2025-01-31"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.reasoning_summary_part.added","item_id":"rs_1","summary_index":0}"#,
            ),
            &sse_event(
                r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"thinking"}"#,
            ),
            &sse_event(
                r#"{"type":"response.reasoning_summary_part.done","item_id":"rs_1","summary_index":0}"#,
            ),
            &sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"thinking"}]}}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_rs","created_at":1741269019,"model":"o3-mini-2025-01-31","incomplete_details":null,"usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":20,"output_tokens_details":{"reasoning_tokens":20}}}}"#,
            ),
        ]);
        mock_sse_response(&server, &chunks).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("o3-mini");

        let options = CallOptions {
            provider_options: Some({
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), json!({ "store": true }));
                m
            }),
            ..CallOptions::new(test_prompt())
        };

        let result = model.do_stream(&options).await.expect("should succeed");
        let parts = collect_stream(result).await;

        // With store=true, ReasoningEnd should be emitted at
        // reasoning_summary_part.done (before output_item.done), so no
        // duplicate ReasoningEnd at output_item.done.
        let reasoning_ends: Vec<&StreamPart> = parts
            .iter()
            .filter(|p| matches!(p, StreamPart::ReasoningEnd { .. }))
            .collect();
        assert_eq!(
            reasoning_ends.len(),
            1,
            "should have exactly one ReasoningEnd with store=true"
        );
    }

    // -- should emit provider metadata with responseId in finish --

    /// TS: finish event should carry providerMetadata.openai.responseId.
    #[tokio::test]
    async fn should_emit_response_id_in_finish() {
        let server = MockServer::start().await;
        let chunks = sse_body(&[
            &sse_event(
                r#"{"type":"response.created","response":{"id":"resp_meta","created_at":1741269019,"model":"gpt-4o-2024-07-18"}}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_meta","created_at":1741269019,"model":"gpt-4o-2024-07-18","incomplete_details":null,"usage":{"input_tokens":0,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0}}}}"#,
            ),
        ]);
        mock_sse_response(&server, &chunks).await;

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let model = OpenAIProvider::new(config).responses_model("gpt-4o");

        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }))
            .expect("should have a Finish part");
        match finish {
            StreamPart::Finish {
                provider_metadata, ..
            } => {
                let pm = provider_metadata
                    .as_ref()
                    .and_then(|v| v.get("openai"))
                    .expect("provider_metadata.openai should exist");
                assert_eq!(pm["responseId"], "resp_meta");
            }
            _ => unreachable!(),
        }
    }
}
