//! Port of `@ai-sdk/google-vertex` 5.0.100
//! `src/maas/edge/google-vertex-maas-provider-edge.test.ts`.
//!
//! aimux has one entry point and no separate Web-Crypto edge auth; these
//! cases exercise the same behaviour through it. Upstream mocks the base
//! provider and calls the fetch wrapper the edge entry point hands it with a
//! hand-made `(url, init)`; here each case makes a chat request through a
//! mock transport, so the request URL is the provider's (not
//! `https://example.com/test`) and `init` is the request the
//! OpenAI-compatible model builds. Upstream's mocked `generateAuthToken`
//! (returning `mock-auth-token`) is the `generate_auth_token` setting, except
//! for `googleCredentials` (aimux: `GoogleAuthOptions::credentials`), which
//! runs aimux's Google auth.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use serde_json::json;

use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use crate::vertex::{
    GoogleAuthOptions, VertexMaasProviderSettings, create_google_vertex_maas, google_vertex_maas,
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
fn settings(fetch: &Arc<MockFetch>) -> (VertexMaasProviderSettings, Arc<AtomicUsize>) {
    let (generator, calls) = mock_generator();
    let settings = VertexMaasProviderSettings {
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
    settings: VertexMaasProviderSettings,
    fetch: &Arc<MockFetch>,
    options: &CallOptions,
) -> Seen {
    create_google_vertex_maas(settings)
        .unwrap()
        .call("test-model")
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

mod google_vertex_maas_provider_edge {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/maas/edge/google-vertex-maas-provider-edge.test.ts › google-vertex-maas-provider-edge › should generate auth token and add to request headers
    ///
    /// The `undefined` credentials argument has no counterpart once the
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

    /// upstream: @ai-sdk/google-vertex src/maas/edge/google-vertex-maas-provider-edge.test.ts › google-vertex-maas-provider-edge › should pass googleCredentials to generateAuthToken
    ///
    /// The credentials reach aimux's real Google auth (upstream mocks the
    /// generator), which signs with the given private key: `test-key` is not
    /// a key, so the request fails on it before anything is sent.
    #[tokio::test]
    async fn should_pass_google_credentials_to_generate_auth_token() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let provider = create_google_vertex_maas(VertexMaasProviderSettings {
            project: Some("test-project".into()),
            location: Some("global".into()),
            fetch: Some(fetch.transport()),
            google_auth_options: Some(GoogleAuthOptions {
                credentials: Some(json!({
                    "client_email": "test@example.com",
                    "private_key": "test-key"
                })),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();

        let error = provider
            .call("test-model")
            .do_generate(&prompt())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("private key"), "{error}");
        assert!(fetch.seen().is_empty());
    }

    /// upstream: @ai-sdk/google-vertex src/maas/edge/google-vertex-maas-provider-edge.test.ts › google-vertex-maas-provider-edge › should merge custom headers with auth header
    #[tokio::test]
    async fn should_merge_custom_headers_with_auth_header() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let (settings, _) = settings(&fetch);
        let settings = VertexMaasProviderSettings {
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

    /// upstream: @ai-sdk/google-vertex src/maas/edge/google-vertex-maas-provider-edge.test.ts › google-vertex-maas-provider-edge › should use custom fetch when provided
    #[tokio::test]
    async fn should_use_custom_fetch_when_provided() {
        let custom_fetch = MockFetch::new(vec![chat_response()]);
        let (settings, _) = settings(&custom_fetch);

        let seen = request(settings, &custom_fetch, &prompt()).await;

        assert_eq!(seen.headers["authorization"], "Bearer mock-auth-token");
    }

    /// upstream: @ai-sdk/google-vertex src/maas/edge/google-vertex-maas-provider-edge.test.ts › google-vertex-maas-provider-edge › should handle async custom headers
    #[tokio::test]
    async fn should_handle_async_custom_headers() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let (settings, _) = settings(&fetch);
        let async_headers = Resolvable::from_future(async {
            Ok::<HeaderMapOpt, _>([("X-Async".to_string(), Some("async-value".to_string()))].into())
        });
        let settings = VertexMaasProviderSettings {
            headers: Some(async_headers),
            ..settings
        };

        let seen = request(settings, &fetch, &prompt()).await;

        assert_eq!(seen.headers["x-async"], "async-value");
        assert_eq!(seen.headers["authorization"], "Bearer mock-auth-token");
    }

    /// upstream: @ai-sdk/google-vertex src/maas/edge/google-vertex-maas-provider-edge.test.ts › google-vertex-maas-provider-edge › should preserve headers from init parameter
    ///
    /// `init`'s headers are the per-call headers here; the request also
    /// carries the model's `content-type`, and `postToApi` appends the
    /// pinned package parts to the per-call `User-Agent`.
    #[tokio::test]
    async fn should_preserve_headers_from_init_parameter() {
        let fetch = MockFetch::new(vec![chat_response()]);
        let (settings, _) = settings(&fetch);
        let mut options = prompt();
        options.headers = Some(
            [("User-Agent", "test-agent"), ("X-Request-ID", "test-id")]
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        );

        let seen = request(settings, &fetch, &options).await;

        assert_eq!(
            sent_headers(&seen, &["content-type", "user-agent"]),
            headers(&[
                ("x-request-id", "test-id"),
                ("authorization", "Bearer mock-auth-token"),
            ])
        );
        assert!(
            seen.headers["user-agent"].starts_with("test-agent"),
            "{}",
            seen.headers["user-agent"]
        );
    }

    /// upstream: @ai-sdk/google-vertex src/maas/edge/google-vertex-maas-provider-edge.test.ts › google-vertex-maas-provider-edge › should export default googleVertexMaas instance
    #[test]
    fn should_export_default_google_vertex_maas_instance() {
        let model = google_vertex_maas().call("test-model");
        assert_eq!(model.provider(), "vertex.maas.chat");
    }
}
