//! Ports of `@ai-sdk/amazon-bedrock` 5.0.100
//! `src/amazon-bedrock-sigv4-fetch.test.ts`.
//!
//! aimux has no `createSigV4FetchFunction` / `createApiKeyFetchFunction`. The
//! provider's transport is `BedrockAuthFetch` (`bedrock/mod.rs`): it adds the
//! `ai-sdk-amazon-bedrock/<v>` user-agent part, then signs with
//! `aimux_provider_utils::SigV4Fetch` (service `bedrock`, aws4fetch's header
//! rule) unless the request already carries `Authorization`. The API key is
//! put into the provider headers, not by the transport, so the cases about
//! adding, merging or replacing it run through `create_amazon_bedrock` and the
//! mock transport; the cases about what the transport does with a request that
//! carries the key start from such a request.
//!
//! Upstream mocks the `aws4fetch` signer: its `x-amz-date` is fixed and its
//! `authorization` is `AWS4-HMAC-SHA256 Credential=test`. The port signs for
//! real on a fixed clock (`SigV4Fetch::with_clock`, the upstream date), so
//! `x-amz-date` is compared exactly and `authorization` by its credential
//! prefix. Upstream also mocks the runtime part of the user-agent
//! (`testenv`), which aimux does not send.
//!
//! Not ported (registered, `AWS SigV4 fetch 也给 GET 请求签名（§9.1 已知差距）`):
//! "should bypass signing for non-POST requests", "should bypass signing if
//! POST request has no body" and "should call original fetch if init is
//! undefined" (a `GET`). aimux signs every request, GETs and bodyless POSTs
//! included (`list_models` is a signed GET).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, TimeZone, Utc};
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AwsCredentials, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt,
    ProxyConfig, RequestCredentials, ReqwestFetch, Resolvable, SigV4Fetch, SigV4Signing,
    set_default_transport,
};

use crate::bedrock::{
    AmazonBedrockProvider, AmazonBedrockProviderSettings, BedrockAuthFetch, SIGV4_SERVICE,
    create_amazon_bedrock,
};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

/// The user-agent the transport sends: the pinned package part.
const USER_AGENT: &str = "ai-sdk-amazon-bedrock/5.0.100";

/// The signing date of upstream's mocked signer.
const AMZ_DATE: &str = "20240315T000000Z";

fn fixed_clock() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 3, 15, 0, 0, 0)
        .single()
        .expect("valid date")
}

/// The credentials of the upstream `createFetchFunction`.
fn credentials(session_token: Option<&str>) -> AwsCredentials {
    AwsCredentials {
        access_key_id: "test-access-key".into(),
        secret_access_key: "test-secret".into(),
        session_token: session_token.map(str::to_string),
        region: "us-west-2".into(),
    }
}

/// `createSigV4FetchFunction(getCredentials, inner, service)`: the provider's
/// transport, signing for `service`.
fn sigv4_fetch_for(
    credentials: Resolvable<AwsCredentials>,
    inner: FetchFunction,
    service: &str,
) -> FetchFunction {
    Arc::new(BedrockAuthFetch {
        signing: SigV4Fetch::new(inner.clone(), credentials, service, SigV4Signing::Aws4Fetch)
            .with_clock(fixed_clock),
        inner,
    })
}

/// `createSigV4FetchFunction(getCredentials, inner)`: the provider's
/// transport with its own service name.
fn sigv4_fetch(credentials: Resolvable<AwsCredentials>, inner: FetchFunction) -> FetchFunction {
    sigv4_fetch_for(credentials, inner, SIGV4_SERVICE)
}

