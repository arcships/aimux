//! Port of `@ai-sdk/google-vertex` 5.0.100
//! `src/xai/google-vertex-xai-provider.test.ts`.
//!
//! Upstream mocks `createOpenAICompatible` and inspects the settings handed
//! to it, calling its `transformRequestBody`, `convertUsage` and
//! `supportedUrls` directly. aimux builds the OpenAI-compatible provider
//! itself, so each setting is observed through the model: `baseURL` through
//! the request URL (`{baseURL}/chat/completions`), `name` through
//! `LanguageModel::provider` (`{name}.chat`), `includeUsage` and
//! `supportsStructuredOutputs` through the request bodies,
//! `transformRequestBody` through the request body, `convertUsage` through
//! the result's usage, `supportedUrls` through `LanguageModel::supported_urls`.
//! The transport is a mock (upstream's `fetch` is `undefined`) and the token
//! comes from the `generate_auth_token` setting (upstream's base provider has
//! no auth).

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::provider::Provider;
use aimux_core::types::ReasoningEffort;
use aimux_provider_utils::Resolvable;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use crate::vertex::{VertexXaiProvider, VertexXaiProviderSettings, create_google_vertex_xai};

/// A Chat Completions response with `usage`.
fn chat_response(usage: Value) -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": usage
    }))
}

fn ok_response() -> Canned {
    chat_response(json!({"prompt_tokens": 1, "completion_tokens": 1}))
}

