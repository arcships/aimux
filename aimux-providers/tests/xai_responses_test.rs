//! Rust translations of the AI SDK xAI Responses API tests.
//!
//! Sources (TS → Rust):
//! - `xai-responses-language-model.test.ts` → `do_generate` / `do_stream` mods
//! - `xai-responses-prepare-tools.test.ts` → `prepare_tools` mod
//! - `convert-xai-responses-usage.test.ts` → `convert_usage` mod
//! - `convert-to-xai-responses-input.test.ts` → `convert_input` mod
//!
//! Each test uses `wiremock` to spin up a mock HTTP server, configures a JSON
//! or SSE response, creates an `XaiResponsesModel` via `XAIProvider`, calls
//! `do_generate` / `do_stream`, and asserts on the result.

use std::collections::HashMap;

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, ProviderTool, Tool};
use aimux_core::types::ReasoningEffort;

use aimux_providers::{XAIConfig, XAIProvider};

// ── shared helpers ───────────────────────────────────────────────────────────

fn test_prompt() -> Vec<LanguageModelPromptMessage> {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("hello")],
        ..Default::default()
    }]
}

fn default_options(prompt: Vec<LanguageModelPromptMessage>) -> CallOptions {
    CallOptions::new(prompt)
}

fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

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

fn make_provider(server: &MockServer) -> XAIProvider {
    let config = XAIConfig::new("test-api-key").with_base_url(server.uri());
    XAIProvider::new(config)
}

fn xai_provider_options(opts: Value) -> Option<HashMap<String, Value>> {
    let mut m = HashMap::new();
    m.insert("xai".to_string(), opts);
    Some(m)
}

/// A standard Responses API JSON body returning a text message.
fn text_response_body() -> Value {
    json!({
        "id": "resp_123",
        "object": "response",
        "created_at": 1700000000,
        "status": "completed",
        "model": "grok-4-fast-non-reasoning",
        "output": [{
            "type": "message",
            "id": "msg_123",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": "hello world",
                "annotations": []
            }]
        }],
        "usage": { "input_tokens": 10, "output_tokens": 5, "total_tokens": 15 }
    })
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — basic text generation
// ════════════════════════════════════════════════════════════════════════════

mod do_generate {
    use super::*;

