//! A `Fetch` injected on the request replaces the network leaf for every
//! helper (`post_json_to_api`, `get_from_api`), while the layers above it —
//! header insertion, error mapping, abort, the download guard — keep working.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::{AbortSignal, AiMuxError};
use aimux_provider_utils::{
    AwsCredentials, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HttpRequest,
    ProviderErrorParts, SigV4Fetch, create_json_error_response_handler,
    create_json_response_handler, default_fetch, get_from_api, post_json_to_api,
};

#[derive(Debug, Deserialize, PartialEq)]
struct Reply {
    answer: u32,
}

/// Records every request and replies with a canned result.
struct MockFetch {
    requests: Mutex<Vec<FetchRequest>>,
    reply: Box<dyn Fn() -> Result<(StatusCode, String), FetchError> + Send + Sync>,
}

impl MockFetch {
    fn json(status: u16, body: serde_json::Value) -> Arc<Self> {
        let body = body.to_string();
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            reply: Box::new(move || Ok((StatusCode::from_u16(status).unwrap(), body.clone()))),
        })
    }

    fn failing(error: FetchError) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            reply: Box::new(move || Err(error.clone())),
        })
    }

    fn calls(&self) -> Vec<FetchRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Fetch for MockFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let url = request.url.clone();
        self.requests.lock().unwrap().push(request);
        let (status, body) = (self.reply)()?;
        Ok(FetchResponse::from_bytes(
            status,
            HeaderMap::new(),
            url,
            Bytes::from(body),
        ))
    }
}

fn failed() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    create_json_error_response_handler(|value| ProviderErrorParts {
        message: value["error"]
            .as_str()
            .unwrap_or("request failed")
            .to_string(),
        provider_code: None,
    })
}

fn request(url: &str, fetch: &Arc<MockFetch>) -> HttpRequest {
    let fetch: FetchFunction = fetch.clone();
    HttpRequest {
        url: url.to_string(),
        headers: vec![("x-custom".into(), "visible".into())],
        fetch: Some(fetch),
        ..Default::default()
    }
}

#[tokio::test]
async fn ordinary_api_does_not_follow_cross_origin_redirects() {
    let source = MockServer::start().await;
    let target = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .and(header("x-api-key", "secret"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("Location", format!("{}/target", target.uri()))
                .set_body_json(json!({"error": "redirect blocked"})),
        )
        .expect(1)
        .mount(&source)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answer": 1})))
        .expect(0)
        .mount(&target)
        .await;

    let error = get_from_api(
        HttpRequest {
            url: format!("{}/start", source.uri()),
            headers: vec![("x-api-key".into(), "secret".into())],
            ..Default::default()
        },
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, AiMuxError::ApiCall(ref detail)
        if detail.status_code == Some(307) && detail.message == "redirect blocked"));
    assert!(target.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_same_origin_redirect_is_followed_and_signed_again() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(307).insert_header("Location", "/target"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/target"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answer": 1})))
        .expect(1)
        .mount(&server)
        .await;
    let fetch = SigV4Fetch::new(
        default_fetch(),
        AwsCredentials {
            access_key_id: "test-key".into(),
            secret_access_key: "test-secret".into(),
            session_token: Some("test-token".into()),
            region: "us-east-1".into(),
        }
        .into(),
        "bedrock",
    );
    get_from_api(
        HttpRequest {
            url: format!("{}/start", server.uri()),
            fetch: Some(Arc::new(fetch)),
            ..Default::default()
        },
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();

    // Each hop went through the signing transport: the signature covers the
    // path, so the two requests carry different ones.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_ne!(
        requests[0].headers["authorization"],
        requests[1].headers["authorization"]
    );
}

#[tokio::test]
async fn post_json_to_api_goes_through_the_injected_fetch() {
    let mock = MockFetch::json(200, json!({"answer": 42}));

    let output = post_json_to_api(
        request("https://api.example.test/v1/chat?alt=json", &mock),
        json!({"hello": "world"}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();

    assert_eq!(output.value, Reply { answer: 42 });
    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    let sent = &calls[0];
    assert_eq!(sent.method, http::Method::POST);
    assert_eq!(
        sent.url.as_str(),
        "https://api.example.test/v1/chat?alt=json"
    );
    assert_eq!(sent.headers["x-custom"], "visible");
    assert_eq!(sent.headers["content-type"], "application/json");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&sent.body).unwrap(),
        json!({"hello": "world"})
    );
}

