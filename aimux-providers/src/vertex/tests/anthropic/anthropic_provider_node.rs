//! Port of `@ai-sdk/google-vertex` 5.0.100
//! `src/anthropic/google-vertex-anthropic-provider-node.test.ts`.
//!
//! Upstream mocks the base provider and resolves the `headers` function the
//! Node wrapper hands it. aimux has one entry point, whose provider headers
//! are those of every request: each case makes a request through a mock
//! transport and reads the request's headers. Upstream's mocked
//! `createAuthTokenGenerator` (returning `mock-auth-token`) is replaced by the
//! `generate_auth_token` setting, except in the cases about the default
//! generator, which run Google auth with inline credentials over the mock.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use serde_json::json;

use crate::test_support::mock_fetch::{Canned, MockFetch, RouteFetch, Seen};
use crate::vertex::{
    GoogleAuthOptions, GoogleAuthScopes, VertexAnthropicProviderSettings,
    create_google_vertex_anthropic,
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

/// Settings of a provider for `test-project` over `fetch`.
fn settings(fetch: &Arc<MockFetch>) -> VertexAnthropicProviderSettings {
    VertexAnthropicProviderSettings {
        project: Some("test-project".into()),
        location: Some("us-east5".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    }
}

/// A token generator returning `token(n)` on its n-th call (from 1), and its
/// call counter.
fn counting_generator(
    token: impl Fn(usize) -> Option<String> + Send + Sync + 'static,
) -> (Resolvable<Option<String>>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let token = Arc::new(token);
    let generator = Resolvable::from_async_fn(move || {
        let call = counter.fetch_add(1, Ordering::SeqCst) + 1;
        let token = token.clone();
        async move { Ok(token(call)) }
    });
    (generator, calls)
}

/// Async caller headers, like upstream's `async () => ({ ... })`.
fn async_headers(headers: &[(&str, &str)]) -> Resolvable<HeaderMapOpt> {
    let headers: HeaderMapOpt = headers
        .iter()
        .map(|(name, value)| (name.to_string(), Some(value.to_string())))
        .collect();
    Resolvable::from_async_fn(move || {
        let headers = headers.clone();
        async move { Ok(headers) }
    })
}

/// The headers the provider adds to a request: everything but the
/// transport's own `content-type` and `user-agent` (upstream resolves the
/// `headers` function alone).
fn provider_headers(seen: &Seen) -> BTreeMap<String, String> {
    seen.headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "content-type" | "user-agent"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// Inline authorized-user credentials for the default Google auth.
fn authorized_user() -> serde_json::Value {
    json!({
        "type": "authorized_user",
        "client_id": "client-id",
        "client_secret": "client-secret",
        "refresh_token": "refresh-token"
    })
}

mod google_vertex_anthropic_provider_node {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › default headers function should return auth token
    #[tokio::test]
    async fn default_headers_function_should_return_auth_token() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            generate_auth_token: Some(Resolvable::Value(Some("mock-auth-token".into()))),
            ..settings(&fetch)
        })
        .unwrap();

        provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap();

        assert_eq!(
            provider_headers(&fetch.seen()[0]),
            headers(&[("authorization", "Bearer mock-auth-token")])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › should use custom headers in addition to auth token when provided
    #[tokio::test]
    async fn should_use_custom_headers_in_addition_to_auth_token_when_provided() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            headers: Some(async_headers(&[("Custom-Header", "custom-value")])),
            generate_auth_token: Some(Resolvable::Value(Some("mock-auth-token".into()))),
            ..settings(&fetch)
        })
        .unwrap();

        provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap();

        assert_eq!(
            provider_headers(&fetch.seen()[0]),
            headers(&[
                ("authorization", "Bearer mock-auth-token"),
                ("custom-header", "custom-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › passes googleAuthOptions to createAuthTokenGenerator
    ///
    /// No seam replaces the generator here, so the options reach the real
    /// Google auth: it reads the given (missing) key file, and no request is
    /// sent.
    #[tokio::test]
    async fn passes_google_auth_options_to_create_auth_token_generator() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            google_auth_options: Some(GoogleAuthOptions {
                scopes: Some(GoogleAuthScopes::Array(vec![
                    "https://www.googleapis.com/auth/cloud-platform".into(),
                ])),
                key_file: Some("path/to/key.json".into()),
                ..Default::default()
            }),
            ..settings(&fetch)
        })
        .unwrap();

        let error = provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("Could not read Google application credentials"),
            "{error}"
        );
        assert!(fetch.seen().is_empty());
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › uses custom generateAuthToken when provided and skips the default
    #[tokio::test]
    async fn uses_custom_generate_auth_token_when_provided_and_skips_the_default() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let (custom_generate, calls) = counting_generator(|_| Some("custom-token".into()));
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            generate_auth_token: Some(custom_generate),
            ..settings(&fetch)
        })
        .unwrap();

        provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap();

        let seen = fetch.seen();
        assert_eq!(
            provider_headers(&seen[0]),
            headers(&[("authorization", "Bearer custom-token")])
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // The default generator made no token request: the model request is
        // the only one.
        assert_eq!(seen.len(), 1);
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › merges custom generateAuthToken with user-provided headers
    #[tokio::test]
    async fn merges_custom_generate_auth_token_with_user_provided_headers() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let (custom_generate, _) = counting_generator(|_| Some("custom-token".into()));
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            generate_auth_token: Some(custom_generate),
            headers: Some(async_headers(&[("Custom-Header", "custom-value")])),
            ..settings(&fetch)
        })
        .unwrap();

        provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap();

        assert_eq!(
            provider_headers(&fetch.seen()[0]),
            headers(&[
                ("authorization", "Bearer custom-token"),
                ("custom-header", "custom-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › invokes custom generateAuthToken on each headers resolution
    #[tokio::test]
    async fn invokes_custom_generate_auth_token_on_each_headers_resolution() {
        let fetch = MockFetch::new(vec![messages_response(), messages_response()]);
        let (custom_generate, calls) = counting_generator(|call| Some(format!("token-{call}")));
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            generate_auth_token: Some(custom_generate),
            ..settings(&fetch)
        })
        .unwrap();
        let model = provider.call("test-model-id");

        model.do_generate(&prompt()).await.unwrap();
        model.do_generate(&prompt()).await.unwrap();

        let seen = fetch.seen();
        assert_eq!(
            provider_headers(&seen[0]),
            headers(&[("authorization", "Bearer token-1")])
        );
        assert_eq!(
            provider_headers(&seen[1]),
            headers(&[("authorization", "Bearer token-2")])
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › propagates errors thrown from custom generateAuthToken
    #[tokio::test]
    async fn propagates_errors_thrown_from_custom_generate_auth_token() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            generate_auth_token: Some(Resolvable::from_async_fn(|| async {
                Err(AiMuxError::other("token mint failed"))
            })),
            ..settings(&fetch)
        })
        .unwrap();

        let error = provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("token mint failed"), "{error}");
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › user-provided Authorization in headers overrides the generated token
    #[tokio::test]
    async fn user_provided_authorization_in_headers_overrides_the_generated_token() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let (custom_generate, _) = counting_generator(|_| Some("custom-token".into()));
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            generate_auth_token: Some(custom_generate),
            headers: Some(async_headers(&[("Authorization", "Bearer user-override")])),
            ..settings(&fetch)
        })
        .unwrap();

        provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap();

        assert_eq!(
            provider_headers(&fetch.seen()[0]),
            headers(&[("authorization", "Bearer user-override")])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › accepts a generateAuthToken that returns null (matches default signature)
    #[tokio::test]
    async fn accepts_a_generate_auth_token_that_returns_null_matches_default_signature() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let (custom_generate, _) = counting_generator(|_| None);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            generate_auth_token: Some(custom_generate),
            ..settings(&fetch)
        })
        .unwrap();

        provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap();

        assert_eq!(
            provider_headers(&fetch.seen()[0]),
            headers(&[("authorization", "Bearer null")])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/google-vertex-anthropic-provider-node.test.ts › google-vertex-anthropic-provider-node › creates the auth token generator once per provider instance
    ///
    /// The generator is aimux's Google auth of the provider. It is one
    /// instance per provider when its token, cached by the instance, serves
    /// both requests: one token exchange for two requests.
    #[tokio::test]
    async fn creates_the_auth_token_generator_once_per_provider_instance() {
        let fetch = RouteFetch::new(|seen| {
            if seen.url.starts_with("https://oauth2.googleapis.com/token") {
                Canned::json(&json!({
                    "access_token": "mock-auth-token", "expires_in": 3600, "token_type": "Bearer"
                }))
            } else {
                messages_response()
            }
        });
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-east5".into()),
            fetch: Some(fetch.transport()),
            google_auth_options: Some(GoogleAuthOptions {
                credentials: Some(authorized_user()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
        let model = provider.call("test-model-id");

        model.do_generate(&prompt()).await.unwrap();
        model.do_generate(&prompt()).await.unwrap();

        assert_eq!(fetch.count("POST", "/token"), 1);
        let model_requests: Vec<_> = fetch
            .seen()
            .into_iter()
            .filter(|seen| seen.url.contains(":rawPredict"))
            .collect();
        assert_eq!(model_requests.len(), 2);
        for seen in model_requests {
            assert_eq!(seen.headers["authorization"], "Bearer mock-auth-token");
        }
    }
}