/// The provider's transport for a request that already carries the API key
/// as `Authorization` (which the provider headers put there): forwarded
/// unsigned. Its signer would fail, so a signed request would show.
fn api_key_fetch(inner: FetchFunction) -> FetchFunction {
    sigv4_fetch(
        Resolvable::from_fn(|| Err(AiMuxError::other("no credentials"))),
        inner,
    )
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

/// Asserts the SigV4 headers of a forwarded request: the fixed signing date
/// and a SigV4 credential of `access_key_id` for `region` and `service`.
fn assert_signed(seen: &Seen, access_key_id: &str, region: &str, service: &str) {
    assert_eq!(seen.headers["x-amz-date"], AMZ_DATE);
    let authorization = &seen.headers["authorization"];
    let credential = format!(
        "AWS4-HMAC-SHA256 Credential={access_key_id}/20240315/{region}/{service}/aws4_request, "
    );
    assert!(
        authorization.starts_with(&credential),
        "unexpected authorization: {authorization}"
    );
}

/// The headers a provider request sends without the `user-agent`, which is
/// checked on its own to carry the package part.
fn caller_visible(headers: &BTreeMap<String, String>) -> Value {
    let mut headers = headers.clone();
    let user_agent = headers.remove("user-agent").expect("user-agent is sent");
    assert!(
        user_agent.split(' ').any(|part| part == USER_AGENT),
        "user-agent {user_agent} lacks {USER_AGENT}"
    );
    json!(headers)
}

/// The Titan embedding response, for the cases that go through the provider.
fn titan_response() -> Canned {
    Canned::json(&json!({ "embedding": [0.1], "inputTextTokenCount": 1 }))
}

const TITAN: &str = "amazon.titan-embed-text-v2:0";

/// `create_amazon_bedrock` with an API key, the given headers, transport and
/// base URL.
fn api_key_provider(
    api_key: &str,
    headers: Option<HeaderMapOpt>,
    fetch: Option<FetchFunction>,
    base_url: &str,
) -> AmazonBedrockProvider {
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        api_key: Some(api_key.to_string().into()),
        region: Some("us-east-1".to_string()),
        base_url: Some(base_url.to_string()),
        headers,
        fetch,
        ..Default::default()
    })
    .expect("valid Bedrock settings")
}

