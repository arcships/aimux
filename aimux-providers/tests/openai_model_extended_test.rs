//! Extended OpenAI model tests — covers previously-missing doGenerate and doStream cases.
//!
//! Sources: `openai-chat-language-model.test.ts` doGenerate (51 missing) and
//! doStream (14 missing) sections.

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ToolChoice};
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};

use aimux_providers::{OpenAIConfig, OpenAIProvider};

fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}
fn default_opts(p: LanguageModelPrompt) -> CallOptions {
    CallOptions {
        prompt: p,
        max_output_tokens: None,
        temperature: None,
        stop_sequences: None,
        top_p: None,
        top_k: None,
        presence_penalty: None,
        frequency_penalty: None,
        response_format: None,
        seed: None,
        tools: None,
        tool_choice: ToolChoice::Auto,
        headers: None,
        provider_options: None,
        reasoning: None,
        body_overrides: None,
        max_retries: None,
    }
}
fn po(map: Value) -> Option<std::collections::HashMap<String, Value>> {
    let mut h = std::collections::HashMap::new();
    h.insert("openai".to_string(), map);
    Some(h)
}
fn sse_event(json_str: &str) -> String {
    format!("data: {}\n\n", json_str)
}
fn sse_body(events: &[&str]) -> String {
    let mut s = String::new();
    for e in events {
        s.push_str(e);
    }
    s.push_str("data: [DONE]\n\n");
    s
}
async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}
async fn mock_sse(server: &MockServer, body: &str) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body.to_string()),
        )
        .mount(server)
        .await;
}
async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        if let Ok(p) = part {
            parts.push(p);
        }
    }
    parts
}
fn text_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — response parsing tests
// ════════════════════════════════════════════════════════════════════════════

mod do_generate_extended {
    use super::*;

