//! SSRF guard wiring tests for `send_validated`.
//!
//! The guard must reject provider-supplied URLs that point at
//! private/loopback/link-local space (before any connection is attempted),
//! re-validate every redirect hop, and keep same-origin traffic against the
//! configured `trusted_origin` working — mirroring AI SDK's `validateUrl` +
//! `trustedOrigin` semantics. The mock server runs on loopback, so it is only
//! reachable through the trusted-origin exemption; anything the guard treats
//! as foreign is rejected as a non-public literal.

use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send_validated};

fn request(url: String) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Get,
        url,
        headers: vec![("authorization".into(), "Bearer test".into())],
        body: HttpBody::Empty,
        abort_signal: None,
        call_id: None,
        recording_context: None,
    }
}

#[tokio::test]
async fn rejects_non_public_literal_urls_before_connecting() {
    for url in [
        "http://169.254.169.254/latest/meta-data",
        "http://127.0.0.1:9/file",
        "http://[::ffff:169.254.169.254]/meta",
        "http://10.1.2.3/file",
    ] {
        let error = send_validated(
            request(url.into()),
            None,
            None,
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await
        .expect_err("non-public literal must be rejected");
        assert!(
            matches!(error, AiMuxError::InvalidArgument(ref m) if m.contains("non-public")),
            "unexpected error for {url}: {error:?}"
        );
    }
}

#[tokio::test]
async fn trusted_origin_download_succeeds_and_keeps_auth_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/generated/file"))
        .and(header("authorization", "Bearer test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": "ok"})))
        .expect(1)
        .mount(&server)
        .await;

    let response = send_validated(
        request(format!("{}/generated/file", server.uri())),
        Some(&server.uri()),
        Some(&server.uri()),
        RetryConfig::default(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap(),
        json!({"value": "ok"})
    );
}

#[tokio::test]
async fn follows_a_relative_redirect_within_the_trusted_origin() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/final"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/final"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"done"))
        .expect(1)
        .mount(&server)
        .await;

    let response = send_validated(
        request(format!("{}/start", server.uri())),
        Some(&server.uri()),
        Some(&server.uri()),
        RetryConfig::default(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body.as_ref(), b"done");
}

#[tokio::test]
async fn rejects_a_redirect_to_a_non_public_target() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://169.254.169.254/meta"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = send_validated(
        request(format!("{}/start", server.uri())),
        Some(&server.uri()),
        Some(&server.uri()),
        RetryConfig::default(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await
    .expect_err("redirect to metadata IP must be rejected");
    assert!(
        matches!(error, AiMuxError::InvalidArgument(ref m) if m.contains("non-public")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn rejects_a_redirect_onto_a_foreign_loopback_origin() {
    // The trusted origin is one loopback server; a redirect to a DIFFERENT
    // loopback origin must not inherit the exemption.
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/private", other.uri()).as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = send_validated(
        request(format!("{}/start", server.uri())),
        Some(&server.uri()),
        Some(&server.uri()),
        RetryConfig::default(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await
    .expect_err("foreign loopback origin must be rejected");
    assert!(
        matches!(error, AiMuxError::InvalidArgument(ref m) if m.contains("non-public")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn rejects_a_redirect_to_a_data_url() {
    // Fetch treats a redirect to a non-HTTP(S) scheme as a network error; a
    // server must not be able to fabricate a response via Location: data:.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "data:text/plain,forged"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = send_validated(
        request(format!("{}/start", server.uri())),
        Some(&server.uri()),
        Some(&server.uri()),
        RetryConfig::default(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await
    .expect_err("redirect to a data: URL must be rejected");
    assert!(
        matches!(error, AiMuxError::ApiCall(ref e) if e.message.contains("non-HTTP scheme")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn sanitizes_metadata_and_cookie_headers_from_download_requests() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok"))
        .expect(1)
        .mount(&server)
        .await;

    let mut req = request(format!("{}/file", server.uri()));
    req.headers.push(("Cookie".into(), "session=secret".into()));
    req.headers
        .push(("Metadata-Flavor".into(), "Google".into()));
    send_validated(
        req,
        Some(&server.uri()),
        Some(&server.uri()),
        RetryConfig::default(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await
    .unwrap();

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let names: Vec<String> = received[0]
        .headers
        .keys()
        .map(|name| name.as_str().to_ascii_lowercase())
        .collect();
    assert!(!names.contains(&"cookie".to_string()));
    assert!(!names.contains(&"metadata-flavor".to_string()));
    assert!(names.contains(&"authorization".to_string()));
}
