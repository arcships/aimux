//! Port of `@ai-sdk/google-vertex` 5.0.100 `src/edge/google-vertex-provider-edge.test.ts`.
//!
//! The edge provider wraps the base provider with the Web-Crypto token
//! generator (`generateAuthToken`). aimux has one provider and one entry
//! point; the same behaviour goes through it: an `access_token` producer
//! stands in for the mocked `generateAuthToken`, `google_auth_options.credentials`
//! (a service-account JSON) carries upstream's `googleCredentials`, and the
//! tests read the provider headers its resolver produces for a request
//! (upstream's `passedOptions.headers`). Every test clears
//! `GOOGLE_VERTEX_API_KEY` (upstream's `beforeEach`) and
//! `GOOGLE_VERTEX_ACCESS_TOKEN` (read before the generator), so all of them
//! are serial.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::FutureExt;
use serde_json::json;
use serial_test::serial;

use crate::test_support::mock_fetch::EnvVar;
use crate::vertex::{
    GoogleAuthOptions, VertexProvider, VertexProviderSettings, create_google_vertex,
};
use aimux_provider_utils::{HeaderMapOpt, Resolvable};

fn clean_environment() -> Vec<EnvVar> {
    ["GOOGLE_VERTEX_API_KEY", "GOOGLE_VERTEX_ACCESS_TOKEN"]
        .into_iter()
        .map(|name| EnvVar::set(name, None))
        .collect()
}

/// `generateAuthToken: vi.fn().mockResolvedValue('mock-auth-token')`,
/// counting its calls.
fn mock_generate_auth_token(calls: &Arc<AtomicUsize>) -> Resolvable<String> {
    let calls = calls.clone();
    Resolvable::AsyncFn(Arc::new(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok("mock-auth-token".to_string()) }.boxed()
    }))
}

fn create(settings: VertexProviderSettings) -> VertexProvider {
    create_google_vertex(settings).expect("valid settings")
}

/// `await resolve(passedOptions.headers)`: the provider headers of one
/// request, in the mode the provider was created in.
async fn provider_headers(provider: &VertexProvider) -> HeaderMapOpt {
    let key = provider.resolver.express_key().await.expect("api key");
    provider
        .resolver
        .headers(key.as_deref())
        .await
        .expect("headers")
}

/// Expected provider headers. aimux combines header maps with lower-cased
/// names (HTTP names are case-insensitive), so the names are lower-cased.
fn headers(entries: &[(&str, &str)]) -> HeaderMapOpt {
    entries
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), Some((*value).to_string())))
        .collect()
}

mod google_vertex_provider_edge {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/edge/google-vertex-provider-edge.test.ts › google-vertex-provider-edge › default headers function should return auth token
    #[tokio::test]
    #[serial]
    async fn default_headers_function_should_return_auth_token() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            access_token: Some(mock_generate_auth_token(&calls)),
            ..Default::default()
        });

        assert_eq!(
            provider_headers(&provider).await,
            headers(&[("Authorization", "Bearer mock-auth-token")])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/edge/google-vertex-provider-edge.test.ts › google-vertex-provider-edge › should use custom headers in addition to auth token when provided
    #[tokio::test]
    #[serial]
    async fn should_use_custom_headers_in_addition_to_auth_token_when_provided() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            headers: Some(Resolvable::AsyncFn(Arc::new(|| {
                async { Ok(headers(&[("Custom-Header", "custom-value")])) }.boxed()
            }))),
            access_token: Some(mock_generate_auth_token(&calls)),
            ..Default::default()
        });

        assert_eq!(
            provider_headers(&provider).await,
            headers(&[
                ("Authorization", "Bearer mock-auth-token"),
                ("Custom-Header", "custom-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/edge/google-vertex-provider-edge.test.ts › google-vertex-provider-edge › should use edge auth token generator
    #[tokio::test]
    #[serial]
    async fn should_use_edge_auth_token_generator() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            access_token: Some(mock_generate_auth_token(&calls)),
            ..Default::default()
        });

        provider_headers(&provider).await;
        assert!(calls.load(Ordering::SeqCst) > 0);
    }

    /// upstream: @ai-sdk/google-vertex src/edge/google-vertex-provider-edge.test.ts › google-vertex-provider-edge › passes googleCredentials to generateAuthToken
    #[tokio::test]
    #[serial]
    async fn passes_google_credentials_to_generate_auth_token() {
        let _env = clean_environment();
        // `googleCredentials: { clientEmail, privateKey }` is a
        // service-account credential JSON in aimux. The credential resolver
        // is built with the provider, so no headers call is needed (and
        // 'test-key' is no RSA key).
        let credentials = json!({
            "client_email": "test@example.com",
            "private_key": "test-key",
        });
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            google_auth_options: Some(GoogleAuthOptions {
                credentials: Some(credentials.clone()),
                ..Default::default()
            }),
            ..Default::default()
        });

        assert_eq!(
            provider.resolver.auth.options.credentials,
            Some(credentials)
        );
    }

    /// upstream: @ai-sdk/google-vertex src/edge/google-vertex-provider-edge.test.ts › google-vertex-provider-edge › should pass options through to base provider when apiKey is provided
    #[tokio::test]
    #[serial]
    async fn should_pass_options_through_to_base_provider_when_api_key_is_provided() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            api_key: Some(Resolvable::Value("test-api-key".into())),
            access_token: Some(mock_generate_auth_token(&calls)),
            ..Default::default()
        });

        assert_eq!(
            provider.resolver.express_key().await.unwrap().as_deref(),
            Some("test-api-key")
        );
        // No auth-token headers: only the Express-mode key.
        let headers = provider_headers(&provider).await;
        assert!(
            !headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("authorization")),
            "{headers:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
