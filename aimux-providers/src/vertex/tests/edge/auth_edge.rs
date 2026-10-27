//! Port of `@ai-sdk/google-vertex` 5.0.100 `src/edge/google-vertex-auth-edge.test.ts`.
//!
//! The edge `generateAuthToken(credentials)` signs a service-account JWT with
//! Web Crypto and exchanges it at the OAuth token endpoint. aimux has one entry
//! point: the provider's Google credential resolver (`GoogleAuth` in
//! `src/vertex/auth_google_auth_library.rs`) does the same exchange for
//! service-account credentials. Upstream's `GoogleCredentials` (`clientEmail`,
//! `privateKey`, `privateKeyId`) is the service-account JSON (`client_email`,
//! `private_key`, `private_key_id`) here. Upstream mocks Web Crypto, so its
//! `'mock-private-key'` is never parsed; aimux signs for real, so the harness
//! uses a test RSA key. `global.fetch` is the resolver's transport, and the
//! stubbed `atob` (which returns the same payload for any input) is [`atob`]
//! below.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch};
use crate::vertex::GoogleAuthOptions;
use crate::vertex::auth_google_auth_library::GoogleAuth;
use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};

const TEST_RSA_KEY: &str = include_str!("../fixtures/aimux-test-rsa-key.pem");

/// Upstream's `GoogleCredentials`.
#[derive(Clone)]
struct GoogleCredentials {
    client_email: &'static str,
    private_key: &'static str,
    private_key_id: Option<&'static str>,
}

/// `mockCredentials`, with the test RSA key.
fn mock_credentials() -> GoogleCredentials {
    GoogleCredentials {
        client_email: "test@test.iam.gserviceaccount.com",
        private_key: TEST_RSA_KEY,
        private_key_id: Some("test-key-id"),
    }
}

/// The service-account JSON of `credentials`.
fn credential_json(credentials: &GoogleCredentials) -> Value {
    let mut value = json!({
        "type": "service_account",
        "client_email": credentials.client_email,
        "private_key": credentials.private_key,
    });
    if let Some(id) = credentials.private_key_id {
        value["private_key_id"] = json!(id);
    }
    value
}

/// `setupAtobStub(credentials)`: the stubbed `atob` returns this payload for
/// any input.
fn atob(credentials: &GoogleCredentials, _input: &str) -> String {
    let mut payload = json!({
        "alg": "RS256",
        "typ": "JWT",
        "iss": credentials.client_email,
        "scope": "https://www.googleapis.com/auth/cloud-platform",
        "aud": "https://oauth2.googleapis.com/token",
        "iat": 1_616_161_616,
        "exp": 1_616_165_216,
    });
    if let Some(id) = credentials.private_key_id {
        payload["kid"] = json!(id);
    }
    payload.to_string()
}

/// `generateAuthToken(credentials)` through `fetch`.
async fn generate_auth_token(
    credentials: &GoogleCredentials,
    fetch: FetchFunction,
) -> Result<String, aimux_core::AiMuxError> {
    GoogleAuth::new(
        GoogleAuthOptions {
            credentials: Some(credential_json(credentials)),
            ..Default::default()
        },
        Some(fetch),
    )
    .access_token()
    .await
}

/// `{ ok: true, json: () => ({ access_token }) }`.
fn token_response(token: &str) -> Canned {
    Canned::json(&json!({"access_token": token}))
}

/// `global.fetch = vi.fn().mockRejectedValue(new Error(message))`.
struct RejectingFetch(&'static str);

#[async_trait]
impl Fetch for RejectingFetch {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchResponse, FetchError> {
        Err(FetchError::Connect(self.0.into()))
    }
}

mod google_vertex_edge_auth {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/edge/google-vertex-auth-edge.test.ts › Google Vertex Edge Auth › should generate a valid JWT token
    #[tokio::test]
    async fn should_generate_a_valid_jwt_token() {
        let credentials = mock_credentials();
        let fetch = MockFetch::new(vec![token_response(
            "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6InRlc3Qta2V5LWlkIn0.eyJpc3MiOiJ0ZXN0QHRlc3QuaWFtLmdzZXJ2aWNlYWNjb3VudC5jb20iLCJzY29wZSI6Imh0dHBzOi8vd3d3Lmdvb2dsZWFwaXMuY29tL2F1dGgvY2xvdWQtcGxhdGZvcm0iLCJhdWQiOiJodHRwczovL29hdXRoMi5nb29nbGVhcGlzLmNvbS90b2tlbiIsImlhdCI6MTYxNjE2MTYxNiwiZXhwIjoxNjE2MTY1MjE2fQ.signature",
        )]);

        let token = generate_auth_token(&credentials, fetch.transport())
            .await
            .unwrap();

        // JWT structure validation
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);

        // Header validation
        let header: Value = serde_json::from_str(&atob(&credentials, parts[0])).unwrap();
        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["typ"], "JWT");
        assert_eq!(header["kid"], "test-key-id");
        assert_eq!(header["iss"], credentials.client_email);

        // Payload validation
        let payload: Value = serde_json::from_str(&atob(&credentials, parts[1])).unwrap();
        assert_eq!(payload["iss"], credentials.client_email);
        assert_eq!(
            payload["scope"],
            "https://www.googleapis.com/auth/cloud-platform"
        );
        assert_eq!(payload["aud"], "https://oauth2.googleapis.com/token");
        assert!(payload.get("iat").is_some());
        assert!(payload.get("exp").is_some());

        // Verify exp is ~1 hour after iat
        assert_eq!(
            payload["exp"].as_i64().unwrap() - payload["iat"].as_i64().unwrap(),
            3600
        );
    }

    /// upstream: @ai-sdk/google-vertex src/edge/google-vertex-auth-edge.test.ts › Google Vertex Edge Auth › should throw error on fetch failure
    #[tokio::test]
    async fn should_throw_error_on_fetch_failure() {
        let error = generate_auth_token(
            &mock_credentials(),
            Arc::new(RejectingFetch("Network error")),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("Network error"), "{error}");
    }

    /// upstream: @ai-sdk/google-vertex src/edge/google-vertex-auth-edge.test.ts › Google Vertex Edge Auth › should work without privateKeyId
    #[tokio::test]
    async fn should_work_without_private_key_id() {
        let credentials_without_key_id = GoogleCredentials {
            private_key_id: None,
            ..mock_credentials()
        };
        let fetch = MockFetch::new(vec![token_response("mock.jwt.token")]);

        let token = generate_auth_token(&credentials_without_key_id, fetch.transport())
            .await
            .unwrap();
        assert!(!token.is_empty());

        // Verify the JWT structure
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);

        // Verify header doesn't include kid when privateKeyId is not provided
        let header: Value =
            serde_json::from_str(&atob(&credentials_without_key_id, parts[0])).unwrap();
        assert!(header.get("kid").is_none());
    }
}
