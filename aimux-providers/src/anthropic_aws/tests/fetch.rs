//! Ports of the `createSigV4FetchFunction` and `createApiKeyFetchFunction`
//! cases of `@ai-sdk/anthropic-aws` 2.0.63 (`src/anthropic-aws-fetch.test.ts`).
//!
//! aimux has no `createSigV4FetchFunction` / `createApiKeyFetchFunction`. Its
//! transport is `authenticated(..)` in `fetch.rs`: it adds the
//! `ai-sdk-anthropic-aws/<v>` user-agent part, then signs with `SigV4Fetch`
//! unless the request carries `x-api-key`. The API key is put into the provider
//! headers (`AwsHeaders::resolve`), not by the transport, so the cases about
//! adding or overriding it run through `create_anthropic_aws` and the mock
//! transport. The cases about what the transport does with a request that
//! already carries the key start from such a request.
//!
//! Upstream mocks the `aws4fetch` signer and the clock, so its `x-amz-date` and
//! `authorization` values are fixed. `authenticated` builds the real signer on
//! the real clock and takes no clock argument, so those headers are asserted by
//! shape and by the credential they carry.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{NaiveDateTime, Utc};
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use aimux_core::error::AiMuxError;
use aimux_core::provider::ProviderDiscovery;
use aimux_provider_utils::{
    AwsCredentials, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt,
    ProxyConfig, RequestCredentials, ReqwestFetch, Resolvable, set_default_transport,
};

use crate::anthropic_aws::fetch::{Auth, authenticated};
use crate::anthropic_aws::{
    AnthropicAwsAuth, AnthropicAwsProvider, AnthropicAwsProviderSettings, create_anthropic_aws,
};
use crate::shared::aisdk_pins::ANTHROPIC_AWS;
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

/// The Claude Platform on AWS endpoint of the provider cases. Its host is not
/// contacted: the mock transport answers.
const BASE_URL: &str = "https://aws-external-anthropic.us-west-2.api.aws/v1";

/// The base URL of the cases that go through the process default transport,
/// which speaks plain HTTP to the fake proxy.
const PLAIN_BASE_URL: &str = "http://example.com/v1";

const WORKSPACE_ID: &str = "ws-test";

/// The `user-agent` the transport sends: the pinned package part. Upstream's
/// expectation also carries a mocked runtime part (`testenv`), which the
/// transport does not add.
fn user_agent() -> String {
    format!("ai-sdk-{}/{}", ANTHROPIC_AWS.name, ANTHROPIC_AWS.version)
}

/// The credentials of the upstream test's `createFetchFunction`.
fn credentials(session_token: Option<&str>) -> AwsCredentials {
    AwsCredentials {
        access_key_id: "test-access-key".into(),
        secret_access_key: "test-secret".into(),
        session_token: session_token.map(str::to_string),
        region: "us-west-2".into(),
    }
}

/// `createSigV4FetchFunction(getCredentials, inner)`: the transport
/// `authenticated` builds for SigV4 auth.
fn sigv4_fetch(credentials: Resolvable<AwsCredentials>, inner: FetchFunction) -> FetchFunction {
    let auth = Arc::new(Auth {
        region: None,
        workspace_id: None,
        auth: Some(AnthropicAwsAuth::SigV4(credentials)),
    });
    authenticated(auth, None, Some(inner)).1
}

/// `createApiKeyFetchFunction(apiKey, inner)` as the transport: a request
/// that already carries `x-api-key` is forwarded unsigned.
fn api_key_fetch(api_key: &str, inner: FetchFunction) -> FetchFunction {
    let auth = Arc::new(Auth {
        region: None,
        workspace_id: None,
        auth: Some(AnthropicAwsAuth::ApiKey(Resolvable::Value(
            api_key.to_string(),
        ))),
    });
    authenticated(auth, None, Some(inner)).1
}

/// A request with the given headers and body. The JS `Request`, `Headers`,
/// header array and `ArrayBuffer` / `Uint8Array` inputs all become one of these.
fn request(method: Method, url: &str, headers: &[(&str, &str)], body: &[u8]) -> FetchRequest {
    let mut request = FetchRequest::new(method, Url::parse(url).expect("valid URL"));
    for (name, value) in headers {
        request.headers.insert(
            HeaderName::from_bytes(name.as_bytes()).expect("valid header name"),
            HeaderValue::from_str(value).expect("valid header value"),
        );
    }
    request.body = Bytes::copy_from_slice(body);
    request
}

