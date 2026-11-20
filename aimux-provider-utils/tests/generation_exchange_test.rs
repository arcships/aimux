//! `ExchangeKind::Generation`: a synchronous generation is not cut at the 30s
//! hang guard of an ordinary exchange, and a failure after the request may
//! have reached the server is not retryable, so the operation retry does not
//! submit (and bill) the generation again (#527).
//!
//! The clock is paused: a transport that answers after 35 virtual seconds
//! takes no real time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use http::{HeaderMap, StatusCode};
use serde_json::json;

use aimux_core::options::TimeoutConfiguration;
use aimux_core::{AiMuxError, SourceError};
use aimux_provider_utils::{
    ExchangeKind, Fetch, FetchError, FetchRequest, FetchResponse, HttpRequest,
    create_json_response_handler, create_status_code_error_response_handler, post_json_to_api,
};

/// What the scripted transport does after its delay.
#[derive(Clone)]
enum Reply {
    Json(u16, serde_json::Value),
    Fail(fn() -> FetchError),
    /// A 200 whose body breaks while it is read.
    BrokenBody,
}

struct SlowFetch {
    delay: Duration,
    reply: Reply,
    calls: AtomicUsize,
}

impl SlowFetch {
    fn new(delay: Duration, reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            delay,
            reply,
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl Fetch for SlowFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        match &self.reply {
            Reply::Json(status, body) => Ok(FetchResponse::from_bytes(
                StatusCode::from_u16(*status).unwrap(),
                HeaderMap::new(),
                request.url,
                Bytes::from(body.to_string()),
            )),
            Reply::Fail(error) => Err(error()),
            Reply::BrokenBody => Ok(FetchResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                url: request.url,
                body: futures::stream::iter([
                    Ok(Bytes::from_static(b"{\"data\":")),
                    Err(FetchError::Io(SourceError::from("connection reset"))),
                ])
                .boxed(),
            }),
        }
    }
}

fn request(fetch: &Arc<SlowFetch>) -> HttpRequest {
    HttpRequest {
        url: "https://api.example.test/v1/images/generations".into(),
        fetch: Some(fetch.clone()),
        ..Default::default()
    }
}

async fn send(request: HttpRequest) -> Result<serde_json::Value, AiMuxError> {
    post_json_to_api(
        request,
        json!({"prompt": "otter"}),
        create_json_response_handler::<serde_json::Value>(),
        create_status_code_error_response_handler(),
    )
    .await
    .map(|output| output.value)
}

fn retryable(error: &AiMuxError) -> bool {
    match error {
        AiMuxError::ApiCall(detail) => detail.is_retryable,
        other => panic!("expected an ApiCall error, got {other:?}"),
    }
}

fn message(error: &AiMuxError) -> &str {
    match error {
        AiMuxError::ApiCall(detail) => &detail.message,
        other => panic!("expected an ApiCall error, got {other:?}"),
    }
}

/// An ordinary exchange keeps its 30s guard, and tripping it stays retryable.
#[tokio::test(start_paused = true)]
async fn a_standard_exchange_still_times_out_at_30s_as_retryable() {
    let fetch = SlowFetch::new(Duration::from_secs(35), Reply::Json(200, json!({})));
    let error = send(request(&fetch)).await.unwrap_err();
    assert_eq!(
        message(&error),
        "Request exceeded the 30s exchange response timeout"
    );
    assert!(retryable(&error));
}

/// A generation that answers after 35s succeeds: its guard defaults to 10
/// minutes.
#[tokio::test(start_paused = true)]
async fn a_generation_slower_than_30s_succeeds() {
    let fetch = SlowFetch::new(
        Duration::from_secs(35),
        Reply::Json(200, json!({"data": [{"b64_json": "AAAA"}]})),
    );
    let value = send(request(&fetch).generation(None)).await.unwrap();
    assert_eq!(value["data"][0]["b64_json"], "AAAA");
    assert_eq!(fetch.calls.load(Ordering::SeqCst), 1);
}