/// One `POST /model/{titan}/invoke` with a JSON body through `provider`.
async fn embed(provider: &AmazonBedrockProvider) {
    provider
        .embedding(TITAN)
        .do_embed(&EmbeddingCallOptions::new("x"))
        .await
        .expect("embedding succeeds");
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
/// `200` with a Titan embedding.
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

    /// Proxies plain-HTTP requests, except to loopback hosts (other tests'
    /// local servers).
    fn config(&self) -> ProxyConfig {
        ProxyConfig {
            http_url: Some(format!("http://127.0.0.1:{}", self.port)),
            no_proxy: Some("localhost,127.0.0.1".to_string()),
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
    let body = r#"{"embedding":[0.1],"inputTextTokenCount":1}"#;
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

/// The request line of the provider's Titan request through the proxy.
const PROXIED_REQUEST_LINE: &str =
    "POST http://example.com/model/amazon.titan-embed-text-v2%3A0/invoke ";

mod create_sig_v4_fetch_function {
    use super::*;

    // "should bypass signing for non-POST requests" and "should bypass signing
    // if POST request has no body": registered (§9.1), see the file comment.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should handle a POST request with a string body and merge signed headers including user-agent
    #[tokio::test]
    async fn should_handle_a_post_request_with_a_string_body_and_merge_signed_headers() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        // The session token makes the signer add `x-amz-security-token`.
        let fetch = sigv4_fetch(
            Resolvable::from_fn(|| Ok(credentials(Some("test-session-token")))),
            mock.transport(),
        );

        fetch
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

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        let headers = &seen[0].headers;
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["custom-header"], "value");
        assert!(!headers.contains_key("empty-header"));
        assert_signed(&seen[0], "test-access-key", "us-west-2", "bedrock");
        assert_eq!(headers["x-amz-security-token"], "test-session-token");
        assert_eq!(headers["user-agent"], USER_AGENT);
        // Body is left unmodified for a string body.
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should handle a POST request with a Request object
    ///
    /// The JS `Request` object and the `init` headers are two layers the
    /// transport merges. A Rust `FetchRequest` is already one request, so both
    /// layers are given as its headers.
    #[tokio::test]
    async fn should_handle_a_post_request_with_a_request_object() {
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

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should sign when input is a POST Request with body and no init
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
        assert_signed(&seen[0], "test-access-key", "us-west-2", "bedrock");
        assert_eq!(headers["user-agent"], USER_AGENT);
    }

    // Not ported (ts-only): "should handle non-string body by stringifying
    // it". Its only subject is a JS object passed as `body` and coerced with
    // `JSON.stringify`. `FetchRequest.body` is bytes, so there is no
    // non-string body to coerce.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should handle Uint8Array body
    ///
    /// A `Uint8Array` body and an `ArrayBuffer` body are both bytes in Rust, so
    /// this case and the next send the same kind of request; the body is
    /// forwarded as the same bytes (upstream: decoded to the same string).
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

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should handle ArrayBuffer body
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

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should extract headers from a Headers instance
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

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should handle headers provided as an array
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
        assert_signed(&seen[0], "test-access-key", "us-west-2", "bedrock");
    }

    // "should call original fetch if init is undefined": registered (§9.1),
    // see the file comment.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should correctly handle async credential providers
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
        assert_signed(&seen[0], "async-access-key", "us-east-1", "bedrock");
        assert_eq!(
            seen[0].headers["x-amz-security-token"],
            "async-session-token"
        );
        assert_eq!(seen[0].headers["content-type"], "application/json");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should handle async credential providers that reject
    #[tokio::test]
    async fn should_handle_async_credential_providers_that_reject() {
        let mock = MockFetch::new(vec![]);
        let fetch = sigv4_fetch(
            Resolvable::from_async_fn(|| async {
                Err(AiMuxError::other("Failed to get credentials"))
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
        // The underlying fetch should not be called.
        assert!(mock.seen().is_empty());
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should use default service name "bedrock" when no service parameter is provided
    ///
    /// The signer options are not observable; the service is the one the
    /// signature's credential scope names. The provider's service is its
    /// default.
    #[tokio::test]
    async fn should_use_default_service_name_bedrock_when_no_service_parameter_is_provided() {
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
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_signed(&seen[0], "test-access-key", "us-west-2", "bedrock");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createSigV4FetchFunction › should use custom service name when service parameter is provided
    #[tokio::test]
    async fn should_use_custom_service_name_when_service_parameter_is_provided() {
        let mock = MockFetch::new(vec![ok("Signed")]);
        let fetch = sigv4_fetch_for(
            Resolvable::from_fn(|| Ok(credentials(None))),
            mock.transport(),
            "bedrock-mantle",
        );

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_signed(&seen[0], "test-access-key", "us-west-2", "bedrock-mantle");
    }
}

mod create_api_key_fetch_function {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should add Authorization header with Bearer token and user-agent
    ///
    /// The key is added by the provider headers, so this runs through the
    /// provider: a `POST` with a JSON body, whose `content-type` the provider
    /// sets.
    #[tokio::test]
    async fn should_add_authorization_header_with_bearer_token_and_user_agent() {
        let mock = MockFetch::new(vec![titan_response()]);
        let provider = api_key_provider(
            "test-api-key-123",
            None,
            Some(mock.transport()),
            "http://example.com",
        );

        embed(&provider).await;

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].json_body(), json!({ "inputText": "x" }));
        assert_eq!(
            caller_visible(&seen[0].headers),
            json!({
                "content-type": "application/json",
                "authorization": "Bearer test-api-key-123",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should merge Authorization header with existing headers
    #[tokio::test]
    async fn should_merge_authorization_header_with_existing_headers() {
        let mock = MockFetch::new(vec![titan_response()]);
        let provider = api_key_provider(
            "test-api-key-456",
            Some(HeaderMapOpt::from([
                (
                    "Content-Type".to_string(),
                    Some("application/json".to_string()),
                ),
                (
                    "Custom-Header".to_string(),
                    Some("custom-value".to_string()),
                ),
                ("X-Request-ID".to_string(), Some("req-123".to_string())),
            ])),
            Some(mock.transport()),
            "http://example.com",
        );

        embed(&provider).await;

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            caller_visible(&seen[0].headers),
            json!({
                "content-type": "application/json",
                "custom-header": "custom-value",
                "x-request-id": "req-123",
                "authorization": "Bearer test-api-key-456",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should work with Headers instance
    ///
    /// The request already carries the key, as the provider puts it there.
    #[tokio::test]
    async fn should_work_with_headers_instance() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch(mock.transport());

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[
                    ("Content-Type", "application/json"),
                    ("X-Custom", "value"),
                    ("Authorization", "Bearer test-api-key-789"),
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
                "authorization": "Bearer test-api-key-789",
                "user-agent": USER_AGENT,
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should work with headers as array
    #[tokio::test]
    async fn should_work_with_headers_as_array() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch(mock.transport());

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[
                    ("Content-Type", "application/json"),
                    ("X-Array-Header", "array-value"),
                    ("Authorization", "Bearer test-api-key-array"),
                ],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
        assert_eq!(
            json!(seen[0].headers),
            json!({
                "content-type": "application/json",
                "x-array-header": "array-value",
                "authorization": "Bearer test-api-key-array",
                "user-agent": USER_AGENT,
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should work with GET requests
    #[tokio::test]
    async fn should_work_with_get_requests() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch(mock.transport());

        fetch
            .fetch(request(
                Method::GET,
                "http://example.com",
                &[
                    ("Accept", "application/json"),
                    ("Authorization", "Bearer test-api-key-get"),
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
                "authorization": "Bearer test-api-key-get",
                "user-agent": USER_AGENT,
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should work when no headers are provided
    #[tokio::test]
    async fn should_work_when_no_headers_are_provided() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch(mock.transport());

        fetch
            .fetch(request(
                Method::POST,
                "http://example.com",
                &[("Authorization", "Bearer test-api-key-no-headers")],
                br#"{"test": "data"}"#,
            ))
            .await
            .expect("fetch succeeds");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].body, br#"{"test": "data"}"#);
        assert_eq!(
            json!(seen[0].headers),
            json!({
                "authorization": "Bearer test-api-key-no-headers",
                "user-agent": USER_AGENT,
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should work when init is undefined
    ///
    /// No `init` is the `fetch` default, a `GET` without headers (but the key).
    #[tokio::test]
    async fn should_work_when_init_is_undefined() {
        let mock = MockFetch::new(vec![ok("OK")]);
        let fetch = api_key_fetch(mock.transport());

        let response = fetch
            .fetch(request(
                Method::GET,
                "http://example.com",
                &[("Authorization", "Bearer test-api-key-undefined")],
                b"",
            ))
            .await
            .expect("fetch succeeds");
        assert_eq!(body_text(response).await, "OK");

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, "http://example.com/");
        assert_eq!(
            json!(seen[0].headers),
            json!({
                "authorization": "Bearer test-api-key-undefined",
                "user-agent": USER_AGENT,
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should override existing Authorization header
    ///
    /// The caller's `Authorization` is a provider header, which the key
    /// replaces. Upstream's expected header object also keeps a second,
    /// lower-case `authorization: Bearer old-token` key (a JS object is
    /// case-sensitive); a Rust header map is not, so that key is not asserted.
    #[tokio::test]
    async fn should_override_existing_authorization_header() {
        let mock = MockFetch::new(vec![titan_response()]);
        let provider = api_key_provider(
            "test-api-key-override",
            Some(HeaderMapOpt::from([
                (
                    "Content-Type".to_string(),
                    Some("application/json".to_string()),
                ),
                (
                    "Authorization".to_string(),
                    Some("Bearer old-token".to_string()),
                ),
            ])),
            Some(mock.transport()),
            "http://example.com",
        );

        embed(&provider).await;

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            caller_visible(&seen[0].headers),
            json!({
                "content-type": "application/json",
                "authorization": "Bearer test-api-key-override",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should use default fetch when no custom fetch provided
    ///
    /// The process default transport is routed to a fake proxy, so the request
    /// reaches it as the default transport would send it.
    #[tokio::test]
    #[serial_test::serial]
    async fn should_use_default_fetch_when_no_custom_fetch_provided() {
        let proxy = FakeProxy::start().await;
        let _restore = RestoreDefaultTransport;
        route_default_transport_to(&proxy);
        let provider = api_key_provider("test-api-key-default", None, None, "http://example.com");

        embed(&provider).await;

        let seen = proxy.seen();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].request_line.starts_with(PROXIED_REQUEST_LINE),
            "unexpected request line: {}",
            seen[0].request_line
        );
        assert_eq!(
            seen[0].headers["authorization"],
            "Bearer test-api-key-default"
        );
        assert!(
            seen[0].headers["user-agent"]
                .split(' ')
                .any(|part| part == USER_AGENT)
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should resolve default fetch lazily when no custom fetch provided
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
        let provider = api_key_provider("test-api-key-lazy", None, None, "http://example.com");
        route_default_transport_to(&patched);

        embed(&provider).await;

        assert!(
            initial.seen().is_empty(),
            "the default transport captured at creation was used: {:?}",
            initial.seen()
        );
        let seen = patched.seen();
        assert_eq!(seen.len(), 1, "the replaced default transport was not used");
        assert_eq!(seen[0].headers["authorization"], "Bearer test-api-key-lazy");
        assert!(
            seen[0].request_line.starts_with(PROXIED_REQUEST_LINE),
            "unexpected request line: {}",
            seen[0].request_line
        );
    }

    // Not ported (sub-feature): "should handle empty string API key". aimux
    // has no `createApiKeyFetchFunction` that could be handed an empty key:
    // the key is a provider header, set only for a non-blank key (as
    // upstream's `createAmazonBedrock` only calls the function for one).

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-sigv4-fetch.test.ts › createApiKeyFetchFunction › should preserve request body and other properties
    ///
    /// `cache: 'no-cache'` has no Rust counterpart on `FetchRequest` and is not
    /// asserted. `credentials: 'include'` is `RequestCredentials::Include`.
    #[tokio::test]
    async fn should_preserve_request_body_and_other_properties() {
        let recorder = Arc::new(Recorder::default());
        let fetch = api_key_fetch(recorder.clone());
        let request_body = json!({ "data": "test" }).to_string();
        let mut sent = request(
            Method::PUT,
            "http://example.com",
            &[
                ("Content-Type", "application/json"),
                ("Authorization", "Bearer test-api-key-preserve"),
            ],
            request_body.as_bytes(),
        );
        sent.credentials = RequestCredentials::Include;

        fetch.fetch(sent).await.expect("fetch succeeds");

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
                "authorization": "Bearer test-api-key-preserve",
                "user-agent": USER_AGENT,
            })
        );
    }
}