/// A Chat Completions stream with one empty chunk.
fn stream_response() -> Canned {
    let chunk = json!({
        "id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "m",
        "choices": [{"index": 0, "delta": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
    });
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: format!("data: {chunk}\n\ndata: [DONE]\n\n").into_bytes(),
    }
}

fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// Settings of `test-project` with the mock transport and a fixed token.
fn settings(fetch: &Arc<MockFetch>) -> VertexXaiProviderSettings {
    VertexXaiProviderSettings {
        project: Some("test-project".into()),
        fetch: Some(fetch.transport()),
        generate_auth_token: Some(Resolvable::Value(Some("test-token".into()))),
        ..Default::default()
    }
}

/// The provider of `settings` in `location` (with `base_url`).
fn provider(
    fetch: &Arc<MockFetch>,
    location: Option<&str>,
    base_url: Option<&str>,
) -> VertexXaiProvider {
    create_google_vertex_xai(VertexXaiProviderSettings {
        location: location.map(String::from),
        base_url: base_url.map(String::from),
        ..settings(fetch)
    })
    .unwrap()
}

/// Generate once with `options` and return the request the transport saw.
async fn request(
    provider: &VertexXaiProvider,
    fetch: &Arc<MockFetch>,
    options: &CallOptions,
) -> Seen {
    let model = provider.call("xai/grok-4.1-fast-reasoning");
    assert_eq!(model.provider(), "googleVertex.xai.chat");
    model.do_generate(options).await.unwrap();
    fetch.seen().last().unwrap().clone()
}

mod google_vertex_xai_provider {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider.test.ts › google-vertex-xai-provider › should create a provider with correct base URL for global location
    #[tokio::test]
    async fn should_create_a_provider_with_correct_base_url_for_global_location() {
        let fetch = MockFetch::new(vec![ok_response()]);
        let provider = provider(&fetch, Some("global"), None);

        let seen = request(&provider, &fetch, &prompt()).await;
        assert_eq!(
            seen.url,
            "https://aiplatform.googleapis.com/v1/projects/test-project/locations/global/endpoints/openapi/chat/completions"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider.test.ts › google-vertex-xai-provider › should construct the default base URL when baseURL is an empty string
    #[tokio::test]
    async fn should_construct_the_default_base_url_when_base_url_is_an_empty_string() {
        let fetch = MockFetch::new(vec![ok_response()]);
        let provider = provider(&fetch, Some("global"), Some(""));

        let seen = request(&provider, &fetch, &prompt()).await;
        assert_eq!(
            seen.url,
            "https://aiplatform.googleapis.com/v1/projects/test-project/locations/global/endpoints/openapi/chat/completions"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider.test.ts › google-vertex-xai-provider › should create a provider with correct base URL for regional location
    #[tokio::test]
    async fn should_create_a_provider_with_correct_base_url_for_regional_location() {
        let fetch = MockFetch::new(vec![ok_response()]);
        let provider = provider(&fetch, Some("us-central1"), None);

        let seen = request(&provider, &fetch, &prompt()).await;
        assert_eq!(
            seen.url,
            "https://aiplatform.googleapis.com/v1/projects/test-project/locations/us-central1/endpoints/openapi/chat/completions"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider.test.ts › google-vertex-xai-provider › should configure OpenAI-compatible behavior for Vertex Grok
    #[tokio::test]
    async fn should_configure_openai_compatible_behavior_for_vertex_grok() {
        let fetch = MockFetch::new(vec![ok_response(), stream_response()]);
        let provider = provider(&fetch, Some("global"), None);
        let model = provider.call("xai/grok-4.1-fast-reasoning");
        assert_eq!(model.provider(), "googleVertex.xai.chat");

        // supportsStructuredOutputs: a JSON schema goes out as `json_schema`.
        let mut options = prompt();
        options.response_format = Some(ResponseFormat::Json {
            schema: Some(json!({"type": "object", "properties": {"a": {"type": "string"}}})),
            name: None,
            description: None,
        });
        model.do_generate(&options).await.unwrap();
        // includeUsage: streams ask for usage.
        let _stream = model.do_stream(&prompt()).await.unwrap();

        let seen = fetch.seen();
        assert_eq!(seen.len(), 2);
        let base_url = "https://aiplatform.googleapis.com/v1/projects/test-project/locations/global/endpoints/openapi";
        for seen in &seen {
            assert_eq!(seen.url, format!("{base_url}/chat/completions"));
        }
        assert_eq!(
            seen[0].json_body()["response_format"]["type"],
            "json_schema"
        );
        assert_eq!(
            seen[1].json_body()["stream_options"],
            json!({"include_usage": true})
        );
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider.test.ts › google-vertex-xai-provider › should strip reasoning_effort from request bodies
    ///
    /// Upstream calls `transformRequestBody` on a body with
    /// `reasoning_effort: 'high'`; here the body is the one a request with
    /// `reasoning: high` builds.
    #[tokio::test]
    async fn should_strip_reasoning_effort_from_request_bodies() {
        let fetch = MockFetch::new(vec![ok_response()]);
        let provider = provider(&fetch, Some("global"), None);
        let mut options = prompt();
        options.reasoning = Some(ReasoningEffort::High);

        let body = request(&provider, &fetch, &options).await.json_body();

        assert!(body.get("reasoning_effort").is_none(), "{body}");
        assert_eq!(body["model"], "xai/grok-4.1-fast-reasoning");
        assert!(body["messages"].is_array(), "{body}");
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider.test.ts › google-vertex-xai-provider › should count Grok reasoning tokens separately from completion tokens
    ///
    /// Upstream calls `convertUsage` on the usage object; here it is the
    /// usage of a response.
    #[tokio::test]
    async fn should_count_grok_reasoning_tokens_separately_from_completion_tokens() {
        let usage = json!({
            "prompt_tokens": 663,
            "prompt_tokens_details": {"cached_tokens": 654},
            "completion_tokens": 50,
            "completion_tokens_details": {"reasoning_tokens": 124}
        });
        let fetch = MockFetch::new(vec![chat_response(usage)]);
        let provider = provider(&fetch, Some("global"), None);

        let result = provider
            .call("xai/grok-4.1-fast-reasoning")
            .do_generate(&prompt())
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({
                "inputTokens": {
                    "cacheRead": 654,
                    "noCache": 9,
                    "total": 663
                },
                "outputTokens": {
                    "reasoning": 124,
                    "text": 50,
                    "total": 174
                },
                "raw": {
                    "completion_tokens": 50,
                    "completion_tokens_details": {
                        "reasoning_tokens": 124
                    },
                    "prompt_tokens": 663,
                    "prompt_tokens_details": {
                        "cached_tokens": 654
                    }
                }
            })
        );
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider.test.ts › google-vertex-xai-provider › should support HTTP image URLs
    #[tokio::test]
    async fn should_support_http_image_urls() {
        let fetch = MockFetch::new(vec![]);
        let provider = provider(&fetch, Some("global"), None);

        let supported_urls = provider
            .call("xai/grok-4.1-fast-reasoning")
            .supported_urls()
            .await;

        let patterns: Vec<(&str, Vec<&str>)> = supported_urls
            .0
            .iter()
            .map(|(media_type, patterns)| {
                (
                    media_type.as_str(),
                    patterns.iter().map(regex::Regex::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(patterns, vec![("image/*", vec![r"^https?://.*$"])]);
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider.test.ts › google-vertex-xai-provider › should throw NoSuchModelError for embedding and image models
    #[test]
    fn should_throw_no_such_model_error_for_embedding_and_image_models() {
        let provider = create_google_vertex_xai(VertexXaiProviderSettings {
            project: Some("test-project".into()),
            ..Default::default()
        })
        .unwrap();

        assert!(matches!(
            provider.embedding_model("invalid-model-id"),
            Err(AiMuxError::NoSuchModel { .. })
        ));
        assert!(matches!(
            provider.image_model("invalid-model-id"),
            Err(AiMuxError::NoSuchModel { .. })
        ));
    }
}
