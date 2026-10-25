//! Port of `@ai-sdk/google-vertex` 5.0.100
//! `src/anthropic/google-vertex-anthropic-provider.test.ts`.
//!
//! Upstream mocks the `AnthropicLanguageModel` constructor and inspects the
//! config handed to it. aimux builds the model itself, so each config value
//! is observed through what it does: `baseURL` and `buildRequestUrl` through
//! the request URL, `transformRequestBody` through the request body, `headers`
//! through the request headers, `supportedUrls` through
//! `LanguageModel::supported_urls`. Requests need a token, which the
//! `generate_auth_token` setting supplies (upstream's base provider has no
//! auth); the transport is a mock (upstream's `fetch` is `undefined`).

use std::collections::BTreeMap;
use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_provider_utils::Resolvable;
use serde_json::json;

use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use crate::vertex::{
    VertexAnthropicProvider, VertexAnthropicProviderSettings, create_google_vertex_anthropic,
};

/// A minimal Messages API response.
fn messages_response() -> Canned {
    Canned::json(&json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))
}

fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// Settings with the mock transport and a fixed token.
fn settings(fetch: &Arc<MockFetch>) -> VertexAnthropicProviderSettings {
    VertexAnthropicProviderSettings {
        fetch: Some(fetch.transport()),
        generate_auth_token: Some(Resolvable::Value(Some("test-token".into()))),
        ..Default::default()
    }
}

/// Generate once with `model_id` and return the request the transport saw.
async fn request(
    provider: &VertexAnthropicProvider,
    fetch: &Arc<MockFetch>,
    model_id: &str,
) -> Seen {
    provider
        .call(model_id)
        .do_generate(&prompt())
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    seen[0].clone()
}

/// The request headers other than the token (supplied by the harness) and
/// the transport's own `content-type` and `user-agent`: upstream's `headers`
/// config value.
fn provider_headers(seen: &Seen) -> BTreeMap<String, String> {
    seen.headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "authorization" | "content-type" | "user-agent"
            )
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