    /// TS: should generate text content
    #[tokio::test]
    async fn generate_text_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body()))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "hello world"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    /// TS: should extract usage correctly
    #[tokio::test]
    async fn extract_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [],
                "usage": {
                    "input_tokens": 345,
                    "output_tokens": 538,
                    "total_tokens": 883,
                    "output_tokens_details": { "reasoning_tokens": 123 }
                }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(345));
        assert_eq!(result.usage.input_tokens.no_cache, Some(345));
        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(538));
        assert_eq!(result.usage.output_tokens.text, Some(415)); // 538 - 123
        assert_eq!(result.usage.output_tokens.reasoning, Some(123));
    }

    /// TS: should expose cost_in_usd_ticks in providerMetadata
    #[tokio::test]
    async fn cost_in_usd_ticks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5, "cost_in_usd_ticks": 113500 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(
            result.provider_metadata,
            Some(json!({ "xai": { "costInUsdTicks": 113500 } }))
        );
    }

    /// TS: should not include providerMetadata when cost_in_usd_ticks is missing
    #[tokio::test]
    async fn no_cost_no_metadata() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert!(result.provider_metadata.is_none());
    }

    /// TS: should extract finish reason from status
    #[tokio::test]
    async fn finish_reason_from_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.finish_reason.raw.as_deref(), Some("completed"));
        assert_eq!(
            result.finish_reason.unified,
            aimux_core::types::FinishReasonUnified::Stop
        );
    }

    /// TS: should return tool-calls finish reason when function_call is present
    #[tokio::test]
    async fn tool_calls_finish_reason() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "function_call",
                    "id": "fc_123",
                    "name": "weather",
                    "arguments": "{\"location\":\"sf\"}",
                    "call_id": "call_123"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![
                FunctionTool::new(
                    "weather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } }
                    }),
                )
                .into(),
            ]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(
            result.finish_reason.unified,
            aimux_core::types::FinishReasonUnified::ToolCalls
        );
        assert_eq!(result.finish_reason.raw.as_deref(), Some("completed"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — reasoning content
// ════════════════════════════════════════════════════════════════════════════

mod reasoning {
    use super::*;

    /// TS: should extract reasoning with encrypted content when store=false
    #[tokio::test]
    async fn reasoning_with_encrypted_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [
                    {
                        "type": "reasoning",
                        "id": "rs_456",
                        "status": "completed",
                        "summary": [{ "type": "summary_text", "text": "First, analyze the question carefully." }],
                        "encrypted_content": "abc123encryptedcontent"
                    },
                    {
                        "type": "message",
                        "id": "msg_123",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": "The answer is 42.", "annotations": [] }]
                    }
                ],
                "usage": { "input_tokens": 10, "output_tokens": 20, "output_tokens_details": { "reasoning_tokens": 15 } }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.content.len(), 2);
        match &result.content[0] {
            GenerateContent::Reasoning {
                text,
                provider_metadata,
            } => {
                assert_eq!(text, "First, analyze the question carefully.");
                assert_eq!(
                    provider_metadata,
                    &Some(
                        json!({ "xai": { "itemId": "rs_456", "reasoningEncryptedContent": "abc123encryptedcontent" } })
                    )
                );
            }
            other => panic!("expected Reasoning, got {other:?}"),
        }
        match &result.content[1] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "The answer is 42."),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    /// TS: should handle reasoning without encrypted content
    #[tokio::test]
    async fn reasoning_without_encrypted_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [
                    {
                        "type": "reasoning",
                        "id": "rs_456",
                        "status": "completed",
                        "summary": [{ "type": "summary_text", "text": "Thinking through the problem." }]
                    },
                    {
                        "type": "message",
                        "id": "msg_123",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": "Solution found.", "annotations": [] }]
                    }
                ],
                "usage": { "input_tokens": 10, "output_tokens": 15 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        match &result.content[0] {
            GenerateContent::Reasoning {
                text,
                provider_metadata,
            } => {
                assert_eq!(text, "Thinking through the problem.");
                assert_eq!(
                    provider_metadata,
                    &Some(json!({ "xai": { "itemId": "rs_456" } }))
                );
            }
            other => panic!("expected Reasoning, got {other:?}"),
        }
    }

    /// TS: should extract reasoning from content when summary is empty
    #[tokio::test]
    async fn reasoning_from_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-3-mini",
                "output": [
                    {
                        "type": "reasoning",
                        "id": "rs_456",
                        "status": "completed",
                        "summary": [],
                        "content": [{ "type": "reasoning_text", "text": "Let me think step by step." }]
                    },
                    {
                        "type": "message",
                        "id": "msg_123",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": "The answer is 444.", "annotations": [] }]
                    }
                ],
                "usage": { "input_tokens": 10, "output_tokens": 15 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-3-mini");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        match &result.content[0] {
            GenerateContent::Reasoning { text, .. } => {
                assert_eq!(text, "Let me think step by step.");
            }
            other => panic!("expected Reasoning, got {other:?}"),
        }
    }

    /// TS: should extract reasoning with encrypted content but empty summary text
    #[tokio::test]
    async fn reasoning_empty_summary_with_encrypted() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [
                    {
                        "type": "reasoning",
                        "id": "rs_789",
                        "status": "completed",
                        "summary": [],
                        "encrypted_content": "encrypted_zdr_content_xyz"
                    },
                    {
                        "type": "message",
                        "id": "msg_123",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": "Here is my response.", "annotations": [] }]
                    }
                ],
                "usage": { "input_tokens": 10, "output_tokens": 20, "output_tokens_details": { "reasoning_tokens": 15 } }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        match &result.content[0] {
            GenerateContent::Reasoning {
                text,
                provider_metadata,
            } => {
                assert_eq!(text, "");
                assert_eq!(
                    provider_metadata,
                    &Some(
                        json!({ "xai": { "itemId": "rs_789", "reasoningEncryptedContent": "encrypted_zdr_content_xyz" } })
                    )
                );
            }
            other => panic!("expected Reasoning, got {other:?}"),
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — settings and provider options
// ════════════════════════════════════════════════════════════════════════════

mod settings {
    use super::*;

    /// TS: should send model id and settings
    #[tokio::test]
    async fn send_model_id_and_settings() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123",
                "object": "response",
                "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            prompt: vec![
                LanguageModelPromptMessage {
                    role: Role::System,
                    content: vec![ContentPart::text("you are helpful")],
                    ..Default::default()
                },
                LanguageModelPromptMessage {
                    role: Role::User,
                    content: vec![ContentPart::text("hello")],
                    ..Default::default()
                },
            ],
            temperature: Some(0.5),
            top_p: Some(0.9),
            max_output_tokens: Some(100),
            ..CallOptions::new(vec![])
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = result.request_body.unwrap();
        assert_eq!(body["model"], "grok-4-fast-non-reasoning");
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["max_output_tokens"], 100);
        assert_eq!(
            body["input"],
            json!([
                { "role": "system", "content": "you are helpful" },
                { "role": "user", "content": [{ "type": "input_text", "text": "hello" }] }
            ])
        );
    }

    /// TS: reasoningEffort provider option
    #[tokio::test]
    async fn reasoning_effort_option() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            provider_options: xai_provider_options(json!({ "reasoningEffort": "high" })),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.request_body.unwrap()["reasoning"]["effort"], "high");
    }

    /// TS: reasoningSummary provider option
    #[tokio::test]
    async fn reasoning_summary_option() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            provider_options: xai_provider_options(json!({ "reasoningSummary": "concise" })),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(
            result.request_body.unwrap()["reasoning"]["summary"],
            "concise"
        );
    }

    /// TS: reasoningEffort and reasoningSummary together
    #[tokio::test]
    async fn reasoning_effort_and_summary() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            provider_options: xai_provider_options(
                json!({ "reasoningEffort": "high", "reasoningSummary": "detailed" }),
            ),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(
            result.request_body.unwrap()["reasoning"],
            json!({ "effort": "high", "summary": "detailed" })
        );
    }

    /// TS: store:false
    #[tokio::test]
    async fn store_false() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            provider_options: xai_provider_options(json!({ "store": false })),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = result.request_body.unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    }

    /// TS: store:true (should not set store or include)
    #[tokio::test]
    async fn store_true() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            provider_options: xai_provider_options(json!({ "store": true })),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = result.request_body.unwrap();
        assert!(body.get("store").is_none());
        assert!(body.get("include").is_none());
    }

    /// TS: previousResponseId
    #[tokio::test]
    async fn previous_response_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            provider_options: xai_provider_options(json!({ "previousResponseId": "resp_456" })),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(
            result.request_body.unwrap()["previous_response_id"],
            "resp_456"
        );
    }

    /// TS: include with file_search_call.results
    #[tokio::test]
    async fn include_option() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            provider_options: xai_provider_options(
                json!({ "include": ["file_search_call.results"] }),
            ),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(
            result.request_body.unwrap()["include"],
            json!(["file_search_call.results"])
        );
    }

    /// TS: include with file_search_call.results and store:false
    #[tokio::test]
    async fn include_with_store_false() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            provider_options: xai_provider_options(
                json!({ "include": ["file_search_call.results"], "store": false }),
            ),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = result.request_body.unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(
            body["include"],
            json!(["file_search_call.results", "reasoning.encrypted_content"])
        );
    }

    /// TS: should map top-level reasoning to reasoning effort
    #[tokio::test]
    async fn top_level_reasoning() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4.3", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4.3");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::High),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.request_body.unwrap()["reasoning"]["effort"], "high");
    }

    /// TS: should map top-level reasoning none to "none"
    #[tokio::test]
    async fn top_level_reasoning_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4.3", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4.3");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::None),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.request_body.unwrap()["reasoning"]["effort"], "none");
    }

    /// TS: should prefer providerOptions reasoningEffort over top-level reasoning
    #[tokio::test]
    async fn prefer_provider_reasoning_effort() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4.3", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4.3");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::None),
            provider_options: xai_provider_options(json!({ "reasoningEffort": "high" })),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.request_body.unwrap()["reasoning"]["effort"], "high");
    }

    /// TS: should omit reasoning effort and warn for models that do not support it
    #[tokio::test]
    async fn unsupported_reasoning_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4.20-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4.20-reasoning");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::None),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = result.request_body.unwrap();
        assert!(body.get("reasoning").is_none());
        assert!(result.warnings.iter().any(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, details } => {
                feature == "reasoning"
                    && details.as_deref()
                        == Some("reasoning \"none\" is not supported by this model.")
            }
            _ => false,
        }));
    }

    /// TS: should warn about unsupported stopSequences
    #[tokio::test]
    async fn warn_stop_sequences() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            stop_sequences: Some(vec!["\n\n".to_string(), "STOP".to_string()]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert!(result.warnings.iter().any(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => feature == "stopSequences",
            _ => false,
        }));
    }

    /// TS: should send response format json schema
    #[tokio::test]
    async fn response_format_json_schema() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            response_format: Some(ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"],
                    "additionalProperties": false
                })),
                name: Some("recipe".to_string()),
                description: Some("A recipe object".to_string()),
            }),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = result.request_body.unwrap();
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["text"]["format"]["strict"], true);
        assert_eq!(body["text"]["format"]["name"], "recipe");
        assert_eq!(body["text"]["format"]["description"], "A recipe object");
    }

    /// TS: should send response format json object when no schema provided
    #[tokio::test]
    async fn response_format_json_object() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            response_format: Some(ResponseFormat::Json {
                schema: None,
                name: None,
                description: None,
            }),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(
            result.request_body.unwrap()["text"]["format"]["type"],
            "json_object"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — xAI-specific tools
// ════════════════════════════════════════════════════════════════════════════

mod tools {
    use super::*;

    /// TS: should send web_search tool with args in request
    #[tokio::test]
    async fn web_search_tool_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({
                    "allowedDomains": ["wikipedia.org"],
                    "enableImageSearch": true,
                    "enableImageUnderstanding": true
                }),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let body = result.request_body.unwrap();
        assert_eq!(
            body["tools"],
            json!([{
                "type": "web_search",
                "allowed_domains": ["wikipedia.org"],
                "enable_image_search": true,
                "enable_image_understanding": true
            }])
        );
    }

    /// TS: should include web_search tool call
    #[tokio::test]
    async fn web_search_tool_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "web_search_call",
                    "id": "ws_123",
                    "name": "web_search",
                    "arguments": "{\"query\":\"test\"}",
                    "call_id": "",
                    "status": "completed"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({}),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                assert_eq!(tool_call_id, "ws_123");
                assert_eq!(tool_name, "web_search");
                assert_eq!(input, "{\"query\":\"test\"}");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// TS: should map web_search_call type to web_search tool name when name is empty
    #[tokio::test]
    async fn web_search_call_empty_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "web_search_call",
                    "id": "ws_123",
                    "name": "",
                    "arguments": "{\"query\":\"test\"}",
                    "call_id": "",
                    "status": "completed"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({}),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        match &result.content[0] {
            GenerateContent::ToolCall { tool_name, .. } => assert_eq!(tool_name, "web_search"),
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// TS: should map x_search_call type to x_search tool name when name is empty
    #[tokio::test]
    async fn x_search_call_empty_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "x_search_call",
                    "id": "xs_123",
                    "name": "",
                    "arguments": "{\"query\":\"test\"}",
                    "call_id": "",
                    "status": "completed"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.x_search".to_string(),
                name: "x_search".to_string(),
                args: json!({}),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        match &result.content[0] {
            GenerateContent::ToolCall { tool_name, .. } => assert_eq!(tool_name, "x_search"),
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// TS: should map code_interpreter_call type to code_execution tool name
    #[tokio::test]
    async fn code_interpreter_call_empty_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "code_interpreter_call",
                    "id": "ci_123",
                    "name": "",
                    "arguments": "{}",
                    "call_id": "",
                    "status": "completed"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.code_execution".to_string(),
                name: "code_execution".to_string(),
                args: json!({}),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        match &result.content[0] {
            GenerateContent::ToolCall { tool_name, .. } => assert_eq!(tool_name, "code_execution"),
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// TS: should map code_execution_call type to code_execution tool name
    #[tokio::test]
    async fn code_execution_call_empty_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "code_execution_call",
                    "id": "ce_123",
                    "name": "",
                    "arguments": "{}",
                    "call_id": "",
                    "status": "completed"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.code_execution".to_string(),
                name: "code_execution".to_string(),
                args: json!({}),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        match &result.content[0] {
            GenerateContent::ToolCall { tool_name, .. } => assert_eq!(tool_name, "code_execution"),
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// TS: should use custom tool name from provider tool when type matches
    #[tokio::test]
    async fn custom_tool_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "web_search_call",
                    "id": "ws_123",
                    "name": "",
                    "arguments": "{}",
                    "call_id": "",
                    "status": "completed"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "my_custom_search".to_string(),
                args: json!({}),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        match &result.content[0] {
            GenerateContent::ToolCall { tool_name, .. } => {
                assert_eq!(tool_name, "my_custom_search")
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// TS: should include file_search tool call and result
    #[tokio::test]
    async fn file_search_tool_call_and_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [
                    {
                        "type": "file_search_call",
                        "id": "fs_123",
                        "status": "completed",
                        "queries": ["AI safety research"],
                        "results": [
                            { "file_id": "file_abc123", "filename": "ai-safety-paper.pdf", "score": 0.95, "text": "Recent advances..." },
                            { "file_id": "file_def456", "filename": "research-notes.md", "score": 0.82, "text": "Key findings..." }
                        ]
                    },
                    {
                        "type": "message",
                        "id": "msg_123",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": "Based on the documents...", "annotations": [] }]
                    }
                ],
                "usage": { "input_tokens": 100, "output_tokens": 20 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.file_search".to_string(),
                name: "file_search".to_string(),
                args: json!({ "vectorStoreIds": ["collection_test123"] }),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.content.len(), 3);
        // Tool call
        match &result.content[0] {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                assert_eq!(tool_call_id, "fs_123");
                assert_eq!(tool_name, "file_search");
                assert_eq!(input, "");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        // Tool result
        match &result.content[1] {
            GenerateContent::ToolResult {
                tool_call_id,
                tool_name,
                result,
                ..
            } => {
                assert_eq!(tool_call_id, "fs_123");
                assert_eq!(tool_name, "file_search");
                assert_eq!(result["queries"], json!(["AI safety research"]));
                assert_eq!(result["results"].as_array().unwrap().len(), 2);
                assert_eq!(result["results"][0]["fileId"], "file_abc123");
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
        // Text
        match &result.content[2] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "Based on the documents..."),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    /// TS: should handle file_search with null results
    #[tokio::test]
    async fn file_search_null_results() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [
                    {
                        "type": "file_search_call",
                        "id": "fs_456",
                        "status": "completed",
                        "queries": ["nonexistent topic"],
                        "results": null
                    },
                    {
                        "type": "message",
                        "id": "msg_456",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": "No relevant documents found.", "annotations": [] }]
                    }
                ],
                "usage": { "input_tokens": 100, "output_tokens": 20 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.file_search".to_string(),
                name: "file_search".to_string(),
                args: json!({ "vectorStoreIds": ["collection_test123"] }),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        match &result.content[1] {
            GenerateContent::ToolResult { result, .. } => {
                assert_eq!(result["queries"], json!(["nonexistent topic"]));
                assert_eq!(result["results"], Value::Null);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    /// TS: should include function tool calls
    #[tokio::test]
    async fn function_tool_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "function_call",
                    "id": "fc_123",
                    "name": "weather",
                    "arguments": "{\"location\":\"sf\"}",
                    "call_id": "call_123"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![
                FunctionTool::new(
                    "weather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } }
                    }),
                )
                .into(),
            ]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        match &result.content[0] {
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                assert_eq!(tool_call_id, "call_123");
                assert_eq!(tool_name, "weather");
                assert_eq!(input, &Value::String(r#"{"location":"sf"}"#.into()));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// TS: should omit additionalProperties from serialized function tool schemas
    #[tokio::test]
    async fn omit_additional_properties() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning", "output": [],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![
                FunctionTool::new(
                    "saveContact",
                    json!({
                        "type": "object",
                        "properties": {
                            "address": {
                                "type": "object",
                                "properties": { "city": { "type": "string" } },
                                "required": ["city"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["address"],
                        "additionalProperties": false
                    }),
                )
                .into(),
            ]),
            ..default_options(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        let tools = result.request_body.unwrap()["tools"]
            .as_array()
            .unwrap()
            .clone();
        let params = &tools[0]["parameters"];
        // additionalProperties: false should be removed at all levels.
        assert!(params.get("additionalProperties").is_none());
        assert!(
            params["properties"]["address"]
                .get("additionalProperties")
                .is_none()
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — citations
// ════════════════════════════════════════════════════════════════════════════

mod citations {
    use super::*;

    /// TS: should extract citations from annotations
    #[tokio::test]
    async fn extract_citations() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_123", "object": "response", "status": "completed",
                "model": "grok-4-fast-non-reasoning",
                "output": [{
                    "type": "message",
                    "id": "msg_123",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{
                        "type": "output_text",
                        "text": "based on research",
                        "annotations": [
                            { "type": "url_citation", "url": "https://example.com", "title": "example title" },
                            { "type": "url_citation", "url": "https://test.com" }
                        ]
                    }]
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            })))
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.content.len(), 3);
        // Text
        match &result.content[0] {
            GenerateContent::Text { text, .. } => assert_eq!(text, "based on research"),
            other => panic!("expected Text, got {other:?}"),
        }
        // Source 1
        match &result.content[1] {
            GenerateContent::Source { url, title, .. } => {
                assert_eq!(url.as_deref(), Some("https://example.com"));
                assert_eq!(title.as_deref(), Some("example title"));
            }
            other => panic!("expected Source, got {other:?}"),
        }
        // Source 2 (title falls back to url)
        match &result.content[2] {
            GenerateContent::Source { url, title, .. } => {
                assert_eq!(url.as_deref(), Some("https://test.com"));
                assert_eq!(title.as_deref(), Some("https://test.com"));
            }
            other => panic!("expected Source, got {other:?}"),
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — text streaming
// ════════════════════════════════════════════════════════════════════════════

mod do_stream {
    use super::*;

    /// TS: should stream text deltas
    #[tokio::test]
    async fn stream_text_deltas() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({
                            "type": "response.created",
                            "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "output": [] }
                        }).to_string()),
                        &sse_event(&json!({
                            "type": "response.output_text.delta",
                            "item_id": "msg_123",
                            "output_index": 0,
                            "content_index": 0,
                            "delta": "Hello"
                        }).to_string()),
                        &sse_event(&json!({
                            "type": "response.output_text.delta",
                            "item_id": "msg_123",
                            "output_index": 0,
                            "content_index": 0,
                            "delta": " world"
                        }).to_string()),
                        &sse_event(&json!({
                            "type": "response.completed",
                            "response": {
                                "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning",
                                "status": "completed", "output": [],
                                "usage": { "input_tokens": 10, "output_tokens": 5 }
                            }
                        }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let text_deltas: Vec<&str> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_deltas, vec!["Hello", " world"]);

        // Should have StreamStart, ResponseMetadata, TextStart, TextDelta x2, TextEnd, Finish
        let has_start = parts
            .iter()
            .any(|p| matches!(p, StreamPart::StreamStart { .. }));
        let has_finish = parts.iter().any(|p| matches!(p, StreamPart::Finish { .. }));
        assert!(has_start);
        assert!(has_finish);
    }

    /// TS: should not emit duplicate text-delta from response.output_item.done after streaming
    #[tokio::test]
    async fn no_duplicate_text_delta() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({
                            "type": "response.created",
                            "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "status": "in_progress", "output": [] }
                        }).to_string()),
                        &sse_event(&json!({
                            "type": "response.output_item.added",
                            "item": { "type": "message", "id": "msg_123", "status": "in_progress", "role": "assistant", "content": [] },
                            "output_index": 0
                        }).to_string()),
                        &sse_event(&json!({ "type": "response.output_text.delta", "item_id": "msg_123", "output_index": 0, "content_index": 0, "delta": "Hello" }).to_string()),
                        &sse_event(&json!({ "type": "response.output_text.delta", "item_id": "msg_123", "output_index": 0, "content_index": 0, "delta": " " }).to_string()),
                        &sse_event(&json!({ "type": "response.output_text.delta", "item_id": "msg_123", "output_index": 0, "content_index": 0, "delta": "world" }).to_string()),
                        &sse_event(&json!({
                            "type": "response.output_item.done",
                            "item": { "type": "message", "id": "msg_123", "status": "completed", "role": "assistant", "content": [{ "type": "output_text", "text": "Hello world", "annotations": [] }] },
                            "output_index": 0
                        }).to_string()),
                        &sse_event(&json!({
                            "type": "response.done",
                            "response": { "id": "resp_123", "object": "response", "status": "completed", "output": [], "usage": { "input_tokens": 10, "output_tokens": 5 } }
                        }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let text_deltas: Vec<&str> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        // Should only have 3 text-deltas, NOT 4 (with duplicate full text)
        assert_eq!(text_deltas, vec!["Hello", " ", "world"]);
    }

    /// TS: should stream reasoning text deltas (response.reasoning_text.delta)
    #[tokio::test]
    async fn stream_reasoning_text_deltas() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-code-fast-1", "output": [] } }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.added", "item": { "type": "reasoning", "id": "rs_456", "status": "in_progress", "summary": [] }, "output_index": 0 }).to_string()),
                        &sse_event(&json!({ "type": "response.reasoning_text.delta", "item_id": "rs_456", "output_index": 0, "content_index": 0, "delta": "First" }).to_string()),
                        &sse_event(&json!({ "type": "response.reasoning_text.delta", "item_id": "rs_456", "output_index": 0, "content_index": 0, "delta": ", analyze the question." }).to_string()),
                        &sse_event(&json!({ "type": "response.reasoning_text.done", "item_id": "rs_456", "output_index": 0, "content_index": 0, "text": "First, analyze the question." }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_456", "status": "completed", "summary": [{ "type": "summary_text", "text": "First, analyze the question." }] }, "output_index": 0 }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.added", "item": { "type": "message", "id": "msg_789", "role": "assistant", "status": "in_progress", "content": [] }, "output_index": 1 }).to_string()),
                        &sse_event(&json!({ "type": "response.output_text.delta", "item_id": "msg_789", "output_index": 1, "content_index": 0, "delta": "The answer." }).to_string()),
                        &sse_event(&json!({ "type": "response.done", "response": { "id": "resp_123", "object": "response", "model": "grok-code-fast-1", "status": "completed", "output": [], "usage": { "input_tokens": 10, "output_tokens": 20, "output_tokens_details": { "reasoning_tokens": 15 } } } }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-code-fast-1");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        // Verify reasoning start/delta/end ordering.
        let start_idx = parts
            .iter()
            .position(|p| matches!(p, StreamPart::ReasoningStart { .. }));
        let first_delta_idx = parts
            .iter()
            .position(|p| matches!(p, StreamPart::ReasoningDelta { .. }));
        let end_idx = parts
            .iter()
            .position(|p| matches!(p, StreamPart::ReasoningEnd { .. }));
        let text_idx = parts
            .iter()
            .position(|p| matches!(p, StreamPart::TextDelta { .. }));

        assert!(start_idx.is_some());
        assert!(first_delta_idx.is_some());
        assert!(end_idx.is_some());
        assert!(text_idx.is_some());
        assert!(start_idx.unwrap() < first_delta_idx.unwrap());
        assert!(first_delta_idx.unwrap() < end_idx.unwrap());
        assert!(end_idx.unwrap() < text_idx.unwrap());

        // Verify reasoning deltas.
        let reasoning_deltas: Vec<&str> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ReasoningDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(reasoning_deltas, vec!["First", ", analyze the question."]);
    }

    /// TS: should include encrypted content in reasoning-end providerMetadata
    #[tokio::test]
    async fn reasoning_end_encrypted_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "output": [] } }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.added", "item": { "type": "reasoning", "id": "rs_456", "status": "in_progress", "summary": [] }, "output_index": 0 }).to_string()),
                        &sse_event(&json!({ "type": "response.reasoning_summary_part.added", "item_id": "rs_456", "output_index": 0, "summary_index": 0, "part": { "type": "summary_text", "text": "" } }).to_string()),
                        &sse_event(&json!({ "type": "response.reasoning_summary_text.delta", "item_id": "rs_456", "output_index": 0, "summary_index": 0, "delta": "Analyzing..." }).to_string()),
                        &sse_event(&json!({ "type": "response.reasoning_summary_text.done", "item_id": "rs_456", "output_index": 0, "summary_index": 0, "text": "Analyzing..." }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_456", "status": "completed", "summary": [{ "type": "summary_text", "text": "Analyzing..." }], "encrypted_content": "encrypted_data_abc123" }, "output_index": 0 }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.added", "item": { "type": "message", "id": "msg_789", "role": "assistant", "status": "in_progress", "content": [] }, "output_index": 1 }).to_string()),
                        &sse_event(&json!({ "type": "response.output_text.delta", "item_id": "msg_789", "output_index": 1, "content_index": 0, "delta": "Result." }).to_string()),
                        &sse_event(&json!({ "type": "response.done", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "status": "completed", "output": [], "usage": { "input_tokens": 10, "output_tokens": 20 } } }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let reasoning_end = parts
            .iter()
            .find(|p| matches!(p, StreamPart::ReasoningEnd { .. }));
        assert!(reasoning_end.is_some());
        if let Some(StreamPart::ReasoningEnd {
            id,
            provider_metadata,
        }) = reasoning_end
        {
            assert_eq!(id, "reasoning-rs_456");
            assert_eq!(
                provider_metadata,
                &Some(
                    json!({ "xai": { "itemId": "rs_456", "reasoningEncryptedContent": "encrypted_data_abc123" } })
                )
            );
        }
    }

    /// TS: should stream web_search tool calls
    #[tokio::test]
    async fn stream_web_search_tool() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "status": "in_progress", "output": [] } }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.added", "item": { "type": "web_search_call", "id": "ws_123", "name": "web_search", "arguments": "{\"query\":\"test\"}", "call_id": "", "status": "completed" }, "output_index": 0 }).to_string()),
                        &sse_event(&json!({ "type": "response.done", "response": { "id": "resp_123", "object": "response", "status": "completed", "output": [], "usage": { "input_tokens": 10, "output_tokens": 5 } } }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({}),
            })]),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        let tool_call = parts
            .iter()
            .find(|p| matches!(p, StreamPart::ToolCall { .. }));
        assert!(tool_call.is_some());
        if let Some(StreamPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) = tool_call
        {
            assert_eq!(tool_call_id, "ws_123");
            assert_eq!(tool_name, "web_search");
            assert_eq!(input, "{\"query\":\"test\"}");
        }
    }

    /// TS: should stream function tool call arguments
    #[tokio::test]
    async fn stream_function_tool_arguments() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "status": "in_progress", "output": [] } }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.added", "output_index": 0, "item": { "type": "function_call", "id": "fc_123", "call_id": "call_123", "name": "weather", "arguments": "", "status": "in_progress" } }).to_string()),
                        &sse_event(&json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_123", "output_index": 0, "delta": "{\"location\"" }).to_string()),
                        &sse_event(&json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_123", "output_index": 0, "delta": ":\"sf\"}" }).to_string()),
                        &sse_event(&json!({ "type": "response.function_call_arguments.done", "item_id": "fc_123", "output_index": 0, "arguments": "{\"location\":\"sf\"}" }).to_string()),
                        &sse_event(&json!({ "type": "response.output_item.done", "output_index": 0, "item": { "type": "function_call", "id": "fc_123", "call_id": "call_123", "name": "weather", "arguments": "{\"location\":\"sf\"}", "status": "completed" } }).to_string()),
                        &sse_event(&json!({ "type": "response.done", "response": { "id": "resp_123", "object": "response", "status": "completed", "output": [], "usage": { "input_tokens": 10, "output_tokens": 5 } } }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let options = CallOptions {
            tools: Some(vec![
                FunctionTool::new(
                    "weather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } }
                    }),
                )
                .into(),
            ]),
            ..default_options(test_prompt())
        };
        let result = model.do_stream(&options).await.unwrap();
        let parts = collect_stream(result).await;

        // Should have tool-input-start, tool-input-delta x2, tool-input-end, tool-call
        let has_start = parts.iter().any(|p| matches!(p, StreamPart::ToolInputStart { id, tool_name, .. } if id == "call_123" && tool_name == "weather"));
        assert!(has_start);

        let deltas: Vec<&str> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ToolInputDelta { id, delta, .. } if id == "call_123" => {
                    Some(delta.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["{\"location\"", ":\"sf\"}"]);

        let has_end = parts
            .iter()
            .any(|p| matches!(p, StreamPart::ToolInputEnd { id, .. } if id == "call_123"));
        assert!(has_end);

        let tool_call = parts
            .iter()
            .find(|p| matches!(p, StreamPart::ToolCall { .. }));
        assert!(tool_call.is_some());
        if let Some(StreamPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) = tool_call
        {
            assert_eq!(tool_call_id, "call_123");
            assert_eq!(tool_name, "weather");
            assert_eq!(input, &Value::String(r#"{"location":"sf"}"#.into()));
        }

        // Finish reason should be tool-calls.
        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        if let Some(StreamPart::Finish { finish_reason, .. }) = finish {
            assert_eq!(
                finish_reason.unified,
                aimux_core::types::FinishReasonUnified::ToolCalls
            );
        }
    }

    /// TS: should expose cost_in_usd_ticks in finish providerMetadata
    #[tokio::test]
    async fn stream_cost_in_usd_ticks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "output": [] } }).to_string()),
                        &sse_event(&json!({ "type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": "Hello" }).to_string()),
                        &sse_event(&json!({
                            "type": "response.completed",
                            "response": {
                                "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning",
                                "status": "completed", "output": [],
                                "usage": { "input_tokens": 10, "output_tokens": 5, "cost_in_usd_ticks": 113500 }
                            }
                        }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        if let Some(StreamPart::Finish {
            provider_metadata, ..
        }) = finish
        {
            assert_eq!(
                provider_metadata,
                &Some(json!({ "xai": { "costInUsdTicks": 113500 } }))
            );
        } else {
            panic!("no finish part found");
        }
    }

    /// TS: should stream citations as sources
    #[tokio::test]
    async fn stream_citations() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "status": "in_progress", "output": [] } }).to_string()),
                        &sse_event(&json!({
                            "type": "response.output_text.annotation.added",
                            "item_id": "msg_123", "output_index": 0, "content_index": 0, "annotation_index": 0,
                            "annotation": { "type": "url_citation", "url": "https://example.com", "title": "example" }
                        }).to_string()),
                        &sse_event(&json!({ "type": "response.done", "response": { "id": "resp_123", "object": "response", "status": "completed", "output": [], "usage": { "input_tokens": 10, "output_tokens": 5 } } }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let source = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Source { .. }));
        assert!(source.is_some());
        if let Some(StreamPart::Source { url, title, .. }) = source {
            assert_eq!(url.as_deref(), Some("https://example.com"));
            assert_eq!(title.as_deref(), Some("example"));
        }
    }

    /// TS: should set finish reason to length for max_output_tokens (incomplete)
    #[tokio::test]
    async fn incomplete_max_output_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "output": [] } }).to_string()),
                        &sse_event(&json!({ "type": "response.output_text.delta", "item_id": "msg_456", "output_index": 0, "content_index": 0, "delta": "partial output" }).to_string()),
                        &sse_event(&json!({
                            "type": "response.incomplete",
                            "response": { "incomplete_details": { "reason": "max_output_tokens" }, "usage": { "input_tokens": 100, "output_tokens": 4096 } }
                        }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        if let Some(StreamPart::Finish {
            finish_reason,
            usage,
            ..
        }) = finish
        {
            assert_eq!(
                finish_reason.unified,
                aimux_core::types::FinishReasonUnified::Length
            );
            assert_eq!(finish_reason.raw.as_deref(), Some("max_output_tokens"));
            assert_eq!(usage.input_tokens.total, Some(100));
            assert_eq!(usage.output_tokens.total, Some(4096));
        } else {
            panic!("no finish part found");
        }
    }

    /// TS: should set finish reason to error on response.failed
    #[tokio::test]
    async fn failed_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "output": [] } }).to_string()),
                        &sse_event(&json!({
                            "type": "response.failed",
                            "response": { "error": { "code": "server_error", "message": "Internal server error" }, "usage": { "input_tokens": 50, "output_tokens": 0 } }
                        }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        if let Some(StreamPart::Finish { finish_reason, .. }) = finish {
            assert_eq!(
                finish_reason.unified,
                aimux_core::types::FinishReasonUnified::Error
            );
            assert_eq!(finish_reason.raw.as_deref(), Some("error"));
        } else {
            panic!("no finish part found");
        }
    }

    /// TS: should handle missing usage in streaming response
    #[tokio::test]
    async fn missing_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[
                        &sse_event(&json!({ "type": "response.created", "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "status": "in_progress", "output": [] } }).to_string()),
                        &sse_event(&json!({ "type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": "Hello" }).to_string()),
                        &sse_event(&json!({
                            "type": "response.completed",
                            "response": { "id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "status": "completed", "output": [] }
                        }).to_string()),
                    ])),
            )
            .mount(&server)
            .await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-4-fast-non-reasoning");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        if let Some(StreamPart::Finish {
            finish_reason,
            usage,
            ..
        }) = finish
        {
            assert_eq!(
                finish_reason.unified,
                aimux_core::types::FinishReasonUnified::Stop
            );
            assert_eq!(finish_reason.raw.as_deref(), Some("completed"));
            assert_eq!(usage.input_tokens.total, Some(0));
            assert_eq!(usage.output_tokens.total, Some(0));
        } else {
            panic!("no finish part found");
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convertXaiResponsesUsage — direct function tests
// (convert-xai-responses-usage.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod convert_usage {
    use aimux_providers::xai::responses::convert::convert_xai_responses_usage;
    use aimux_providers::xai::responses::types::XaiResponsesUsage;

    fn make_usage(input: u64, output: u64) -> XaiResponsesUsage {
        XaiResponsesUsage {
            input_tokens: input,
            output_tokens: output,
            ..Default::default()
        }
    }

    /// TS: should convert basic usage without caching or reasoning
    #[test]
    fn basic_usage() {
        let result = convert_xai_responses_usage(&make_usage(100, 50));
        assert_eq!(result.input_tokens.total, Some(100));
        assert_eq!(result.input_tokens.no_cache, Some(100));
        assert_eq!(result.input_tokens.cache_read, Some(0));
        assert_eq!(result.output_tokens.total, Some(50));
        assert_eq!(result.output_tokens.text, Some(50));
        assert_eq!(result.output_tokens.reasoning, Some(0));
    }

    /// TS: should convert usage with reasoning tokens
    #[test]
    fn usage_with_reasoning() {
        let mut usage = make_usage(1941, 583);
        usage.output_tokens_details = Some(
            aimux_providers::xai::responses::types::XaiResponsesOutputTokensDetails {
                reasoning_tokens: Some(380),
            },
        );
        let result = convert_xai_responses_usage(&usage);
        assert_eq!(result.output_tokens.total, Some(583));
        assert_eq!(result.output_tokens.text, Some(203)); // 583 - 380
        assert_eq!(result.output_tokens.reasoning, Some(380));
    }

    /// TS: should convert usage with cached input tokens
    #[test]
    fn usage_with_cached_tokens() {
        let mut usage = make_usage(200, 50);
        usage.input_tokens_details = Some(
            aimux_providers::xai::responses::types::XaiResponsesInputTokensDetails {
                cached_tokens: Some(150),
            },
        );
        let result = convert_xai_responses_usage(&usage);
        assert_eq!(result.input_tokens.cache_read, Some(150));
        assert_eq!(result.input_tokens.no_cache, Some(50)); // 200 - 150
        assert_eq!(result.input_tokens.total, Some(200));
    }

    /// TS: should handle cached_tokens exceeding input_tokens (non-inclusive reporting)
    #[test]
    fn cached_exceeds_input() {
        let mut usage = make_usage(4142, 254);
        usage.input_tokens_details = Some(
            aimux_providers::xai::responses::types::XaiResponsesInputTokensDetails {
                cached_tokens: Some(4328),
            },
        );
        let result = convert_xai_responses_usage(&usage);
        assert_eq!(result.input_tokens.cache_read, Some(4328));
        assert_eq!(result.input_tokens.no_cache, Some(4142));
        assert_eq!(result.input_tokens.total, Some(8470)); // 4142 + 4328
    }

    /// TS: should convert usage with both cached input and reasoning
    #[test]
    fn both_cached_and_reasoning() {
        let mut usage = make_usage(200, 583);
        usage.input_tokens_details = Some(
            aimux_providers::xai::responses::types::XaiResponsesInputTokensDetails {
                cached_tokens: Some(150),
            },
        );
        usage.output_tokens_details = Some(
            aimux_providers::xai::responses::types::XaiResponsesOutputTokensDetails {
                reasoning_tokens: Some(380),
            },
        );
        let result = convert_xai_responses_usage(&usage);
        assert_eq!(result.input_tokens.cache_read, Some(150));
        assert_eq!(result.input_tokens.no_cache, Some(50));
        assert_eq!(result.input_tokens.total, Some(200));
        assert_eq!(result.output_tokens.reasoning, Some(380));
        assert_eq!(result.output_tokens.text, Some(203));
        assert_eq!(result.output_tokens.total, Some(583));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// prepareResponsesTools — direct function tests
// (xai-responses-prepare-tools.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod prepare_tools {
    use super::*;
    use aimux_providers::xai::responses::convert::prepare_responses_tools;

    /// TS: should return undefined tools when tools are undefined
    #[test]
    fn no_tools() {
        let result = prepare_responses_tools(&None, None);
        assert!(result.tools.is_none());
        assert!(result.tool_choice.is_none());
    }

    /// TS: should return undefined tools when tools are empty
    #[test]
    fn empty_tools() {
        let result = prepare_responses_tools(&Some(vec![]), None);
        assert!(result.tools.is_none());
    }

    /// TS: should prepare web_search tool with no args
    #[test]
    fn web_search_no_args() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({}),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "web_search");
    }

    /// TS: should prepare web_search tool with allowed domains
    #[test]
    fn web_search_with_domains() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({ "allowedDomains": ["wikipedia.org", "example.com"] }),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(
            tools[0]["allowed_domains"],
            json!(["wikipedia.org", "example.com"])
        );
    }

    /// TS: should prepare x_search tool with no args
    #[test]
    fn x_search_no_args() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.x_search".to_string(),
                name: "x_search".to_string(),
                args: json!({}),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["type"], "x_search");
    }

    /// TS: should prepare code_execution tool as code_interpreter
    #[test]
    fn code_execution() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.code_execution".to_string(),
                name: "code_execution".to_string(),
                args: json!({}),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["type"], "code_interpreter");
    }

    /// TS: should prepare view_image tool
    #[test]
    fn view_image() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.view_image".to_string(),
                name: "view_image".to_string(),
                args: json!({}),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["type"], "view_image");
    }

    /// TS: should prepare view_x_video tool
    #[test]
    fn view_x_video() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.view_x_video".to_string(),
                name: "view_x_video".to_string(),
                args: json!({}),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["type"], "view_x_video");
    }

    /// TS: should prepare file_search tool with vector store IDs
    #[test]
    fn file_search() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.file_search".to_string(),
                name: "file_search".to_string(),
                args: json!({ "vectorStoreIds": ["collection_1", "collection_2"] }),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["type"], "file_search");
        assert_eq!(
            tools[0]["vector_store_ids"],
            json!(["collection_1", "collection_2"])
        );
    }

    /// TS: should prepare file_search tool with max num results
    #[test]
    fn file_search_max_results() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.file_search".to_string(),
                name: "file_search".to_string(),
                args: json!({ "vectorStoreIds": ["collection_1"], "maxNumResults": 10 }),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["max_num_results"], 10);
    }

    /// TS: should prepare mcp tool with required args only
    #[test]
    fn mcp_required_args() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.mcp".to_string(),
                name: "mcp".to_string(),
                args: json!({ "serverUrl": "https://example.com/mcp", "serverLabel": "test-server" }),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["type"], "mcp");
        assert_eq!(tools[0]["server_url"], "https://example.com/mcp");
        assert_eq!(tools[0]["server_label"], "test-server");
    }

    /// TS: should prepare mcp tool with all optional args
    #[test]
    fn mcp_all_args() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.mcp".to_string(),
                name: "mcp".to_string(),
                args: json!({
                    "serverUrl": "https://example.com/mcp",
                    "serverLabel": "test-server",
                    "serverDescription": "A test MCP server",
                    "allowedTools": ["tool1", "tool2"],
                    "headers": { "X-Custom": "value" },
                    "authorization": "Bearer token123"
                }),
            })]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["server_description"], "A test MCP server");
        assert_eq!(tools[0]["allowed_tools"], json!(["tool1", "tool2"]));
        assert_eq!(tools[0]["headers"]["X-Custom"], "value");
        assert_eq!(tools[0]["authorization"], "Bearer token123");
    }

    /// TS: should prepare function tools
    #[test]
    fn function_tools() {
        let result = prepare_responses_tools(
            &Some(vec![
                FunctionTool::new(
                    "weather",
                    json!({
                        "type": "object",
                        "properties": { "location": { "type": "string" } },
                        "required": ["location"]
                    }),
                )
                .with_description("get weather information")
                .into(),
            ]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "weather");
        assert_eq!(tools[0]["description"], "get weather information");
        assert_eq!(
            tools[0]["parameters"]["properties"]["location"]["type"],
            "string"
        );
    }

    /// TS: should pass through strict mode when strict is true
    #[test]
    fn strict_true() {
        let result = prepare_responses_tools(
            &Some(vec![
                FunctionTool::new(
                    "testFunction",
                    json!({
                        "type": "object", "properties": {}
                    }),
                )
                .with_description("A test function")
                .with_strict(true)
                .into(),
            ]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools[0]["strict"], true);
    }

    /// TS: should not include strict when strict is undefined
    #[test]
    fn strict_undefined() {
        let result = prepare_responses_tools(
            &Some(vec![
                FunctionTool::new(
                    "testFunction",
                    json!({
                        "type": "object", "properties": {}
                    }),
                )
                .with_description("A test function")
                .into(),
            ]),
            None,
        );
        let tools = result.tools.unwrap();
        assert!(tools[0].get("strict").is_none());
    }

    /// TS: should handle tool choice auto
    #[test]
    fn tool_choice_auto() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({}),
            })]),
            Some(&ToolChoice::Auto),
        );
        assert_eq!(result.tool_choice, Some(json!("auto")));
    }

    /// TS: should handle tool choice required
    #[test]
    fn tool_choice_required() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({}),
            })]),
            Some(&ToolChoice::Required),
        );
        assert_eq!(result.tool_choice, Some(json!("required")));
    }

    /// TS: should handle tool choice none
    #[test]
    fn tool_choice_none() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({}),
            })]),
            Some(&ToolChoice::None),
        );
        assert_eq!(result.tool_choice, Some(json!("none")));
    }

    /// TS: should handle specific tool choice (function)
    #[test]
    fn tool_choice_specific_function() {
        let result = prepare_responses_tools(
            &Some(vec![
                FunctionTool::new(
                    "weather",
                    json!({
                        "type": "object", "properties": {}
                    }),
                )
                .into(),
            ]),
            Some(&ToolChoice::Tool {
                tool_name: "weather".to_string(),
            }),
        );
        assert_eq!(
            result.tool_choice,
            Some(json!({ "type": "function", "name": "weather" }))
        );
    }

    /// TS: should warn when trying to force server-side tool via toolChoice
    #[test]
    fn tool_choice_server_side_warning() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "xai.web_search".to_string(),
                name: "web_search".to_string(),
                args: json!({}),
            })]),
            Some(&ToolChoice::Tool {
                tool_name: "web_search".to_string(),
            }),
        );
        assert!(result.tool_choice.is_none());
        assert!(result.tool_warnings.iter().any(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => {
                feature == "toolChoice for server-side tool \"web_search\""
            }
            _ => false,
        }));
    }

    /// TS: should warn about unsupported provider-defined tools
    #[test]
    fn unsupported_provider_tool() {
        let result = prepare_responses_tools(
            &Some(vec![Tool::Provider(ProviderTool {
                id: "unsupported.tool".to_string(),
                name: "unsupported".to_string(),
                args: json!({}),
            })]),
            None,
        );
        assert!(result.tools.is_none());
        assert!(result.tool_warnings.iter().any(|w| match w {
            aimux_core::types::Warning::Unsupported { feature, .. } => {
                feature == "provider-defined tool unsupported"
            }
            _ => false,
        }));
    }

    /// TS: should handle multiple tools including provider-defined and functions
    #[test]
    fn multiple_tools() {
        let result = prepare_responses_tools(
            &Some(vec![
                FunctionTool::new("calculator", json!({ "type": "object", "properties": {} }))
                    .into(),
                Tool::Provider(ProviderTool {
                    id: "xai.web_search".to_string(),
                    name: "web_search".to_string(),
                    args: json!({}),
                }),
                Tool::Provider(ProviderTool {
                    id: "xai.x_search".to_string(),
                    name: "x_search".to_string(),
                    args: json!({}),
                }),
            ]),
            None,
        );
        let tools = result.tools.unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[1]["type"], "web_search");
        assert_eq!(tools[2]["type"], "x_search");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convertToXaiResponsesInput — direct function tests