/// A canned `200 OK` with a text body.
fn ok(body: &str) -> Canned {
    Canned {
        status: 200,
        headers: vec![],
        body: body.as_bytes().to_vec(),
    }
}

async fn body_text(response: FetchResponse) -> String {
    let chunks: Vec<Result<Bytes, FetchError>> = response.bytes_stream().collect().await;
    let bytes: Vec<u8> = chunks
        .into_iter()
        .flat_map(|chunk| chunk.expect("body chunk").to_vec())
        .collect();
    String::from_utf8(bytes).expect("UTF-8 body")
}

/// Asserts the SigV4 headers of a forwarded request: `x-amz-date` is the
/// current signing time, and `authorization` is a SigV4 credential for
/// `access_key_id` on the `aws-external-anthropic` service.
fn assert_signed(seen: &Seen, access_key_id: &str) {
    let amz_date = seen
        .headers
        .get("x-amz-date")
        .expect("x-amz-date is signed");
    let signed_at = NaiveDateTime::parse_from_str(amz_date, "%Y%m%dT%H%M%SZ")
        .expect("x-amz-date is an ISO 8601 basic timestamp");
    let skew = (Utc::now().naive_utc() - signed_at).num_seconds().abs();
    assert!(
        skew < 300,
        "x-amz-date {amz_date} is not the signing time (off by {skew}s)"
    );
    let authorization = seen
        .headers
        .get("authorization")
        .expect("authorization is signed");
    assert!(
        authorization.starts_with(&format!("AWS4-HMAC-SHA256 Credential={access_key_id}/")),
        "unexpected authorization: {authorization}"
    );
    assert!(
        authorization.contains("/aws-external-anthropic/aws4_request"),
        "unexpected authorization scope: {authorization}"
    );
}

/// The headers a provider request sends that the upstream expectations do not
/// list: `anthropic-version` and `anthropic-workspace-id` come from the
/// provider, and `user-agent` is checked on its own.
fn caller_visible(headers: &BTreeMap<String, String>) -> Value {
    let mut headers = headers.clone();
    for name in ["anthropic-version", "anthropic-workspace-id", "user-agent"] {
        headers.remove(name);
    }
    json!(headers)
}

fn assert_user_agent_part(headers: &BTreeMap<String, String>) {
    let user_agent_header = headers.get("user-agent").expect("user-agent is sent");
    assert!(
        user_agent_header
            .split(' ')
            .any(|part| part == user_agent()),
        "user-agent {user_agent_header} lacks {}",
        user_agent()
    );
}

/// `create_anthropic_aws` with the workspace id and the given auth, headers,
/// transport and base URL.
fn anthropic_aws_provider(
    auth: AnthropicAwsAuth,
    headers: Option<HeaderMapOpt>,
    fetch: Option<FetchFunction>,
    base_url: &str,
) -> AnthropicAwsProvider {
    create_anthropic_aws(AnthropicAwsProviderSettings {
        base_url: Some(base_url.to_string()),
        workspace_id: Some(WORKSPACE_ID.to_string()),
        auth: Some(auth),
        headers,
        fetch,
        ..Default::default()
    })
    .expect("valid Anthropic-AWS settings")
}

fn header_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().expect("ASCII header").to_string(),
            )
        })
        .collect()
}

/// Records the whole request the transport forwards, including the fields
/// [`Seen`] does not carry.
#[derive(Default)]
struct Recorder {
    requests: Mutex<Vec<FetchRequest>>,
}

#[async_trait]
impl Fetch for Recorder {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let url = request.url.clone();
        self.requests.lock().unwrap().push(request);
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            HeaderMap::new(),
            url,
            Bytes::from_static(b"OK"),
        ))
    }
}

/// One request as the fake proxy received it.
#[derive(Clone, Debug)]
struct SeenByProxy {
    request_line: String,
    headers: BTreeMap<String, String>,
}

/// A plain-HTTP stand-in for a proxy. The default transport sends plain-HTTP
/// requests to it in absolute form. It records each request head and answers
/// `200` with `{"data":[]}`.
#[derive(Clone)]
struct FakeProxy {
    port: u16,
    seen: Arc<Mutex<Vec<SeenByProxy>>>,
}