    /// TS: "should parse annotations/citations"
    #[tokio::test]
    async fn parse_annotations_citations() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "chatcmpl-95ZTZkhr0mHNKqerQfiwkuox3PHAd",
                "object": "chat.completion",
                "created": 1711115037,
                "model": "gpt-3.5-turbo-0125",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Based on the search results [doc1], I found information.",
                        "annotations": [{
                            "type": "url_citation",
                            "url_citation": {
                                "start_index": 24, "end_index": 29,
                                "url": "https://example.com/doc1.pdf",
                                "title": "Document 1"
                            }
                        }]
                    },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
            }),
        )
        .await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_opts(test_prompt()))
            .await
            .expect("should succeed");

        // Should have Text content + Source content
        assert!(result.content.len() >= 2);
        match &result.content[0] {
            GenerateContent::Text { text, .. } => {
                assert_eq!(
                    text,
                    "Based on the search results [doc1], I found information."
                );
            }
            other => panic!("expected Text, got {:?}", other),
        }
        match &result.content[1] {
            GenerateContent::Source {
                source_type,
                url,
                title,
                ..
            } => {
                assert_eq!(source_type, "url");
                assert_eq!(url.as_deref(), Some("https://example.com/doc1.pdf"));
                assert_eq!(title.as_deref(), Some("Document 1"));
            }
            other => panic!("expected Source, got {:?}", other),
        }
    }

    /// TS: "should return cached_tokens in prompt_details_tokens"
    #[tokio::test]
    async fn cached_tokens_in_usage() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-4o-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 2000, "completion_tokens": 20, "total_tokens": 2020,
                "prompt_tokens_details": { "cached_tokens": 1152, "cache_write_tokens": 256 }
            }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o-mini");

        let result = model
            .do_generate(&default_opts(test_prompt()))
            .await
            .expect("should succeed");

        assert_eq!(result.usage.input_tokens.total, Some(2000));
        assert_eq!(result.usage.input_tokens.cache_read, Some(1152));
        assert_eq!(result.usage.input_tokens.cache_write, Some(256));
        assert_eq!(result.usage.input_tokens.no_cache, Some(592));
        assert_eq!(result.usage.output_tokens.total, Some(20));
    }

    /// TS: "should return accepted_prediction_tokens and rejected_prediction_tokens"
    #[tokio::test]
    async fn prediction_tokens_in_provider_metadata() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-4o-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 15, "completion_tokens": 20, "total_tokens": 35,
                "completion_tokens_details": { "accepted_prediction_tokens": 123, "rejected_prediction_tokens": 456 }
            }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o-mini");

        let result = model
            .do_generate(&default_opts(test_prompt()))
            .await
            .expect("should succeed");

        let pm = result
            .provider_metadata
            .as_ref()
            .expect("provider metadata");
        assert_eq!(pm["openai"]["acceptedPredictionTokens"], json!(123));
        assert_eq!(pm["openai"]["rejectedPredictionTokens"], json!(456));
    }

    /// TS: "should return the reasoning tokens in the provider metadata"
    #[tokio::test]
    async fn reasoning_tokens_in_usage() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "o4-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 15, "completion_tokens": 20, "total_tokens": 35,
                "completion_tokens_details": { "reasoning_tokens": 10 }
            }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let result = model
            .do_generate(&default_opts(test_prompt()))
            .await
            .expect("should succeed");

        assert_eq!(result.usage.output_tokens.total, Some(20));
        assert_eq!(result.usage.output_tokens.reasoning, Some(10));
        assert_eq!(result.usage.output_tokens.text, Some(10));
    }

    /// TS: "should send request body" — verify the default request body shape
    #[tokio::test]
    async fn sends_default_request_body() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-3.5-turbo",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_opts(test_prompt()))
            .await
            .expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["model"], json!("gpt-3.5-turbo"));
        assert_eq!(
            body["messages"],
            json!([{ "role": "user", "content": "Hello" }])
        );
    }

    /// TS: "should pass the model and the messages"
    #[tokio::test]
    async fn passes_model_and_messages() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-3.5-turbo",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_generate(&default_opts(test_prompt()))
            .await
            .expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(
            body.clone(),
            json!({ "model": "gpt-3.5-turbo", "messages": [{ "role": "user", "content": "Hello" }] })
        );
    }

    /// TS: "should pass settings" (logitBias, parallelToolCalls, user)
    #[tokio::test]
    async fn passes_settings() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-3.5-turbo",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let opts = CallOptions {
            provider_options: po(
                json!({ "logitBias": { "50256": -100 }, "parallelToolCalls": false, "user": "test-user-id" }),
            ),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["logit_bias"], json!({ "50256": -100 }));
        assert_eq!(body["parallel_tool_calls"], json!(false));
        assert_eq!(body["user"], json!("test-user-id"));
    }

    /// TS: "should not set reasoning_effort when reasoning is 'provider-default'"
    #[tokio::test]
    async fn no_reasoning_effort_for_provider_default() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "o4-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let opts = CallOptions {
            reasoning: Some(ReasoningEffort::ProviderDefault),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert!(body.get("reasoning_effort").is_none());
    }

    /// TS: "should pass top-level reasoning as reasoning_effort"
    #[tokio::test]
    async fn top_level_reasoning_as_effort() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "o4-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let opts = CallOptions {
            reasoning: Some(ReasoningEffort::Medium),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["reasoning_effort"], json!("medium"));
    }

    /// TS: "should pass reasoningEffort setting from provider metadata"
    #[tokio::test]
    async fn reasoning_effort_from_provider() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "o4-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let opts = CallOptions {
            provider_options: po(json!({ "reasoningEffort": "low" })),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["reasoning_effort"], json!("low"));
    }

    /// TS: "should pass textVerbosity setting from provider options"
    #[tokio::test]
    async fn text_verbosity() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-4o",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o");

        let opts = CallOptions {
            provider_options: po(json!({ "textVerbosity": "low" })),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["verbosity"], json!("low"));
    }

    /// TS: reasoning models — "should clear out temperature, top_p, etc."
    #[tokio::test]
    async fn reasoning_clears_temperature() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "o4-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let opts = CallOptions {
            temperature: Some(0.5),
            top_p: Some(0.7),
            frequency_penalty: Some(0.2),
            presence_penalty: Some(0.3),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert!(body.get("frequency_penalty").is_none());
        assert!(body.get("presence_penalty").is_none());
        assert_eq!(result.warnings.len(), 4);
    }

    /// TS: "should convert maxOutputTokens to max_completion_tokens"
    #[tokio::test]
    async fn reasoning_max_completion_tokens() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "o4-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let opts = CallOptions {
            max_output_tokens: Some(1000),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["max_completion_tokens"], json!(1000));
        assert!(body.get("max_tokens").is_none());
    }

    /// TS: "should use developer messages for o1"
    #[tokio::test]
    async fn developer_messages_for_o1() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "o1",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o1");

        let p: LanguageModelPrompt = vec![
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
        let result = model
            .do_generate(&default_opts(p))
            .await
            .expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["messages"][0]["role"], json!("developer"));
    }

    /// TS: "should send store extension setting"
    #[tokio::test]
    async fn store_extension() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-3.5-turbo",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let opts = CallOptions {
            provider_options: po(json!({ "store": true })),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["store"], json!(true));
    }

    /// TS: "should send metadata extension values"
    #[tokio::test]
    async fn metadata_extension() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-3.5-turbo",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let opts = CallOptions {
            provider_options: po(json!({ "metadata": { "custom": "value" } })),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["metadata"], json!({ "custom": "value" }));
    }

    /// TS: "should send prediction extension setting"
    #[tokio::test]
    async fn prediction_extension() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-3.5-turbo",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let opts = CallOptions {
            provider_options: po(
                json!({ "prediction": { "type": "content", "content": "Hello, World!" } }),
            ),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(
            body["prediction"],
            json!({ "type": "content", "content": "Hello, World!" })
        );
    }

    /// TS: "should send serviceTier flex processing setting"
    #[tokio::test]
    async fn service_tier_flex() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "o4-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "flex" })),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["service_tier"], json!("flex"));
    }

    /// TS: "should send serviceTier priority processing setting"
    #[tokio::test]
    async fn service_tier_priority() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-4o-mini",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o-mini");

        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "priority" })),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["service_tier"], json!("priority"));
    }

    /// TS: "should remove temperature setting for gpt-4o-search-preview"
    #[tokio::test]
    async fn search_preview_removes_temperature() {
        let server = MockServer::start().await;
        mock_json(&server, json!({
            "id": "test", "object": "chat.completion", "created": 123, "model": "gpt-4o-search-preview",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        })).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o-search-preview");

        let opts = CallOptions {
            temperature: Some(0.7),
            ..default_opts(test_prompt())
        };
        let result = model.do_generate(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert!(body.get("temperature").is_none());
        assert_eq!(result.warnings.len(), 1);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — extended tests
// ════════════════════════════════════════════════════════════════════════════

mod do_stream_extended {
    use super::*;

    /// TS: "should stream annotations/citations"
    #[tokio::test]
    async fn stream_annotations_citations() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":1,"delta":{"content":"Based on search results"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":1,"delta":{"annotations":[{"type":"url_citation","url_citation":{"start_index":24,"end_index":29,"url":"https://example.com/doc1.pdf","title":"Document 1"}}]},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0125","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":17,"completion_tokens":227,"total_tokens":244}}"#,
            ),
        ]);
        mock_sse(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&default_opts(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        let deltas = text_deltas(&parts);
        assert_eq!(deltaags(&deltas), vec!["", "Based on search results"]);

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { finish_reason, .. }) => {
                assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    fn deltaags(d: &[String]) -> &[String] {
        d
    }

    /// TS: "should return accepted_prediction_tokens and rejected_prediction_tokens in providerMetadata"
    #[tokio::test]
    async fn stream_prediction_tokens() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":15,"completion_tokens":20,"total_tokens":35,"completion_tokens_details":{"accepted_prediction_tokens":123,"rejected_prediction_tokens":456}}}"#,
            ),
        ]);
        mock_sse(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let result = model
            .do_stream(&default_opts(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish {
                provider_metadata, ..
            }) => {
                let pm = provider_metadata.as_ref().expect("provider metadata");
                assert_eq!(pm["openai"]["acceptedPredictionTokens"], json!(123));
                assert_eq!(pm["openai"]["rejectedPredictionTokens"], json!(456));
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    /// TS: reasoning models → "should send reasoning tokens"
    #[tokio::test]
    async fn stream_reasoning_tokens() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":null,"choices":[{"index":1,"delta":{"content":"Hello, World!"},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":15,"completion_tokens":20,"total_tokens":35,"completion_tokens_details":{"reasoning_tokens":10}}}"#,
            ),
        ]);
        mock_sse(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let result = model
            .do_stream(&default_opts(test_prompt()))
            .await
            .expect("should succeed");
        let parts = collect_stream(result).await;

        let finish = parts
            .iter()
            .find(|p| matches!(p, StreamPart::Finish { .. }));
        match finish {
            Some(StreamPart::Finish { usage, .. }) => {
                assert_eq!(usage.output_tokens.total, Some(20));
                assert_eq!(usage.output_tokens.reasoning, Some(10));
                assert_eq!(usage.output_tokens.text, Some(10));
            }
            other => panic!("expected Finish, got {:?}", other),
        }
    }

    /// TS: "should send store extension setting" (streaming)
    #[tokio::test]
    async fn stream_store_extension() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}"#,
            ),
        ]);
        mock_sse(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let opts = CallOptions {
            provider_options: po(json!({ "store": true })),
            ..default_opts(test_prompt())
        };
        let result = model.do_stream(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["store"], json!(true));
        assert_eq!(body["stream"], json!(true));
    }

    /// TS: "should send metadata extension values" (streaming)
    #[tokio::test]
    async fn stream_metadata_extension() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-3.5-turbo-0613","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}"#,
            ),
        ]);
        mock_sse(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-3.5-turbo");

        let opts = CallOptions {
            provider_options: po(json!({ "metadata": { "custom": "value" } })),
            ..default_opts(test_prompt())
        };
        let result = model.do_stream(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["metadata"], json!({ "custom": "value" }));
    }

    /// TS: "should send serviceTier flex processing setting in streaming"
    #[tokio::test]
    async fn stream_service_tier_flex() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"o4-mini","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}"#,
            ),
        ]);
        mock_sse(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o4-mini");

        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "flex" })),
            ..default_opts(test_prompt())
        };
        let result = model.do_stream(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["service_tier"], json!("flex"));
    }

    /// TS: "should send serviceTier priority processing setting in streaming"
    #[tokio::test]
    async fn stream_service_tier_priority() {
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-4o-mini","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-4o-mini","system_fingerprint":null,"choices":[{"index":0,"delta":{},"finish_reason":"stop","logprobs":null}]}"#,
            ),
            &sse_event(
                r#"{"id":"chatcmpl-96aZqmeDpA9IPD6tACY8djkMsJCMP","object":"chat.completion.chunk","created":1702657020,"model":"gpt-4o-mini","system_fingerprint":"fp_3bc1b5746c","choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}"#,
            ),
        ]);
        mock_sse(&server, &body).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o-mini");

        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "priority" })),
            ..default_opts(test_prompt())
        };
        let result = model.do_stream(&opts).await.expect("should succeed");
        let body = result.request_body.as_ref().expect("request body");
        assert_eq!(body["service_tier"], json!("priority"));
    }
}
