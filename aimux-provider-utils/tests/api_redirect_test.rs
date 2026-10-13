//! Redirects on an API request (`validate_url: false`): every hop is
//! followed, as the AI SDK's `postToApi` does under fetch's default
//! `redirect: "follow"`, and a hop off the request's origin carries only
//! `User-Agent`, as `fetchWithValidatedRedirects` does.

use std::sync::Arc;

use aimux_provider_utils::{
    AwsCredentials, HttpRequest, ProviderErrorParts, Resolvable, SigV4Fetch, SigV4Signing,
    create_json_error_response_handler, create_json_response_handler, default_fetch,
    post_json_to_api,
};
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[derive(Deserialize)]
struct Reply {
    #[allow(dead_code)]
    ok: bool,
}

fn failed() -> aimux_provider_utils::ResponseHandler<aimux_core::AiMuxError> {
    create_json_error_response_handler(|_| ProviderErrorParts {
        message: "request failed".into(),
        provider_code: None,
    })
}

fn credentialed(url: String) -> HttpRequest {
    HttpRequest {
        url,
        headers: vec![
            ("authorization".into(), "Bearer secret".into()),
            ("x-api-key".into(), "secret-key".into()),
            ("user-agent".into(), "caller-agent".into()),
        ],
        ..Default::default()
    }
}

async fn post(request: HttpRequest) {
    post_json_to_api(
        request,
        json!({"prompt": "hi"}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();
}

async fn only_request(server: &MockServer) -> Request {
    let mut received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    received.remove(0)
}

fn has(request: &Request, name: &str) -> bool {
    request.headers.contains_key(name)
}

async fn redirect(server: &MockServer, from: &str, to: String) {
    Mock::given(path(from.to_string()))
        .respond_with(ResponseTemplate::new(307).insert_header("location", to.as_str()))
        .mount(server)
        .await;
}

async fn ok(server: &MockServer, at: &str) {
    Mock::given(path(at.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_cross_origin_redirect_is_followed_without_caller_headers() {
    let provider = MockServer::start().await;
    let foreign = MockServer::start().await;
    redirect(&provider, "/call", format!("{}/moved", foreign.uri())).await;
    ok(&foreign, "/moved").await;

    post(credentialed(format!("{}/call", provider.uri()))).await;

    let first = only_request(&provider).await;
    assert!(has(&first, "authorization") && has(&first, "x-api-key"));
    let hop = only_request(&foreign).await;
    assert_eq!(hop.method.as_str(), "POST", "307 keeps the method");
    assert!(!has(&hop, "authorization"));
    assert!(!has(&hop, "x-api-key"));
    let agent = hop.headers["user-agent"].to_str().unwrap();
    assert!(agent.starts_with("caller-agent"), "{agent}");
}

#[tokio::test]
async fn a_same_origin_redirect_keeps_caller_headers() {
    let provider = MockServer::start().await;
    redirect(&provider, "/call", "/moved".into()).await;
    ok(&provider, "/moved").await;

    post(credentialed(format!("{}/call", provider.uri()))).await;

    let received = provider.received_requests().await.unwrap();
    assert_eq!(received.len(), 2);
    assert!(has(&received[1], "authorization") && has(&received[1], "x-api-key"));
}

#[tokio::test]
async fn headers_dropped_off_origin_do_not_return_on_a_hop_back() {
    let provider = MockServer::start().await;
    let foreign = MockServer::start().await;
    redirect(&provider, "/call", format!("{}/bounce", foreign.uri())).await;
    redirect(&foreign, "/bounce", format!("{}/back", provider.uri())).await;
    ok(&provider, "/back").await;

    post(credentialed(format!("{}/call", provider.uri()))).await;

    let received = provider.received_requests().await.unwrap();
    assert_eq!(received.len(), 2);
    assert_eq!(received[1].url.path(), "/back");
    assert!(!has(&received[1], "authorization"));
    assert!(!has(&received[1], "x-api-key"));
}

#[tokio::test]
async fn a_signing_transport_does_not_sign_a_cross_origin_hop() {
    let provider = MockServer::start().await;
    let foreign = MockServer::start().await;
    redirect(&provider, "/call", format!("{}/moved", foreign.uri())).await;
    ok(&foreign, "/moved").await;
    let signing = SigV4Fetch::new(
        default_fetch(),
        Resolvable::from(AwsCredentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "secret".into(),
            session_token: Some("SESSION".into()),
            region: "us-east-1".into(),
        }),
        "bedrock",
        SigV4Signing::Aws4Fetch,
    );

    post(HttpRequest {
        url: format!("{}/call", provider.uri()),
        fetch: Some(Arc::new(signing)),
        ..Default::default()
    })
    .await;

    let first = only_request(&provider).await;
    assert!(has(&first, "authorization") && has(&first, "x-amz-security-token"));
    let hop = only_request(&foreign).await;
    for name in ["authorization", "x-amz-security-token", "x-amz-date"] {
        assert!(!has(&hop, name), "{name}");
    }
}

#[tokio::test]
async fn a_303_turns_the_hop_into_a_get() {
    let provider = MockServer::start().await;
    let foreign = MockServer::start().await;
    Mock::given(path("/call"))
        .respond_with(
            ResponseTemplate::new(303)
                .insert_header("location", format!("{}/moved", foreign.uri())),
        )
        .mount(&provider)
        .await;
    Mock::given(method("GET"))
        .and(path("/moved"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&foreign)
        .await;

    post(credentialed(format!("{}/call", provider.uri()))).await;

    assert_eq!(only_request(&foreign).await.method.as_str(), "GET");
}