/// Tripping a generation's guard is not retryable: the server may already be
/// running (and billing) the generation.
#[tokio::test(start_paused = true)]
async fn a_generation_past_its_guard_is_not_retryable() {
    let fetch = SlowFetch::new(Duration::from_secs(11 * 60), Reply::Json(200, json!({})));
    let error = send(request(&fetch).generation(None)).await.unwrap_err();
    assert_eq!(
        message(&error),
        "Request exceeded the 600s exchange response timeout"
    );
    assert!(!retryable(&error));
}

/// A guard the provider declares is kept, and is not retryable either.
#[tokio::test(start_paused = true)]
async fn a_declared_guard_is_kept_for_a_generation() {
    let fetch = SlowFetch::new(Duration::from_secs(2), Reply::Json(200, json!({})));
    let mut slow = request(&fetch);
    slow.response_timeout = Some(Duration::from_secs(1));
    let error = send(slow.generation(None)).await.unwrap_err();
    assert_eq!(
        message(&error),
        "Request exceeded the 1s exchange response timeout"
    );
    assert!(!retryable(&error));
}

/// A failure that may come after the request reached the server is not
/// retryable for a generation; a refused connection (nothing was sent) and
/// an HTTP status keep their usual retryability.
#[tokio::test(start_paused = true)]
async fn a_generation_is_replayed_only_when_the_server_cannot_have_seen_it() {
    let cases: [(Reply, bool); 5] = [
        (Reply::Fail(|| FetchError::Timeout), false),
        (
            Reply::Fail(|| FetchError::Io(SourceError::from("broken pipe"))),
            false,
        ),
        (Reply::BrokenBody, false),
        (
            Reply::Fail(|| FetchError::Connect(SourceError::from("connection refused"))),
            true,
        ),
        (Reply::Json(503, json!({"error": "overloaded"})), true),
    ];
    for (reply, expected) in cases {
        let fetch = SlowFetch::new(Duration::ZERO, reply.clone());
        let error = send(request(&fetch).generation(None)).await.unwrap_err();
        assert_eq!(retryable(&error), expected, "{error:?}");

        // The same failure on an ordinary exchange is retryable.
        let fetch = SlowFetch::new(Duration::ZERO, reply);
        let error = send(request(&fetch)).await.unwrap_err();
        assert!(retryable(&error), "{error:?}");
    }
}

/// The guard is never tighter than the operation's own deadline: a longer
/// `totalMs` / `stepMs` widens it, a shorter one leaves the guard alone (the
/// Core deadline ends the call first).
#[test]
fn the_operation_deadline_widens_the_generation_guard() {
    let base = HttpRequest::default();
    let with = |total_ms: Option<u64>, step_ms: Option<u64>| TimeoutConfiguration {
        total_ms,
        step_ms,
        ..TimeoutConfiguration::default()
    };

    let request = base.clone().generation(None);
    assert_eq!(request.kind, ExchangeKind::Generation);
    assert_eq!(request.response_timeout, None);
    assert_eq!(
        request.kind.default_response_timeout(),
        Duration::from_secs(600)
    );

    let request = base
        .clone()
        .generation(Some(&with(Some(20 * 60_000), None)));
    assert_eq!(request.response_timeout, Some(Duration::from_secs(20 * 60)));

    // The tighter of total and step bounds one attempt.
    let request = base
        .clone()
        .generation(Some(&with(Some(30 * 60_000), Some(15 * 60_000))));
    assert_eq!(request.response_timeout, Some(Duration::from_secs(15 * 60)));

    let request = base.clone().generation(Some(&with(Some(60_000), None)));
    assert_eq!(request.response_timeout, None);

    let mut declared = base;
    declared.response_timeout = Some(Duration::from_secs(65));
    let request = declared.clone().generation(Some(&with(Some(60_000), None)));
    assert_eq!(request.response_timeout, Some(Duration::from_secs(65)));
    let request = declared.generation(Some(&with(Some(120_000), None)));
    assert_eq!(request.response_timeout, Some(Duration::from_secs(120)));
}
