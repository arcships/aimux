//! Port of `@ai-sdk/google-vertex` 5.0.100
//! `src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts`.
//!
//! aimux has one entry point and no separate Web-Crypto edge auth; these
//! cases exercise the same behaviour through it. Upstream mocks the base
//! provider and resolves the `headers` function the edge wrapper hands it;
//! here each case makes a request through a mock transport and reads the
//! request's headers. Upstream's mocked `generateAuthToken` (returning
//! `mock-auth-token`) is replaced by the `generate_auth_token` setting,
//! except in the cases about the default generator and `googleCredentials`
//! (aimux: `GoogleAuthOptions::credentials`), which run aimux's Google auth.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use serde_json::json;

use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, RouteFetch, Seen};
use crate::vertex::{
    GoogleAuthOptions, VertexAnthropicProviderSettings, create_google_vertex_anthropic,
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
mod google_vertex_anthropic_provider_edge {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › default headers function should return auth token
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

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › should use custom headers in addition to auth token when provided
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

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › should use edge auth token generator
    ///
    /// Without `generate_auth_token` the default generator makes the token:
    /// here aimux's Google auth, from the metadata server.
    #[tokio::test]
    #[serial_test::serial]
    async fn should_use_edge_auth_token_generator() {
        let _env = metadata_only_env();
        let fetch = RouteFetch::new(|seen| {
            if seen
                .url
                .contains("/computeMetadata/v1/instance/service-accounts/default/token")
            {
                metadata_token("mock-auth-token")
            } else {
                messages_response()
            }
        });
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-east5".into()),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        provider
            .call("test-model-id")
            .do_generate(&prompt())
            .await
            .unwrap();

        assert_eq!(
            fetch.count(
                "GET",
                "/computeMetadata/v1/instance/service-accounts/default/token"
            ),
            1
        );
        let seen = fetch.seen();
        let model_request = seen
            .iter()
            .find(|seen| seen.url.contains(":rawPredict"))
            .unwrap();
        assert_eq!(
            provider_headers(model_request),
            headers(&[("authorization", "Bearer mock-auth-token")])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › passes googleCredentials to generateAuthToken
    ///
    /// The credentials reach aimux's real Google auth (upstream mocks the
    /// generator), which signs with the given private key: `test-key` is not
    /// a key, so the request fails on it before anything is sent.
    #[tokio::test]
    async fn passes_google_credentials_to_generate_auth_token() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            google_auth_options: Some(GoogleAuthOptions {
                credentials: Some(json!({
                    "client_email": "test@example.com",
                    "private_key": "test-key"
                })),
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

        assert!(error.to_string().contains("private key"), "{error}");
        assert!(fetch.seen().is_empty());
    }

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › uses custom generateAuthToken when provided and skips the default
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

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › merges custom generateAuthToken with user-provided headers
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

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › invokes custom generateAuthToken on each headers resolution
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

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › propagates errors thrown from custom generateAuthToken
    #[tokio::test]
    async fn propagates_errors_thrown_from_custom_generate_auth_token() {
        let fetch = MockFetch::new(vec![messages_response()]);
        let provider = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            generate_auth_token: Some(Resolvable::from_async_fn(|| async {
                Err(AiMuxError::Other("token mint failed".into()))
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

    /// upstream: @ai-sdk/google-vertex src/anthropic/edge/google-vertex-anthropic-provider-edge.test.ts › google-vertex-anthropic-provider-edge › user-provided Authorization in headers overrides the generated token
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
}
