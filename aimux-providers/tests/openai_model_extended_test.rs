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
use aimux_core::result::{GenerateContent, Source};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;

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
        timeout: None,
        abort_signal: None,
        session_id: None,
        include_raw_chunks: None,
        call_id: None,
        recording_context: None,
    }
}

fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
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
            other => panic!("expected Text, got {other:?}"),
        }
        match &result.content[1] {
            GenerateContent::Source(Source {
                source_type,
                url,
                title,
                ..
            }) => {
                assert_eq!(source_type, "url");
                assert_eq!(url.as_deref(), Some("https://example.com/doc1.pdf"));
                assert_eq!(title.as_deref(), Some("Document 1"));
            }
            other => panic!("expected Source, got {other:?}"),
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
            other => panic!("expected Finish, got {other:?}"),
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
            other => panic!("expected Finish, got {other:?}"),
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
            other => panic!("expected Finish, got {other:?}"),
        }
    }
}
