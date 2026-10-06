//! Validated downloads use the guarded transport instead of an injected fetch.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::AiMuxError;
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HttpRequest, ProviderErrorParts,
    create_json_error_response_handler, create_json_response_handler, get_from_api,
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