mod google_vertex_anthropic_provider {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should create a language model with default settings
    #[tokio::test]
    async fn should_create_a_language_model_with_default_settings() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("test-location".into()),
            ..settings(&fetch)
        })
        .unwrap();
        let model = provider.call("test-model-id");
        assert_eq!(model.model_id(), "test-model-id");
        assert_eq!(model.provider(), "googleVertex.anthropic.messages");

        let seen = request(&provider, &fetch, "test-model-id").await;
        // baseURL
        assert!(
            seen.url.contains(
                "/projects/test-project/locations/test-location/publishers/anthropic/models"
            ),
            "{}",
            seen.url
        );
        // buildRequestUrl
        assert!(
            seen.url.ends_with("/test-model-id:rawPredict"),
            "{}",
            seen.url
        );
        // transformRequestBody
        let body = seen.json_body();
        assert!(body.get("model").is_none(), "{body}");
        assert_eq!(body["anthropic_version"], "vertex-2023-10-16");
        // headers: an object (only the token here)
        assert_eq!(seen.headers["authorization"], "Bearer test-token");
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should pass baseURL to the model when created
    #[tokio::test]
    async fn should_pass_base_url_to_the_model_when_created() {
        let custom_base_url = "https://custom-url.com";
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            base_url: Some(custom_base_url.into()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model-id").await;
        assert_eq!(
            seen.url,
            format!("{custom_base_url}/test-model-id:rawPredict")
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should throw NoSuchModelError for textEmbeddingModel
    #[test]
    fn should_throw_no_such_model_error_for_text_embedding_model() {
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            ..Default::default()
        })
        .unwrap();

        assert!(matches!(
            provider.embedding_model("invalid-model-id"),
            Err(AiMuxError::NoSuchModel { .. })
        ));
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should pass custom headers to the model constructor
    #[tokio::test]
    async fn should_pass_custom_headers_to_the_model_constructor() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-east5".into()),
            headers: Some(Resolvable::Value(
                [(
                    "Custom-Header".to_string(),
                    Some("custom-value".to_string()),
                )]
                .into(),
            )),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model-id").await;
        assert_eq!(
            provider_headers(&seen),
            BTreeMap::from([("custom-header".to_string(), "custom-value".to_string())])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should create a Google Vertex Anthropic provider instance with custom settings
    #[test]
    fn should_create_a_google_vertex_anthropic_provider_instance_with_custom_settings() {
        let custom_provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("custom-project".into()),
            location: Some("custom-location".into()),
            base_url: Some("https://custom.base.url".into()),
            headers: Some(Resolvable::Value(
                [("Custom-Header".to_string(), Some("value".to_string()))].into(),
            )),
            ..Default::default()
        });

        let custom_provider = custom_provider.unwrap();
        // Callable (`call`), with a language model factory.
        let _ = custom_provider.call("test-model-id");
        assert!(custom_provider.language_model("test-model-id").is_ok());
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should not support URL sources to force base64 conversion
    #[tokio::test]
    async fn should_not_support_url_sources_to_force_base64_conversion() {
        let provider =
            create_google_vertex_anthropic(VertexAnthropicProviderSettings::default()).unwrap();
        let model = provider.call("test-model-id");
        assert_eq!(model.model_id(), "test-model-id");

        assert!(model.supported_urls().await.0.is_empty());
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should use correct URL for global location
    #[tokio::test]
    async fn should_use_correct_url_for_global_location() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("global".into()),
            ..settings(&fetch)
        })
        .unwrap();
        assert_eq!(
            provider.call("test-model-id").provider(),
            "googleVertex.anthropic.messages"
        );

        let seen = request(&provider, &fetch, "test-model-id").await;
        assert_eq!(
            seen.url,
            "https://aiplatform.googleapis.com/v1/projects/test-project/locations/global/publishers/anthropic/models/test-model-id:rawPredict"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should use region-prefixed URL for non-global locations
    #[tokio::test]
    async fn should_use_region_prefixed_url_for_non_global_locations() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-east5".into()),
            ..settings(&fetch)
        })
        .unwrap();
        assert_eq!(
            provider.call("test-model-id").provider(),
            "googleVertex.anthropic.messages"
        );

        let seen = request(&provider, &fetch, "test-model-id").await;
        assert_eq!(
            seen.url,
            "https://us-east5-aiplatform.googleapis.com/v1/projects/test-project/locations/us-east5/publishers/anthropic/models/test-model-id:rawPredict"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should use multi-region URL for eu location
    #[tokio::test]
    async fn should_use_multi_region_url_for_eu_location() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("eu".into()),
            ..settings(&fetch)
        })
        .unwrap();
        let model = provider.call("test-model-id");
        assert_eq!(model.provider(), "googleVertex.anthropic.messages");
        assert!(model.supported_urls().await.0.is_empty());

        let seen = request(&provider, &fetch, "test-model-id").await;
        assert_eq!(
            seen.url,
            "https://aiplatform.eu.rep.googleapis.com/v1/projects/test-project/locations/eu/publishers/anthropic/models/test-model-id:rawPredict"
        );
        assert_eq!(provider_headers(&seen), BTreeMap::new());
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should use multi-region URL for us location
    #[tokio::test]
    async fn should_use_multi_region_url_for_us_location() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("us".into()),
            ..settings(&fetch)
        })
        .unwrap();
        let model = provider.call("test-model-id");
        assert_eq!(model.provider(), "googleVertex.anthropic.messages");
        assert!(model.supported_urls().await.0.is_empty());

        let seen = request(&provider, &fetch, "test-model-id").await;
        assert_eq!(
            seen.url,
            "https://aiplatform.us.rep.googleapis.com/v1/projects/test-project/locations/us/publishers/anthropic/models/test-model-id:rawPredict"
        );
        assert_eq!(provider_headers(&seen), BTreeMap::new());
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider.test.ts › google-vertex-anthropic-provider › should support combining tools with structured outputs (inherited from Anthropic)
    #[test]
    fn should_support_combining_tools_with_structured_outputs_inherited_from_anthropic() {
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-east5".into()),
            ..Default::default()
        })
        .unwrap();

        let model = provider.call("claude-3-5-sonnet-v2@20241022");

        assert_eq!(model.model_id(), "claude-3-5-sonnet-v2@20241022");
        assert_eq!(model.provider(), "googleVertex.anthropic.messages");
    }
}
