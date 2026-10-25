//! Port of `@ai-sdk/google-vertex` 5.0.100 `src/google-vertex-provider.test.ts`.
//!
//! Upstream's Node provider wraps the base provider: it mocks
//! `createAuthTokenGenerator` and the base `createGoogleVertex` and checks the
//! options the wrapper passes on. aimux has one provider, so the tests look at
//! the same values inside it: the provider headers its resolver produces for a
//! request (upstream's `passedOptions.headers`), the options of the Google
//! credential resolver it built (upstream's `createAuthTokenGenerator`
//! argument), and an `access_token` producer standing in for the mocked token
//! generator. Every test clears `GOOGLE_VERTEX_API_KEY` (upstream's
//! `beforeEach`) and `GOOGLE_VERTEX_ACCESS_TOKEN` (read before the generator),
//! so all of them are serial.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::FutureExt;
use serde_json::json;
use serial_test::serial;

use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch};
use crate::vertex::{
    GoogleAuthOptions, GoogleAuthScopes, VertexProvider, VertexProviderSettings,
    create_google_vertex,
};
use aimux_provider_utils::{HeaderMapOpt, Resolvable};

const TEST_RSA_KEY: &str = include_str!("fixtures/aimux-test-rsa-key.pem");

fn clean_environment() -> Vec<EnvVar> {
    ["GOOGLE_VERTEX_API_KEY", "GOOGLE_VERTEX_ACCESS_TOKEN"]
        .into_iter()
        .map(|name| EnvVar::set(name, None))
        .collect()
}

/// `vi.fn().mockResolvedValue('mock-auth-token')`, counting its calls.
fn mock_auth_token(calls: &Arc<AtomicUsize>) -> Resolvable<String> {
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

/// The options of the provider's Google credential resolver
/// (`createAuthTokenGenerator`'s argument).
fn auth_options(provider: &VertexProvider) -> &GoogleAuthOptions {
    &provider.resolver.auth.options
}

fn scopes(options: &GoogleAuthOptions) -> Option<Vec<String>> {
    options.scopes.as_ref().map(|scopes| match scopes {
        GoogleAuthScopes::String(scope) => vec![scope.clone()],
        GoogleAuthScopes::Array(scopes) => scopes.clone(),
    })
}

mod google_vertex_provider {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › default headers function should return auth token
    #[tokio::test]
    #[serial]
    async fn default_headers_function_should_return_auth_token() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            access_token: Some(mock_auth_token(&calls)),
            ..Default::default()
        });

        assert_eq!(
            provider_headers(&provider).await,
            headers(&[("Authorization", "Bearer mock-auth-token")])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › should use custom headers in addition to auth token when provided
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
            access_token: Some(mock_auth_token(&calls)),
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

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › passes googleAuthOptions to createAuthTokenGenerator
    #[tokio::test]
    #[serial]
    async fn passes_google_auth_options_to_create_auth_token_generator() {
        let _env = clean_environment();
        // aimux builds the credential resolver with the provider, so there
        // is no headers call to trigger it (the key file does not exist).
        let provider = create(VertexProviderSettings {
            google_auth_options: Some(GoogleAuthOptions {
                scopes: Some(GoogleAuthScopes::Array(vec![
                    "https://www.googleapis.com/auth/cloud-platform".into(),
                ])),
                key_file: Some("path/to/key.json".into()),
                ..Default::default()
            }),
            ..Default::default()
        });

        let options = auth_options(&provider);
        assert_eq!(
            scopes(options),
            Some(vec![
                "https://www.googleapis.com/auth/cloud-platform".to_string()
            ])
        );
        assert_eq!(options.key_file, Some(PathBuf::from("path/to/key.json")));
        assert_eq!(options.project_id, None);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › passes project to createAuthTokenGenerator as projectId
    #[tokio::test]
    #[serial]
    async fn passes_project_to_create_auth_token_generator_as_project_id() {
        let _env = clean_environment();
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            ..Default::default()
        });

        let options = auth_options(&provider);
        assert_eq!(options.project_id.as_deref(), Some("test-project"));
        assert_eq!(scopes(options), None);
        assert_eq!(options.key_file, None);
        assert!(options.credentials.is_none());
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › does not override explicit googleAuthOptions projectId
    #[tokio::test]
    #[serial]
    async fn does_not_override_explicit_google_auth_options_project_id() {
        let _env = clean_environment();
        let provider = create(VertexProviderSettings {
            project: Some("provider-project".into()),
            google_auth_options: Some(GoogleAuthOptions {
                project_id: Some("auth-project".into()),
                key_file: Some("path/to/key.json".into()),
                ..Default::default()
            }),
            ..Default::default()
        });

        let options = auth_options(&provider);
        assert_eq!(options.project_id.as_deref(), Some("auth-project"));
        assert_eq!(options.key_file, Some(PathBuf::from("path/to/key.json")));
        assert_eq!(scopes(options), None);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › should pass options through to base provider when apiKey is provided
    #[tokio::test]
    #[serial]
    async fn should_pass_options_through_to_base_provider_when_api_key_is_provided() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            api_key: Some(Resolvable::Value("test-api-key".into())),
            access_token: Some(mock_auth_token(&calls)),
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

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › creates the auth token generator once per provider instance
    #[tokio::test]
    #[serial]
    async fn creates_the_auth_token_generator_once_per_provider_instance() {
        let _env = clean_environment();
        // One generator per provider: its service-account token is fetched
        // once and reused by the second headers call.
        let fetch = MockFetch::new(vec![Canned::json(
            &json!({"access_token": "sa-token", "expires_in": 3600}),
        )]);
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            google_auth_options: Some(GoogleAuthOptions {
                credentials: Some(json!({
                    "type": "service_account",
                    "client_email": "test@test.iam.gserviceaccount.com",
                    "private_key": TEST_RSA_KEY,
                })),
                ..Default::default()
            }),
            fetch: Some(fetch.transport()),
            ..Default::default()
        });

        let first = provider_headers(&provider).await;
        let second = provider_headers(&provider).await;

        assert_eq!(first, headers(&[("Authorization", "Bearer sa-token")]));
        assert_eq!(second, first);
        assert_eq!(fetch.seen().len(), 1);
    }
}