impl FakeProxy {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake proxy");
        let port = listener.local_addr().expect("local address").port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move { serve_one(stream, recorded).await });
            }
        });
        Self { port, seen }
    }

    fn config(&self) -> ProxyConfig {
        ProxyConfig {
            http_url: Some(format!("http://127.0.0.1:{}", self.port)),
            ..ProxyConfig::default()
        }
    }

    fn seen(&self) -> Vec<SeenByProxy> {
        self.seen.lock().unwrap().clone()
    }
}

async fn serve_one(mut stream: TcpStream, recorded: Arc<Mutex<Vec<SeenByProxy>>>) {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_string();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    recorded.lock().unwrap().push(SeenByProxy {
        request_line,
        headers,
    });
    let body = r#"{"data":[]}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Routes the process default transport through `proxy`. Tests that call it
/// are `#[serial]` and hold a [`RestoreDefaultTransport`].
fn route_default_transport_to(proxy: &FakeProxy) {
    set_default_transport(ReqwestFetch::new(proxy.config()));
}

/// Restores the built-in process default transport when dropped.
struct RestoreDefaultTransport;

impl Drop for RestoreDefaultTransport {
    fn drop(&mut self) {
        set_default_transport(ReqwestFetch::default());
    }
}

mod create_sig_v4_fetch_function {
    use super::*;

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should handle a POST request with a string body and merge signed headers including user-agent
    #[tokio::test]
    async fn should_handle_a_post_request_with_a_string_body_and_merge_signed_headers() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        // The session token makes the signer add `x-amz-security-token`.
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(Some("test-session-token")))),
            mock.transport(),
        );

        let response = fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[
                    ("Content-Type", "application/json"),
                    ("Custom-Header", "value"),
                ],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");
        assert_eq!(body_text(response).await, "Signed");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        let headers = &seen[0].headers;
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["custom-header"], "value");
        assert!(!headers.contains_key("empty-header"));
        assert_signed(&seen[0], "test-access-key");
        assert_eq!(headers["x-amz-security-token"], "test-session-token");
        assert_eq!(headers["user-agent"], user_agent());
        // Body is left unmodified for a string body.
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › shold handle a POST request with a Request object
    ///
    /// The JS `Request` object and the `init` headers are two layers the
    /// transport merges. A Rust `FetchRequest` is already one request, so both
    /// layers are given as its headers.
    #[tokio::test]
    async fn shold_handle_a_post_request_with_a_request_object() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(None))),
            mock.transport(),
        );

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[
                    ("X-From-Request", "from-request"),
                    ("Content-Type", "application/json"),
                    ("Custom-Header", "value"),
                ],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, "http://example.com/");
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
        let headers = &seen[0].headers;
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["custom-header"], "value");
        assert_eq!(headers["x-from-request"], "from-request");
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should sign when input is a POST Request with body and no init
    #[tokio::test]
    async fn should_sign_when_input_is_a_post_request_with_body_and_no_init() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(None))),
            mock.transport(),
        );

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[
                    ("Content-Type", "application/json"),
                    ("X-From-Request", "from-request"),
                ],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
        let headers = &seen[0].headers;
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["x-from-request"], "from-request");
        assert_signed(&seen[0], "test-access-key");
        assert_eq!(headers["user-agent"], user_agent());
    }

    // Not ported (ts-only): "should handle non-string body by stringifying it".
    // Its only subject is a JS object passed as `body` and coerced with
    // `JSON.stringify`. `FetchRequest.body` is bytes, so there is no
    // non-string body to coerce.

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should handle Uint8Array body
    ///
    /// A `Uint8Array` body and an `ArrayBuffer` body are both bytes in Rust, so
    /// the two cases below send the same kind of request.
    #[tokio::test]
    async fn should_handle_uint8array_body() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(None))),
            mock.transport(),
        );

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[],
                b"binaryTest",
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, b"binaryTest");
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should handle ArrayBuffer body
    #[tokio::test]
    async fn should_handle_arraybuffer_body() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(None))),
            mock.transport(),
        );

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[],
                b"bufferTest",
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, b"bufferTest");
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should extract headers from a Headers instance
    #[tokio::test]
    async fn should_extract_headers_from_a_headers_instance() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(None))),
            mock.transport(),
        );

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[("A", "value-a"), ("B", "value-b")],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].headers["a"], "value-a");
        assert_eq!(seen[0].headers["b"], "value-b");
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should handle headers provided as an array
    #[tokio::test]
    async fn should_handle_headers_provided_as_an_array() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(None))),
            mock.transport(),
        );

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[
                    ("Array-Header", "array-value"),
                    ("Another-Header", "another-value"),
                ],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].headers["array-header"], "array-value");
        assert_eq!(seen[0].headers["another-header"], "another-value");
        // Also check that the signing headers are included.
        assert_signed(&seen[0], "test-access-key");
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should call original fetch if init is undefined
    ///
    /// With no `init`, the method is the `fetch` default, GET. aimux signs
    /// GET requests too (registered in docs/aisdk-architecture-alignment.md
    /// §9.1: `list_models` is a signed GET), so the request reaches the
    /// original transport signed; upstream's unsigned headers are not asserted.
    #[tokio::test]
    async fn should_call_original_fetch_if_init_is_undefined() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(None))),
            mock.transport(),
        );

        let response = fetch
            .fetch(request(Method::GET, "http://example.com", &[], b""))
            .await
            .expect("fetch succeeds");
        assert_eq!(body_text(response).await, "OK");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "GET");
        assert_eq!(seen[0].url, "http://example.com/");
        assert_eq!(seen[0].headers["user-agent"], user_agent());
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should correctly handle async credential providers
    #[tokio::test]
    async fn should_correctly_handle_async_credential_providers() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        let fetch = sigv4_fetch(
            Resolvable::from_async_fn(|| async {
                Ok(AwsCredentials {
                    access_key_id: "async-access-key".into(),
                    secret_access_key: "async-secret-key".into(),
                    session_token: Some("async-session-token".into()),
                    region: "us-east-1".into(),
                })
            }),
            mock.transport(),
        );

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[("Content-Type", "application/json")],
                br#"{"test": "async"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_signed(&seen[0], "async-access-key");
        assert_eq!(
            seen[0].headers["x-amz-security-token"],
            "async-session-token"
        );
        assert_eq!(seen[0].headers["content-type"], "application/json");
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createSigV4FetchFunction › should handle async credential providers that reject
    #[tokio::test]
    async fn should_handle_async_credential_providers_that_reject() {
        let mock = MockFetch::new(vec![]);
        let fetch = sigv4_fetch(
            Resolvable::from_async_fn(|| async {
                Err(AiMuxError::Other("Failed to get credentials".into()))
            }),
            mock.transport(),
        );

        let error = fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect_err("the fetch rejects");
        assert!(
            error.to_string().contains("Failed to get credentials"),
            "unexpected error: {error}"
        );
        // The underlying transport is not called.
        assert!(mock.seen().is_empty());
    }
}

