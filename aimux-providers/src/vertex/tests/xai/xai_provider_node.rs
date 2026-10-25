//! Port of `@ai-sdk/google-vertex` 5.0.100
//! `src/xai/google-vertex-xai-provider-node.test.ts`.
//!
//! Upstream mocks the base provider and calls the fetch wrapper the Node
//! entry point hands it with a hand-made `(url, init)`. aimux has one entry
//! point, whose fetch layer adds the same headers: each case makes a chat
//! request through a mock transport, so the request URL is the provider's
//! (not `https://example.com/test`) and `init` is the request the
//! OpenAI-compatible model builds. Upstream's mocked token generator
//! (returning `mock-auth-token`) is the `generate_auth_token` setting, except
//! in the cases about the default generator, which run aimux's Google auth
//! over the mock transport.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::Resolvable;
use serde_json::json;

use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, RouteFetch, Seen};
use crate::vertex::{
    GoogleAuthOptions, GoogleAuthScopes, VertexXaiProviderSettings, create_google_vertex_xai,
    google_vertex_xai,
};

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

/// A token generator returning `mock-auth-token`, and its call counter.
fn mock_generator() -> (Resolvable<Option<String>>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let generator = Resolvable::from_async_fn(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        async { Ok(Some("mock-auth-token".to_string())) }
    });
    (generator, calls)
}

/// Settings of a provider for `test-project` over `fetch`, with the mocked
/// token generator.
fn settings(fetch: &Arc<MockFetch>) -> (VertexXaiProviderSettings, Arc<AtomicUsize>) {
    let (generator, calls) = mock_generator();
    let settings = VertexXaiProviderSettings {
        project: Some("test-project".into()),
        location: Some("global".into()),
        fetch: Some(fetch.transport()),
        generate_auth_token: Some(generator),
        ..Default::default()
    };
    (settings, calls)
}

/// Generate once with `options` and return the request the transport saw.
async fn request(
    settings: VertexXaiProviderSettings,
    fetch: &Arc<MockFetch>,
    options: &CallOptions,
) -> Seen {
    create_google_vertex_xai(settings)
        .unwrap()
        .call("xai/grok-4.1-fast-reasoning")
        .do_generate(options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    seen[0].clone()
}

/// The request headers except `skip`.
fn sent_headers(seen: &Seen, skip: &[&str]) -> BTreeMap<String, String> {
    seen.headers
        .iter()
        .filter(|(name, _)| !skip.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// No credential file or gcloud configuration and the default metadata
/// host: the default Google auth asks the metadata server for a token.
fn metadata_only_env() -> Vec<EnvVar> {
    vec![
        EnvVar::set("GOOGLE_APPLICATION_CREDENTIALS", None),
        EnvVar::set("google_application_credentials", None),
        EnvVar::set(
            "CLOUDSDK_CONFIG",
            Some("/nonexistent/aimux-vertex-test-gcloud"),
        ),
        EnvVar::set("GCE_METADATA_IP", None),
        EnvVar::set("GCE_METADATA_HOST", None),
    ]
}

const METADATA_TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";

/// The metadata server's token response.
fn metadata_token(token: &str) -> Canned {
    let mut canned = Canned::json(&json!({
        "access_token": token, "expires_in": 3600, "token_type": "Bearer"
    }));
    canned
        .headers
        .push(("Metadata-Flavor".into(), "Google".into()));
    canned
}

mod google_vertex_xai_provider_node {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider-node.test.ts › google-vertex-xai-provider-node › should generate auth token and add to request headers
    ///
    /// `createAuthTokenGenerator(undefined)` has no counterpart once the
    /// generator is replaced; the generator's single call is asserted.
    #[tokio::test]
    async fn should_generate_auth_token_and_add_to_request_headers() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let (settings, calls) = settings(&fetch);

        let seen = request(settings, &fetch, &prompt()).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(seen.method, "POST");
        assert_eq!(
            sent_headers(&seen, &["user-agent"]),
            headers(&[
                ("content-type", "application/json"),
                ("authorization", "Bearer mock-auth-token"),
            ])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider-node.test.ts › google-vertex-xai-provider-node › should pass googleAuthOptions to createAuthTokenGenerator
    ///
    /// The options reach aimux's Google auth (upstream mocks the generator):
    /// with no credentials it asks the metadata server for a token of the
    /// given scope, once.
    #[tokio::test]
    #[serial_test::serial]
    async fn should_pass_google_auth_options_to_create_auth_token_generator() {
        let _env = metadata_only_env();
        let fetch = RouteFetch::new(|seen| {
            if seen.url.contains(METADATA_TOKEN_PATH) {
                metadata_token("mock-auth-token")
            } else {
                chat_response()
            }
        });
        let provider = create_google_vertex_xai(VertexXaiProviderSettings {
            project: Some("test-project".into()),
            location: Some("global".into()),
            fetch: Some(fetch.transport()),
            google_auth_options: Some(GoogleAuthOptions {
                scopes: Some(GoogleAuthScopes::Array(vec!["test-scope".into()])),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();

        provider
            .call("xai/grok-4.1-fast-reasoning")
            .do_generate(&prompt())
            .await
            .unwrap();

        assert_eq!(fetch.count("GET", METADATA_TOKEN_PATH), 1);
        let seen = fetch.seen();
        let metadata = seen
            .iter()
            .find(|seen| seen.url.contains(METADATA_TOKEN_PATH))
            .unwrap();
        assert!(
            metadata.url.ends_with("?scopes=test-scope"),
            "{}",
            metadata.url
        );
        let chat = seen
            .iter()
            .find(|seen| seen.url.ends_with("/chat/completions"))
            .unwrap();
        assert_eq!(chat.headers["authorization"], "Bearer mock-auth-token");
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider-node.test.ts › google-vertex-xai-provider-node › should merge custom headers with auth header
    #[tokio::test]
    async fn should_merge_custom_headers_with_auth_header() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let (settings, _) = settings(&fetch);
        let settings = VertexXaiProviderSettings {
            headers: Some(Resolvable::Value(
                [("X-Custom".to_string(), Some("header-value".to_string()))].into(),
            )),
            ..settings
        };

        let seen = request(settings, &fetch, &prompt()).await;

        assert_eq!(
            sent_headers(&seen, &["user-agent"]),
            headers(&[
                ("content-type", "application/json"),
                ("x-custom", "header-value"),
                ("authorization", "Bearer mock-auth-token"),
            ])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider-node.test.ts › google-vertex-xai-provider-node › should use custom fetch when provided
    #[tokio::test]
    async fn should_use_custom_fetch_when_provided() {
        let custom_fetch = MockFetch::new(vec![chat_response()]);
        let (settings, _) = settings(&custom_fetch);

        let seen = request(settings, &custom_fetch, &prompt()).await;

        assert_eq!(
            sent_headers(&seen, &["content-type", "user-agent"]),
            headers(&[("authorization", "Bearer mock-auth-token")])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/xai/google-vertex-xai-provider-node.test.ts › google-vertex-xai-provider-node › should export default googleVertexXai instance
    #[test]
    fn should_export_default_google_vertex_xai_instance() {
        let model = google_vertex_xai().call("xai/grok-4.1-fast-reasoning");
        assert_eq!(model.provider(), "googleVertex.xai.chat");
    }
}
