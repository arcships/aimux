//! Port of `@ai-sdk/google-vertex` 5.0.100
//! `src/maas/google-vertex-maas-provider.test.ts`.
//!
//! Upstream mocks `createOpenAICompatible` and inspects the settings handed
//! to it. aimux builds the OpenAI-compatible provider itself, so each setting
//! is observed through a request over a mock transport: `baseURL` through the
//! request URL (`{baseURL}/chat/completions`), `name` through
//! `LanguageModel::provider` (`{name}.chat`), `transformRequestBody` through
//! the request body. The transport is a mock (upstream's `fetch` is
//! `undefined`) and the token comes from the `generate_auth_token` setting
//! (upstream's base provider has no auth).

use std::sync::Arc;

use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::Resolvable;
use serde_json::json;

use crate::openai_compatible::internal::OpenAICompatibleProvider;
use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, Seen};
use crate::vertex::{VertexMaasProviderSettings, create_google_vertex_maas};

/// A minimal Chat Completions response.
fn chat_response() -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1}
    }))
}

fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// Settings with the mock transport and a fixed token.
fn settings(fetch: &Arc<MockFetch>) -> VertexMaasProviderSettings {
    VertexMaasProviderSettings {
        fetch: Some(fetch.transport()),
        generate_auth_token: Some(Resolvable::Value(Some("test-token".into()))),
        ..Default::default()
    }
}

/// Generate once with `model_id` and return the request the transport saw.
async fn request(
    provider: &OpenAICompatibleProvider,
    fetch: &Arc<MockFetch>,
    model_id: &str,
    options: &CallOptions,
) -> Seen {
    let model = provider.call(model_id);
    assert_eq!(model.provider(), "vertex.maas.chat");
    model.do_generate(options).await.unwrap();
    fetch.seen().last().unwrap().clone()
}

mod google_vertex_maas_provider {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should create a provider with correct base URL for global location
    #[tokio::test]
    async fn should_create_a_provider_with_correct_base_url_for_global_location() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            location: Some("global".into()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model", &prompt()).await;
        assert_eq!(
            seen.url,
            "https://aiplatform.googleapis.com/v1/projects/test-project/locations/global/endpoints/openapi/chat/completions"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should create a provider with correct base URL for regional location
    #[tokio::test]
    async fn should_create_a_provider_with_correct_base_url_for_regional_location() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-central1".into()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model", &prompt()).await;
        assert_eq!(
            seen.url,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/test-project/locations/us-central1/endpoints/openapi/chat/completions"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should create a provider with correct base URL for multi-region location
    #[tokio::test]
    async fn should_create_a_provider_with_correct_base_url_for_multi_region_location() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            location: Some("eu".into()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model", &prompt()).await;
        assert_eq!(
            seen.url,
            "https://aiplatform.eu.rep.googleapis.com/v1/projects/test-project/locations/eu/endpoints/openapi/chat/completions"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should default to global location when not specified
    ///
    /// Upstream's mocked `loadOptionalSetting` ignores the environment; here
    /// `GOOGLE_VERTEX_LOCATION` is unset for the test.
    #[tokio::test]
    #[serial_test::serial]
    async fn should_default_to_global_location_when_not_specified() {
        let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", None);
        let fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model", &prompt()).await;
        assert_eq!(
            seen.url,
            "https://aiplatform.googleapis.com/v1/projects/test-project/locations/global/endpoints/openapi/chat/completions"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should use custom baseURL when provided
    #[tokio::test]
    async fn should_use_custom_base_url_when_provided() {
        let custom_base_url = "https://custom-endpoint.example.com";
        let fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            base_url: Some(custom_base_url.into()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model", &prompt()).await;
        assert_eq!(seen.url, format!("{custom_base_url}/chat/completions"));
    }

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should pass custom fetch to openai-compatible provider
    #[tokio::test]
    async fn should_pass_custom_fetch_to_openai_compatible_provider() {
        let custom_fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            location: Some("global".into()),
            ..settings(&custom_fetch)
        })
        .unwrap();

        request(&provider, &custom_fetch, "test-model", &prompt()).await;
        assert_eq!(custom_fetch.seen().len(), 1);
    }

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should default Llama 4 max tokens without overriding explicit or other model settings
    ///
    /// Upstream calls `transformRequestBody` on three bodies; here each body
    /// is the one a request with that model and `max_tokens` builds.
    #[tokio::test]
    async fn should_default_llama_4_max_tokens_without_overriding_explicit_or_other_settings() {
        let fetch = MockFetch::new(vec![chat_response(), chat_response(), chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            location: Some("global".into()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(
            &provider,
            &fetch,
            "meta/llama-4-scout-17b-16e-instruct-maas",
            &prompt(),
        )
        .await;
        let body = seen.json_body();
        assert_eq!(body["model"], "meta/llama-4-scout-17b-16e-instruct-maas");
        assert_eq!(body["max_tokens"], 8192);

        let mut explicit = prompt();
        explicit.max_output_tokens = Some(64);
        let seen = request(
            &provider,
            &fetch,
            "meta/llama-4-maverick-17b-128e-instruct-maas",
            &explicit,
        )
        .await;
        let body = seen.json_body();
        assert_eq!(
            body["model"],
            "meta/llama-4-maverick-17b-128e-instruct-maas"
        );
        assert_eq!(body["max_tokens"], 64);

        let seen = request(&provider, &fetch, "openai/gpt-oss-20b-maas", &prompt()).await;
        let body = seen.json_body();
        assert_eq!(body["model"], "openai/gpt-oss-20b-maas");
        assert!(body.get("max_tokens").is_none(), "{body}");
    }

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should construct correct URL with trailing slash removed from baseURL
    #[tokio::test]
    async fn should_construct_correct_url_with_trailing_slash_removed_from_base_url() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            base_url: Some("https://custom-endpoint.example.com/".into()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model", &prompt()).await;
        assert_eq!(
            seen.url,
            "https://custom-endpoint.example.com/chat/completions"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/maas/google-vertex-maas-provider.test.ts › google-vertex-maas-provider › should construct correct URL when baseURL is empty string
    #[tokio::test]
    async fn should_construct_correct_url_when_base_url_is_empty_string() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-central1".into()),
            base_url: Some(String::new()),
            ..settings(&fetch)
        })
        .unwrap();

        let seen = request(&provider, &fetch, "test-model", &prompt()).await;
        assert_eq!(
            seen.url,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/test-project/locations/us-central1/endpoints/openapi/chat/completions"
        );
    }
}