mod create_api_key_fetch_function {
    use super::*;

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should add x-api-key header with user-agent
    ///
    /// The key is added by the provider headers, so this runs through the
    /// provider. Its request is a `GET` of the model list, the one exchange the
    /// provider sends without a body; the upstream case is a `POST`.
    #[tokio::test]
    async fn should_add_x_api_key_header_with_user_agent() {
        let mock = MockFetch::new(vec![Canned::json(&json!({ "data": [] }))]);
        let provider = anthropic_aws_provider(
            AnthropicAwsAuth::ApiKey(Resolvable::Value("test-api-key-123".into())),
            None,
            Some(mock.transport()),
            BASE_URL,
        );

        provider.list_models().await.expect("models are listed");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].headers["x-api-key"], "test-api-key-123");
        assert_user_agent_part(&seen[0].headers);
        assert!(!seen[0].headers.contains_key("authorization"));
        assert!(!seen[0].headers.contains_key("x-amz-date"));
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should merge x-api-key header with existing headers
    #[tokio::test]
    async fn should_merge_x_api_key_header_with_existing_headers() {
        let mock = MockFetch::new(vec![Canned::json(&json!({ "data": [] }))]);
        let provider = anthropic_aws_provider(
            AnthropicAwsAuth::ApiKey(Resolvable::Value("test-api-key-456".into())),
            Some(HeaderMapOpt::from([
                (
                    "content-type".to_string(),
                    Some("application/json".to_string()),
                ),
                (
                    "custom-header".to_string(),
                    Some("custom-value".to_string()),
                ),
                ("x-request-id".to_string(), Some("req-123".to_string())),
            ])),
            Some(mock.transport()),
            BASE_URL,
        );

        provider.list_models().await.expect("models are listed");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            caller_visible(&seen[0].headers),
            json!({
                "content-type": "application/json",
                "custom-header": "custom-value",
                "x-request-id": "req-123",
                "x-api-key": "test-api-key-456",
            })
        );
        assert_user_agent_part(&seen[0].headers);
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should work with Headers instance
    ///
    /// The request already carries the key, as the provider puts it there.
    #[tokio::test]
    async fn should_work_with_headers_instance() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch("test-api-key-789", mock.transport());

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[
                    ("Content-Type", "application/json"),
                    ("X-Custom", "value"),
                    ("x-api-key", "test-api-key-789"),
                ],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].url, "http://example.com/");
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
        assert_eq!(
            json!(seen[0].headers),
            json!({
                "content-type": "application/json",
                "x-custom": "value",
                "x-api-key": "test-api-key-789",
                "user-agent": user_agent(),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should work with headers as array
    #[tokio::test]
    async fn should_work_with_headers_as_array() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch("test-api-key-array", mock.transport());

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[
                    ("Content-Type", "application/json"),
                    ("X-Array-Header", "array-value"),
                    ("x-api-key", "test-api-key-array"),
                ],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
        assert_eq!(
            json!(seen[0].headers),
            json!({
                "content-type": "application/json",
                "x-array-header": "array-value",
                "x-api-key": "test-api-key-array",
                "user-agent": user_agent(),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should work with GET requests
    #[tokio::test]
    async fn should_work_with_get_requests() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch("test-api-key-get", mock.transport());

        fetch
            .fetch(request(
                Method::GET,
                "http://example.com",
                &[
                    ("Accept", "application/json"),
                    ("x-api-key", "test-api-key-get"),
                ],
                b"",
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "GET");
        assert_eq!(
            json!(seen[0].headers),
            json!({
                "accept": "application/json",
                "x-api-key": "test-api-key-get",
                "user-agent": user_agent(),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should work when no headers are provided
    #[tokio::test]
    async fn should_work_when_no_headers_are_provided() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch("test-api-key-no-headers", mock.transport());

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[("x-api-key", "test-api-key-no-headers")],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
        assert_eq!(
            json!(seen[0].headers),
            json!({
                "x-api-key": "test-api-key-no-headers",
                "user-agent": user_agent(),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should work when init is undefined
    #[tokio::test]
    async fn should_work_when_init_is_undefined() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch("test-api-key-undefined", mock.transport());

        fetch
            .fetch(request(
                Method::GET,
                "http://example.com",
                &[("x-api-key", "test-api-key-undefined")],
                b"",
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, "http://example.com/");
        assert_eq!(
            json!(seen[0].headers),
            json!({
                "x-api-key": "test-api-key-undefined",
                "user-agent": user_agent(),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should override existing x-api-key header
    ///
    /// The caller's `x-api-key` is a provider header, which the key replaces.
    #[tokio::test]
    async fn should_override_existing_x_api_key_header() {
        let mock = MockFetch::new(vec![Canned::json(&json!({ "data": [] }))]);
        let provider = anthropic_aws_provider(
            AnthropicAwsAuth::ApiKey(Resolvable::Value("test-api-key-override".into())),
            Some(HeaderMapOpt::from([
                (
                    "content-type".to_string(),
                    Some("application/json".to_string()),
                ),
                ("x-api-key".to_string(), Some("old-token".to_string())),
            ])),
            Some(mock.transport()),
            BASE_URL,
        );

        provider.list_models().await.expect("models are listed");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            caller_visible(&seen[0].headers),
            json!({
                "content-type": "application/json",
                "x-api-key": "test-api-key-override",
            })
        );
        assert_user_agent_part(&seen[0].headers);
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should use default fetch when no custom fetch provided
    ///
    /// The process default transport is routed to a fake proxy, so the request
    /// reaches it as the default transport would send it.
    #[tokio::test]
    #[serial_test::serial]
    async fn should_use_default_fetch_when_no_custom_fetch_provided() {
        let proxy = FakeProxy::start().await;
        let _restore = RestoreDefaultTransport;
        route_default_transport_to(&proxy);
        let provider = anthropic_aws_provider(
            AnthropicAwsAuth::ApiKey(Resolvable::Value("test-api-key-default".into())),
            None,
            None,
            PLAIN_BASE_URL,
        );

        provider.list_models().await.expect("models are listed");

        let seen = proxy.seen();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0]
                .request_line
                .starts_with("GET http://example.com/v1/models "),
            "unexpected request line: {}",
            seen[0].request_line
        );
        assert_eq!(seen[0].headers["x-api-key"], "test-api-key-default");
        assert!(
            seen[0].headers["user-agent"]
                .split(' ')
                .any(|part| part == user_agent())
        );
        assert!(!seen[0].headers.contains_key("authorization"));
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should resolve default fetch lazily when no custom fetch provided
    ///
    /// The provider is created while `initial` is the process default, and the
    /// default is replaced afterwards. The request must use the default that
    /// is current when it is made.
    #[tokio::test]
    #[serial_test::serial]
    async fn should_resolve_default_fetch_lazily_when_no_custom_fetch_provided() {
        let initial = FakeProxy::start().await;
        let patched = FakeProxy::start().await;
        let _restore = RestoreDefaultTransport;
        route_default_transport_to(&initial);
        let provider = anthropic_aws_provider(
            AnthropicAwsAuth::ApiKey(Resolvable::Value("test-api-key-lazy".into())),
            None,
            None,
            PLAIN_BASE_URL,
        );
        route_default_transport_to(&patched);

        provider.list_models().await.expect("models are listed");

        assert!(
            initial.seen().is_empty(),
            "the default transport captured at creation was used: {:?}",
            initial.seen()
        );
        let seen = patched.seen();
        assert_eq!(seen.len(), 1, "the replaced default transport was not used");
        assert_eq!(seen[0].headers["x-api-key"], "test-api-key-lazy");
        assert!(
            seen[0]
                .request_line
                .starts_with("GET http://example.com/v1/models "),
            "unexpected request line: {}",
            seen[0].request_line
        );
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should handle empty string API key
    #[tokio::test]
    async fn should_handle_empty_string_api_key() {
        let mock = MockFetch::new(vec![Canned::json(&json!({ "data": [] }))]);
        let provider = anthropic_aws_provider(
            AnthropicAwsAuth::ApiKey(Resolvable::Value(String::new())),
            None,
            Some(mock.transport()),
            BASE_URL,
        );

        provider.list_models().await.expect("models are listed");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(caller_visible(&seen[0].headers), json!({ "x-api-key": "" }));
        assert_user_agent_part(&seen[0].headers);
    }

    /// upstream: @ai-sdk/anthropic-aws src/anthropic-aws-fetch.test.ts › createApiKeyFetchFunction › should preserve request body and other properties
    ///
    /// `cache: 'no-cache'` has no Rust counterpart on `FetchRequest` and is not
    /// asserted. `credentials: 'include'` is `RequestCredentials::Include`.
    #[tokio::test]
    async fn should_preserve_request_body_and_other_properties() {
        let recorder = Arc::new(Recorder::default());
        let fetch = api_key_fetch("test-api-key-preserve", recorder.clone());
        let request_body = json!({ "data": "test" }).to_string();

        fetch
            .fetch(request(
                Method::PUT,
                "http://example.com",
                &[
                    ("Content-Type", "application/json"),
                    ("x-api-key", "test-api-key-preserve"),
                ],
                request_body.as_bytes(),
            ))
            .await
            .expect("fetch succeeds");

        let requests = recorder.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let forwarded = &requests[0];
        assert_eq!(forwarded.method, Method::PUT);
        assert_eq!(forwarded.url.as_str(), "http://example.com/");
        assert_eq!(forwarded.body, Bytes::from(request_body));
        assert_eq!(forwarded.credentials, RequestCredentials::Include);
        assert_eq!(
            json!(header_map(&forwarded.headers)),
            json!({
                "content-type": "application/json",
                "x-api-key": "test-api-key-preserve",
                "user-agent": user_agent(),
            })
        );
    }
}