#[tokio::test]
async fn get_from_api_goes_through_the_injected_fetch() {
    let mock = MockFetch::json(200, json!({"answer": 7}));

    let output = get_from_api(
        request("https://api.example.test/v1/models", &mock),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();

    assert_eq!(output.value, Reply { answer: 7 });
    let calls = mock.calls();
    assert_eq!(calls[0].method, http::Method::GET);
    assert!(calls[0].body.is_empty());
}

#[tokio::test]
async fn same_named_headers_are_inserted_not_appended() {
    let mock = MockFetch::json(200, json!({"answer": 1}));
    let mut http_request = request("https://api.example.test/v1/chat", &mock);
    http_request.headers = vec![
        ("Authorization".into(), "Bearer provider".into()),
        ("x-keep".into(), "1".into()),
        ("authorization".into(), "Bearer call".into()),
    ];

    post_json_to_api(
        http_request,
        json!({}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();

    let sent = &mock.calls()[0];
    let values: Vec<_> = sent.headers.get_all("authorization").iter().collect();
    assert_eq!(
        values.len(),
        1,
        "one Authorization header: {:?}",
        sent.headers
    );
    assert_eq!(values[0], "Bearer call");
    assert_eq!(sent.headers["x-keep"], "1");
}

#[tokio::test]
async fn http_error_statuses_reach_the_failed_response_handler() {
    let mock = MockFetch::json(429, json!({"error": "slow down"}));

    let error = post_json_to_api(
        request("https://api.example.test/v1/chat", &mock),
        json!({}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap_err();

    match error {
        AiMuxError::ApiCall(detail) => {
            assert_eq!(detail.status_code, Some(429));
            assert!(detail.is_retryable);
            assert!(detail.message.contains("slow down"));
        }
        other => panic!("expected ApiCall, got {other:?}"),
    }
}

#[tokio::test]
async fn transport_failures_become_retryable_status_less_api_calls() {
    let mock = MockFetch::failing(FetchError::Connect("connection refused".into()));

    let error = post_json_to_api(
        request("https://api.example.test/v1/chat?key=secret", &mock),
        json!({}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap_err();

    match error {
        AiMuxError::ApiCall(detail) => {
            assert!(detail.is_retryable);
            assert_eq!(detail.status_code, None);
            assert!(detail.message.contains("connection refused"));
            assert!(
                !detail.url.contains("secret"),
                "query must be stripped: {}",
                detail.url
            );
        }
        other => panic!("expected ApiCall, got {other:?}"),
    }
}

#[tokio::test]
async fn an_aborted_signal_wins_before_the_fetch_is_called() {
    let mock = MockFetch::json(200, json!({"answer": 1}));
    let signal = AbortSignal::new();
    signal.abort();
    let mut http_request = request("https://api.example.test/v1/chat", &mock);
    http_request.abort_signal = Some(signal);

    let error = post_json_to_api(
        http_request,
        json!({}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, AiMuxError::Aborted(_)), "{error:?}");
    assert!(mock.calls().is_empty());
}

/// D26: a validated download is never routed through a provider-injected
/// transport — it uses the guard's pinned transport instead.
#[tokio::test]
async fn validate_url_ignores_the_injected_fetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/asset"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answer": 9})))
        .mount(&server)
        .await;
    let mock = MockFetch::json(200, json!({"answer": 0}));
    let mut http_request = request(&format!("{}/asset", server.uri()), &mock);
    http_request.headers = vec![];
    http_request.validate_url = true;
    http_request.trusted_origin = Some(server.uri());

    let output = get_from_api(
        http_request,
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();

    assert_eq!(output.value, Reply { answer: 9 });
    assert!(
        mock.calls().is_empty(),
        "the injected fetch must not see downloads"
    );
}