// (convert-to-xai-responses-input.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod convert_input {
    use super::*;
    use aimux_providers::xai::responses::convert::convert_to_xai_responses_input;

    /// TS: should convert system messages
    #[test]
    fn system_message() {
        let prompt = vec![LanguageModelPromptMessage {
            role: Role::System,
            content: vec![ContentPart::text("you are a helpful assistant")],
            ..Default::default()
        }];
        let (input, warnings) = convert_to_xai_responses_input(&prompt).unwrap();
        assert_eq!(warnings.len(), 0);
        assert_eq!(
            input,
            vec![json!({ "role": "system", "content": "you are a helpful assistant" })]
        );
    }

    /// TS: should convert single text part
    #[test]
    fn user_text() {
        let (input, warnings) = convert_to_xai_responses_input(&test_prompt()).unwrap();
        assert_eq!(warnings.len(), 0);
        assert_eq!(
            input,
            vec![json!({
                "role": "user",
                "content": [{ "type": "input_text", "text": "hello" }]
            })]
        );
    }

    /// TS: should convert image file parts with URL
    #[test]
    fn user_image_url() {
        let prompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![
                ContentPart::text("what is in this image"),
                ContentPart::file_url("https://example.com/image.jpg", "image/jpeg"),
            ],
            ..Default::default()
        }];
        let (input, _) = convert_to_xai_responses_input(&prompt).unwrap();
        assert_eq!(
            input[0]["content"][1],
            json!({ "type": "input_image", "image_url": "https://example.com/image.jpg" })
        );
    }

    /// TS: should convert non-image file parts with URL to input_file with file_url
    #[test]
    fn user_pdf_url() {
        let prompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![
                ContentPart::text("summarize this PDF"),
                ContentPart::file_url("https://example.com/document.pdf", "application/pdf"),
            ],
            ..Default::default()
        }];
        let (input, _) = convert_to_xai_responses_input(&prompt).unwrap();
        assert_eq!(
            input[0]["content"][1],
            json!({ "type": "input_file", "file_url": "https://example.com/document.pdf" })
        );
    }

    /// TS: should convert assistant text
    #[test]
    fn assistant_text() {
        let prompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("hi")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::text("hello back")],
                ..Default::default()
            },
        ];
        let (input, _) = convert_to_xai_responses_input(&prompt).unwrap();
        assert_eq!(
            input[1],
            json!({ "role": "assistant", "content": "hello back" })
        );
    }

    /// TS: should convert tool results
    #[test]
    fn tool_result() {
        let prompt = vec![
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("weather?")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::tool_call("call_123", "weather", json!({}))],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Tool,
                content: vec![ContentPart::tool_result("call_123", json!("sunny"))],
                ..Default::default()
            },
        ];
        let (input, _) = convert_to_xai_responses_input(&prompt).unwrap();
        // Should have: user message, function_call, function_call_output
        assert_eq!(input.len(), 3);
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_123");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_123");
        assert_eq!(input[2]["output"], "sunny");
    }
}
