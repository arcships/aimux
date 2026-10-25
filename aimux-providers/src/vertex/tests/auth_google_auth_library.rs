//! Port of `@ai-sdk/google-vertex` 5.0.100
//! `src/google-vertex-auth-google-auth-library.test.ts`.
//!
//! Upstream's `createAuthTokenGenerator` wraps `google-auth-library`, which the
//! test replaces with a mock `GoogleAuth` whose client returns a fixed token.
//! aimux's counterpart is the provider's Google credential resolver
//! (`GoogleAuth` in `src/vertex/auth.rs`); it cannot be replaced, so the tests
//! give it real service-account credentials (a test RSA key written to a
//! key file) and answer its token request from a mock transport. The scopes
//! and key file upstream passes to the `GoogleAuth` constructor are read
//! from the signed assertion the resolver sends; the mocked client's
//! `{ token: null }` is the `auth_client` option, aimux's replacement for the
//! OAuth client. Tests that set environment variables are serial.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use serde_json::{Value, json};
use serial_test::serial;

use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, Seen};
use crate::vertex::GoogleAuthOptions;
use crate::vertex::auth::GoogleAuth;
use aimux_provider_utils::Resolvable;

const TEST_RSA_KEY: &str = include_str!("fixtures/aimux-test-rsa-key.pem");
const CLOUD_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// A directory of key files for one test, removed on drop.
struct KeyDirectory(PathBuf);

impl KeyDirectory {
    fn new(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "aimux-vertex-auth-library-{}-{test}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// A service-account key file named `name` for `client_email`.
    fn key_file(&self, name: &str, client_email: &str) -> PathBuf {
        let path = self.0.join(name);
        let credentials = json!({
            "type": "service_account",
            "client_email": client_email,
            "private_key": TEST_RSA_KEY,
        });
        std::fs::write(&path, credentials.to_string()).unwrap();
        path
    }
}

impl Drop for KeyDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `getAccessToken.mockResolvedValue({ token })`: the token endpoint's answer.
fn token_response(token: &str) -> Canned {
    Canned::json(&json!({"access_token": token, "expires_in": 3600}))
}

fn generator(options: GoogleAuthOptions, fetch: &Arc<MockFetch>) -> GoogleAuth {
    GoogleAuth::new(options, Some(fetch.transport()))
}

fn key_file_options(path: &Path) -> GoogleAuthOptions {
    GoogleAuthOptions {
        key_file: Some(path.to_path_buf()),
        ..Default::default()
    }
}

/// The claims of the JWT assertion a token request carries.
fn assertion_claims(seen: &Seen) -> Value {
    let assertion = url::form_urlencoded::parse(&seen.body)
        .find(|(name, _)| name == "assertion")
        .map(|(_, value)| value.into_owned())
        .expect("a JWT-bearer token request");
    let claims = assertion.split('.').nth(1).expect("a JWT");
    serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(claims)
            .unwrap(),
    )
    .unwrap()
}

mod create_auth_token_generator {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/google-vertex-auth-google-auth-library.test.ts › createAuthTokenGenerator › should generate a valid auth token
    #[tokio::test]
    #[serial]
    async fn should_generate_a_valid_auth_token() {
        // `createAuthTokenGenerator()` with no options: the application
        // default credentials, here from GOOGLE_APPLICATION_CREDENTIALS.
        let keys = KeyDirectory::new("valid-token");
        let path = keys.key_file("adc.json", "adc@test.iam.gserviceaccount.com");
        let _credentials = EnvVar::set(
            "GOOGLE_APPLICATION_CREDENTIALS",
            Some(path.to_str().unwrap()),
        );
        let fetch = MockFetch::new(vec![token_response("mocked-token")]);
        let generate_auth_token = generator(GoogleAuthOptions::default(), &fetch);

        let token = generate_auth_token.access_token().await.unwrap();

        assert_eq!(token, "mocked-token");
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-auth-google-auth-library.test.ts › createAuthTokenGenerator › should return null if no token is received
    #[tokio::test]
    async fn should_return_null_if_no_token_is_received() {
        // The client answers `{ token: null }`. aimux's generator returns the
        // token text that goes into `Authorization: Bearer <token>`, where a
        // null token is the text `null`, as in upstream's template string.
        let fetch = MockFetch::new(vec![]);
        let generate_auth_token = generator(
            GoogleAuthOptions {
                auth_client: Some(Resolvable::Value(None)),
                ..Default::default()
            },
            &fetch,
        );

        let token = generate_auth_token.access_token().await.unwrap();

        assert_eq!(token, "null");
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-auth-google-auth-library.test.ts › createAuthTokenGenerator › should create a new auth instance with provided options
    #[tokio::test]
    async fn should_create_a_new_auth_instance_with_provided_options() {
        let keys = KeyDirectory::new("provided-options");
        let path = keys.key_file("test-key.json", "test-key@test.iam.gserviceaccount.com");
        let fetch = MockFetch::new(vec![token_response("mocked-token")]);

        generator(key_file_options(&path), &fetch)
            .access_token()
            .await
            .unwrap();

        // `{ scopes: [cloud-platform], keyFile: 'test-key.json' }`: the
        // credentials of the key file, with the Cloud Platform scope.
        let claims = assertion_claims(&fetch.seen()[0]);
        assert_eq!(claims["iss"], "test-key@test.iam.gserviceaccount.com");
        assert_eq!(claims["scope"], CLOUD_SCOPE);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-auth-google-auth-library.test.ts › createAuthTokenGenerator › should create only one GoogleAuth instance for repeated calls
    #[tokio::test]
    async fn should_create_only_one_google_auth_instance_for_repeated_calls() {
        let keys = KeyDirectory::new("repeated-calls");
        let path = keys.key_file("key.json", "repeat@test.iam.gserviceaccount.com");
        let fetch = MockFetch::new(vec![token_response("mocked-token")]);
        let generate_auth_token = generator(key_file_options(&path), &fetch);

        generate_auth_token.access_token().await.unwrap();
        generate_auth_token.access_token().await.unwrap();

        // One auth instance: its client is set up (and its token fetched)
        // once.
        assert_eq!(fetch.seen().len(), 1);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-auth-google-auth-library.test.ts › createAuthTokenGenerator › should create independent generators for separate option sets
    #[tokio::test]
    async fn should_create_independent_generators_for_separate_option_sets() {
        let keys = KeyDirectory::new("separate-options");
        let first = keys.key_file("first-key.json", "first@test.iam.gserviceaccount.com");
        let second = keys.key_file("second-key.json", "second@test.iam.gserviceaccount.com");
        let fetch = MockFetch::new(vec![
            token_response("first-token"),
            token_response("second-token"),
        ]);
        let generate_first_auth_token = generator(key_file_options(&first), &fetch);
        let generate_second_auth_token = generator(key_file_options(&second), &fetch);

        generate_first_auth_token.access_token().await.unwrap();
        generate_second_auth_token.access_token().await.unwrap();

        let seen = fetch.seen();
        assert_eq!(seen.len(), 2);
        let first_claims = assertion_claims(&seen[0]);
        assert_eq!(first_claims["iss"], "first@test.iam.gserviceaccount.com");
        assert_eq!(first_claims["scope"], CLOUD_SCOPE);
        let second_claims = assertion_claims(&seen[1]);
        assert_eq!(second_claims["iss"], "second@test.iam.gserviceaccount.com");
        assert_eq!(second_claims["scope"], CLOUD_SCOPE);
    }
}
