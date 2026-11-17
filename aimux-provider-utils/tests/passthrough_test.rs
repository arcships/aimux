use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
use sha2::{Digest, Sha256};
use url::Url;

use aimux_core::{AbortSignal, SourceError};
use aimux_provider_utils::{
    AuthScheme, AwsCredentials, Credential, Fetch, FetchError, FetchFunction, FetchRequest,
    FetchResponse, NativeContext, NativeEndpoint, NativeError, NativeExchangeFinished,
    NativeExchangeObserver, NativeExchangeOutcome, NativeExchangeStarted, NativeRequest,
    NativeRetryPolicy, ProviderHeaders, ReqwestFetch, Resolvable, SIGV4_PROTECTED_HEADERS,
    SafeRetryOptions, SigV4Fetch, SigV4Signing, native_request,
};

type FetchHandler = Arc<
    dyn Fn(
            FetchRequest,
        )
            -> Pin<Box<dyn std::future::Future<Output = Result<FetchResponse, FetchError>> + Send>>
        + Send
        + Sync,
>;

struct MockFetch {
    calls: Arc<Mutex<Vec<RecordedRequest>>>,
    handler: FetchHandler,
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Bytes,
}

impl MockFetch {
    fn new<F, Fut>(handler: F) -> (Arc<Self>, Arc<Mutex<Vec<RecordedRequest>>>)
    where
        F: Fn(FetchRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<FetchResponse, FetchError>> + Send + 'static,
    {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_ref = calls.clone();
        let mock = Arc::new(Self {
            calls: calls.clone(),
            handler: Arc::new(move |req| Box::pin(handler(req))),
        });
        let _ = calls_ref;
        (mock, calls)
    }
}

#[async_trait]
impl Fetch for MockFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.calls.lock().unwrap().push(RecordedRequest {
            method: request.method.clone(),
            url: request.url.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
        });
        (self.handler)(request).await
    }
}

#[derive(Default)]
struct RecordingObserver {
    started: Mutex<Vec<NativeExchangeStarted>>,
    heads: Mutex<Vec<(u64, u16, HeaderMap, String)>>,
    chunks: Mutex<Vec<(u64, Bytes)>>,
    finished: Mutex<Vec<NativeExchangeFinished>>,
    fail_on_started: AtomicBool,
    fail_on_head: AtomicBool,
    fail_on_chunk_at: Option<usize>,
}

impl RecordingObserver {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn failing_after_chunk(n: usize) -> Arc<Self> {
        Arc::new(Self {
            fail_on_chunk_at: Some(n),
            ..Self::default()
        })
    }

    fn started_events(&self) -> Vec<NativeExchangeStarted> {
        self.started.lock().unwrap().clone()
    }

    fn finished_events(&self) -> Vec<NativeExchangeFinished> {
        self.finished.lock().unwrap().clone()
    }

    fn chunk_count(&self) -> usize {
        self.chunks.lock().unwrap().len()
    }
}

impl NativeExchangeObserver for RecordingObserver {
    fn on_exchange_started(&self, event: &NativeExchangeStarted) -> Result<(), String> {
        self.started.lock().unwrap().push(event.clone());
        if self.fail_on_started.load(Ordering::SeqCst) {
            return Err("sink failed on start".to_string());
        }
        Ok(())
    }

    fn on_response_head(
        &self,
        exchange_id: u64,
        status: StatusCode,
        headers: &HeaderMap,
        sanitized_effective_url: &str,
    ) -> Result<(), String> {
        self.heads.lock().unwrap().push((
            exchange_id,
            status.as_u16(),
            headers.clone(),
            sanitized_effective_url.to_string(),
        ));
        if self.fail_on_head.load(Ordering::SeqCst) {
            return Err("sink failed on head".to_string());
        }
        Ok(())
    }

    fn on_response_chunk(&self, exchange_id: u64, chunk: &Bytes) -> Result<(), String> {
        let mut chunks = self.chunks.lock().unwrap();
        let next_idx = chunks.len() + 1;
        if let Some(fail_at) = self.fail_on_chunk_at
            && next_idx >= fail_at
        {
            return Err("sink io failure".to_string());
        }
        chunks.push((exchange_id, chunk.clone()));
        Ok(())
    }

    fn on_exchange_finished(&self, event: NativeExchangeFinished) {
        self.finished.lock().unwrap().push(event);
    }
}

struct DropTrackingStream {
    items: std::collections::VecDeque<Result<Bytes, FetchError>>,
    poll_count: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    pending_forever_after_empty: bool,
}

impl Stream for DropTrackingStream {
    type Item = Result<Bytes, FetchError>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.poll_count.fetch_add(1, Ordering::SeqCst);
        match self.items.pop_front() {
            Some(item) => Poll::Ready(Some(item)),
            None if self.pending_forever_after_empty => Poll::Pending,
            None => Poll::Ready(None),
        }
    }
}

impl Drop for DropTrackingStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

async fn drain_body(
    mut stream: aimux_provider_utils::NativeBodyStream,
) -> Result<Bytes, NativeError> {
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let chunk = item?;
        out.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(out))
}

#[tokio::test]
async fn n01_byte_fidelity_non_2xx_sse_multi_headers_and_golden_urls() {
    let req_bytes: Vec<u8> = (0u8..=255).collect();
    let resp_bytes: Vec<u8> = (0u8..=255).rev().collect();
    let req_hash = Sha256::digest(&req_bytes);
    let resp_hash = Sha256::digest(&resp_bytes);

    let resp_bytes_clone = Bytes::from(resp_bytes.clone());
    let (mock, calls) = MockFetch::new(move |req| {
        let resp_bytes_clone = resp_bytes_clone.clone();
        async move {
            let mut headers = HeaderMap::new();
            headers.append("x-multi", HeaderValue::from_static("alpha"));
            headers.append("x-multi", HeaderValue::from_static("beta"));
            headers.insert(
                "content-type",
                HeaderValue::from_static("application/octet-stream"),
            );
            Ok(FetchResponse::from_bytes(
                StatusCode::CREATED,
                headers,
                req.url,
                resp_bytes_clone,
            ))
        }
    });

    // Golden URLs: /v1 and /v1/ both preserve prefix and query encoding/order verbatim.
    for base in ["https://api.example.com/v1", "https://api.example.com/v1/"] {
        let endpoint = NativeEndpoint::with_provider_headers(
            base,
            ProviderHeaders::bearer(
                Credential::Explicit(Resolvable::Value("test-key".to_string())),
                vec![],
                None,
            ),
        )
        .unwrap()
        .with_fetch(mock.clone());

        let mut req_headers = HeaderMap::new();
        req_headers.append("x-req-multi", HeaderValue::from_static("first"));
        req_headers.append("x-req-multi", HeaderValue::from_static("second"));
        req_headers.insert(
            "content-type",
            HeaderValue::from_static("application/octet-stream"),
        );

        let req = NativeRequest::new(Method::POST, "responses?x=1&x=2&enc=%2F")
            .with_headers(req_headers)
            .with_body(req_bytes.clone());

        let resp = native_request(&endpoint, req, NativeContext::new())
            .await
            .unwrap();
        assert_eq!(resp.status, StatusCode::CREATED);
        assert_eq!(
            resp.effective_url.as_str(),
            "https://api.example.com/v1/responses?x=1&x=2&enc=%2F"
        );
        let resp_multi: Vec<_> = resp
            .headers
            .get_all("x-multi")
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        assert_eq!(resp_multi, ["alpha", "beta"]);
        let got = drain_body(resp.body).await.unwrap();
        assert_eq!(Sha256::digest(&got), resp_hash);
    }

    // Golden root-path and query-only examples.
    let root_ep = NativeEndpoint::unauthenticated("https://api.example.com").unwrap();
    assert_eq!(
        root_ep.resolve_url("v1/models").unwrap().as_str(),
        "https://api.example.com/v1/models"
    );
    let v1_ep = NativeEndpoint::unauthenticated("https://api.example.com/v1").unwrap();
    assert_eq!(
        v1_ep.resolve_url("?dry_run=true").unwrap().as_str(),
        "https://api.example.com/v1?dry_run=true"
    );

    {
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        for call in recorded.iter() {
            assert_eq!(call.method, Method::POST);
            assert_eq!(
                call.url.as_str(),
                "https://api.example.com/v1/responses?x=1&x=2&enc=%2F"
            );
            assert_eq!(Sha256::digest(&call.body), req_hash);
            let multi: Vec<_> = call
                .headers
                .get_all("x-req-multi")
                .iter()
                .map(|v| v.to_str().unwrap())
                .collect();
            assert_eq!(multi, ["first", "second"]);
            assert_eq!(
                call.headers.get("authorization").unwrap().to_str().unwrap(),
                "Bearer test-key"
            );
        }
    }

    // Non-2xx non-JSON, SSE stream, and 302 redirect are returned raw without model error or redirect following.
    let (mock_status, status_calls) = MockFetch::new(move |req| async move {
        let path = req.url.path().to_string();
        if path.ends_with("/bad") {
            let mut h = HeaderMap::new();
            h.insert("content-type", HeaderValue::from_static("text/plain"));
            Ok(FetchResponse::from_bytes(
                StatusCode::BAD_REQUEST,
                h,
                req.url,
                Bytes::from_static(b"plain bad \xff\x00"),
            ))
        } else if path.ends_with("/sse") {
            let mut h = HeaderMap::new();
            h.insert(
                "content-type",
                HeaderValue::from_static("text/event-stream"),
            );
            Ok(FetchResponse::from_bytes(
                StatusCode::OK,
                h,
                req.url,
                Bytes::from_static(b"data: {\"a\":1}\n\n: comment\n\ndata: [DONE]\n\n"),
            ))
        } else {
            let mut h = HeaderMap::new();
            h.insert(
                "location",
                HeaderValue::from_static("https://evil.test/steal"),
            );
            Ok(FetchResponse::from_bytes(
                StatusCode::FOUND,
                h,
                req.url,
                Bytes::from_static(b"redirect body"),
            ))
        }
    });

    let ep = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(mock_status);

    let r_bad = native_request(
        &ep,
        NativeRequest::new(Method::GET, "bad"),
        NativeContext::new(),
    )
    .await
    .unwrap();
    assert_eq!(r_bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        drain_body(r_bad.body).await.unwrap().as_ref(),
        b"plain bad \xff\x00"
    );

    let r_sse = native_request(
        &ep,
        NativeRequest::new(Method::POST, "sse"),
        NativeContext::new(),
    )
    .await
    .unwrap();
    assert_eq!(r_sse.status, StatusCode::OK);
    assert_eq!(
        drain_body(r_sse.body).await.unwrap().as_ref(),
        b"data: {\"a\":1}\n\n: comment\n\ndata: [DONE]\n\n"
    );

    let r_redir = native_request(
        &ep,
        NativeRequest::new(Method::GET, "redir"),
        NativeContext::new(),
    )
    .await
    .unwrap();
    assert_eq!(r_redir.status, StatusCode::FOUND);
    assert_eq!(
        r_redir.headers.get("location").unwrap().to_str().unwrap(),
        "https://evil.test/steal"
    );
    assert_eq!(status_calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn n02_url_traversal_and_protected_header_guards_zero_network_or_auth() {
    let cred_evals = Arc::new(AtomicUsize::new(0));
    let cred_evals_ref = cred_evals.clone();
    let resolvable = Resolvable::from_async_fn(move || {
        let evals = cred_evals_ref.clone();
        async move {
            evals.fetch_add(1, Ordering::SeqCst);
            Ok("secret-token".to_string())
        }
    });

    let (mock, fetch_calls) = MockFetch::new(|req| async move {
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            HeaderMap::new(),
            req.url,
            Bytes::new(),
        ))
    });

    let bearer_ep = NativeEndpoint::with_provider_headers(
        "https://api.example.com/v1",
        ProviderHeaders::bearer(Credential::Explicit(resolvable), vec![], None),
    )
    .unwrap()
    .with_fetch(mock.clone());

    let invalid_paths = [
        "https://evil.test/x",
        "http:evil",
        "//evil.test/x",
        "/root-escape",
        "../admin",
        "a/../../admin",
        ".",
        "a/./b",
        "%2e%2e/admin",
        "a/%2E./b",
        "a/.%2e/b",
        "a/%2e/b",
        "a/%2f../b",
        "a\\b",
        "a/%5cb",
        "a/%00b",
        "a/%0Ab",
        "responses#frag",
        "a//b",
        "responses%",
        "responses%2",
        "responses%ZZ",
        "responses?a=1\n",
        " responses",
    ];
    for bad in invalid_paths {
        let err = native_request(
            &bearer_ep,
            NativeRequest::new(Method::GET, bad),
            NativeContext::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, NativeError::InvalidPath { .. }),
            "expected InvalidPath for `{bad}`, got {err:?}"
        );
    }

    // Legitimate Vertex-style colon in later segment passes.
    assert_eq!(
        bearer_ep
            .resolve_url("models/gemini-2.5-flash:generateContent")
            .unwrap()
            .as_str(),
        "https://api.example.com/v1/models/gemini-2.5-flash:generateContent"
    );

    // Caller overrides protected Authorization (case-insensitive) -> InvalidHeader, 0 auth/fetch.
    let mut bad_headers = HeaderMap::new();
    bad_headers.insert("Authorization", HeaderValue::from_static("Bearer attacker"));
    let err = native_request(
        &bearer_ep,
        NativeRequest::new(Method::POST, "responses").with_headers(bad_headers),
        NativeContext::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, NativeError::InvalidHeader { .. }));
    assert_eq!(cred_evals.load(Ordering::SeqCst), 0);
    assert_eq!(fetch_calls.lock().unwrap().len(), 0);

    // Header auth scheme protects x-api-key.
    let api_key_ep = NativeEndpoint::with_provider_headers(
        "https://api.anthropic.com/v1",
        ProviderHeaders::new(
            Credential::Explicit(Resolvable::Value("k".to_string())),
            AuthScheme::Header("x-api-key"),
            vec![],
            None,
        ),
    )
    .unwrap()
    .with_fetch(mock.clone());
    let mut key_override = HeaderMap::new();
    key_override.insert("X-API-Key", HeaderValue::from_static("spoof"));
    let err = native_request(
        &api_key_ep,
        NativeRequest::new(Method::POST, "messages").with_headers(key_override),
        NativeContext::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, NativeError::InvalidHeader { .. }));

    // SigV4-protected headers cannot be overridden; valid request gets SigV4 headers signed over body bytes.
    let (inner_capture, sigv4_wire_calls) = MockFetch::new(|req| async move {
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            HeaderMap::new(),
            req.url,
            Bytes::from_static(b"signed-ok"),
        ))
    });
    let sigv4: FetchFunction = Arc::new(SigV4Fetch::new(
        inner_capture,
        Resolvable::Value(AwsCredentials {
            access_key_id: "AKIAEXAMPLE".to_string(),
            secret_access_key: "secret-aws-key".to_string(),
            session_token: Some("sess-token".to_string()),
            region: "us-east-1".to_string(),
        }),
        "bedrock",
        SigV4Signing::AllHeaders,
    ));
    let aws_ep = NativeEndpoint::unauthenticated("https://bedrock-runtime.us-east-1.amazonaws.com")
        .unwrap()
        .try_with_protected_headers(SIGV4_PROTECTED_HEADERS.iter().copied())
        .unwrap()
        .with_fetch(sigv4);

    for protected in SIGV4_PROTECTED_HEADERS {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::try_from(*protected).unwrap(),
            HeaderValue::from_static("spoof"),
        );
        let err = native_request(
            &aws_ep,
            NativeRequest::new(Method::POST, "model/m/invoke").with_headers(h),
            NativeContext::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, NativeError::InvalidHeader { .. }));
    }
    assert_eq!(sigv4_wire_calls.lock().unwrap().len(), 0);

    let r = native_request(
        &aws_ep,
        NativeRequest::new(Method::POST, "model/m/invoke?x=1")
            .with_body(Bytes::from_static(b"payload")),
        NativeContext::new(),
    )
    .await
    .unwrap();
    assert_eq!(drain_body(r.body).await.unwrap().as_ref(), b"signed-ok");
    {
        let aws_calls = sigv4_wire_calls.lock().unwrap();
        assert_eq!(aws_calls.len(), 1);
        assert!(aws_calls[0].headers.contains_key("authorization"));
        assert!(aws_calls[0].headers.contains_key("x-amz-date"));
        assert_eq!(
            aws_calls[0]
                .headers
                .get("x-amz-security-token")
                .unwrap()
                .to_str()
                .unwrap(),
            "sess-token"
        );
    }

    // Unauthenticated endpoint permits normal headers and explicit policy can protect authorization.
    let unauth_ep = NativeEndpoint::unauthenticated("https://local.example.com/v1")
        .unwrap()
        .with_fetch(mock.clone());
    let mut normal = HeaderMap::new();
    normal.insert("x-custom", HeaderValue::from_static("ok"));
    let r = native_request(
        &unauth_ep,
        NativeRequest::new(Method::GET, "health").with_headers(normal),
        NativeContext::new(),
    )
    .await
    .unwrap();
    assert_eq!(r.status, StatusCode::OK);

    let strict_unauth_ep = NativeEndpoint::unauthenticated("https://local.example.com/v1")
        .unwrap()
        .try_with_protected_headers(["authorization"])
        .unwrap()
        .with_fetch(mock);
    let mut auth_h = HeaderMap::new();
    auth_h.insert("authorization", HeaderValue::from_static("Bearer x"));
    let err = native_request(
        &strict_unauth_ep,
        NativeRequest::new(Method::GET, "health").with_headers(auth_h),
        NativeContext::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, NativeError::InvalidHeader { .. }));
}

#[tokio::test]
async fn n03_cancellation_timeout_drop_and_eof_race() {
    // 1. Pre-aborted: 0 auth, 0 fetch, 0 observer.
    let cred_evals = Arc::new(AtomicUsize::new(0));
    let cred_evals_ref = cred_evals.clone();
    let (mock, calls) = MockFetch::new(|req| async move {
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            HeaderMap::new(),
            req.url,
            Bytes::new(),
        ))
    });
    let ep = NativeEndpoint::with_provider_headers(
        "https://api.example.com/v1",
        ProviderHeaders::bearer(
            Credential::Explicit(Resolvable::from_async_fn(move || {
                let evals = cred_evals_ref.clone();
                async move {
                    evals.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok("key".to_string())
                }
            })),
            vec![],
            None,
        ),
    )
    .unwrap()
    .with_fetch(mock);

    let sig = AbortSignal::new();
    sig.abort();
    let obs = RecordingObserver::new();
    let err = native_request(
        &ep,
        NativeRequest::new(Method::GET, "models"),
        NativeContext::new()
            .with_abort_signal(sig)
            .with_observer(obs.clone()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, NativeError::Aborted(_)));
    assert_eq!(cred_evals.load(Ordering::SeqCst), 0);
    assert_eq!(calls.lock().unwrap().len(), 0);
    assert_eq!(obs.started.lock().unwrap().len(), 0);
    assert_eq!(obs.finished.lock().unwrap().len(), 0);

    // 2. Abort while waiting for credential resolver.
    let sig = AbortSignal::new();
    let sig_trigger = sig.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        sig_trigger.abort();
    });
    let err = native_request(
        &ep,
        NativeRequest::new(Method::GET, "models"),
        NativeContext::new().with_abort_signal(sig),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, NativeError::Aborted(_)));
    assert_eq!(cred_evals.load(Ordering::SeqCst), 1);
    assert_eq!(calls.lock().unwrap().len(), 0);

    // 3. Abort while awaiting response headers -> finish emitted once as Aborted.
    let (slow_head_mock, _) = MockFetch::new(|_req| async move {
        std::future::pending::<Result<FetchResponse, FetchError>>().await
    });
    let slow_ep = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(slow_head_mock);
    let sig = AbortSignal::new();
    let sig_trigger = sig.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        sig_trigger.abort();
    });
    let obs = RecordingObserver::new();
    let err = native_request(
        &slow_ep,
        NativeRequest::new(Method::GET, "models"),
        NativeContext::new()
            .with_abort_signal(sig)
            .with_observer(obs.clone()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, NativeError::Aborted(_)));
    let finished = obs.finished_events();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].outcome, NativeExchangeOutcome::Aborted);
    assert!(!finished[0].recording_complete);

    // 4. Abort mid-body stream + drop tracking: stops cleanly without extra chunks.
    let dropped = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let dropped_ref = dropped.clone();
    let polls_ref = polls.clone();
    let (mid_abort_mock, _) = MockFetch::new(move |req| {
        let dropped_ref = dropped_ref.clone();
        let polls_ref = polls_ref.clone();
        async move {
            let mut items = std::collections::VecDeque::new();
            items.push_back(Ok(Bytes::from_static(b"chunk-1")));
            let stream: BoxStream<'static, Result<Bytes, FetchError>> =
                Box::pin(DropTrackingStream {
                    items,
                    poll_count: polls_ref,
                    dropped: dropped_ref,
                    pending_forever_after_empty: true,
                });
            Ok(FetchResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                url: req.url,
                body: stream,
            })
        }
    });
    let ep_mid = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(mid_abort_mock);
    let sig = AbortSignal::new();
    let obs = RecordingObserver::new();
    let mut resp = native_request(
        &ep_mid,
        NativeRequest::new(Method::GET, "stream"),
        NativeContext::new()
            .with_abort_signal(sig.clone())
            .with_observer(obs.clone()),
    )
    .await
    .unwrap();
    let first = resp.body.next().await.unwrap().unwrap();
    assert_eq!(first.as_ref(), b"chunk-1");
    sig.abort();
    let second = resp.body.next().await.unwrap().unwrap_err();
    assert!(matches!(second, NativeError::Aborted(_)));
    assert!(resp.body.next().await.is_none());
    assert!(dropped.load(Ordering::SeqCst));
    let finished = obs.finished_events();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].outcome, NativeExchangeOutcome::Aborted);
    assert!(!finished[0].recording_complete);

    // 5. Drop body mid-stream synchronously drops inner stream without background draining.
    let dropped2 = Arc::new(AtomicBool::new(false));
    let polls2 = Arc::new(AtomicUsize::new(0));
    let dropped2_ref = dropped2.clone();
    let polls2_ref = polls2.clone();
    let (drop_mock, _) = MockFetch::new(move |req| {
        let dropped2_ref = dropped2_ref.clone();
        let polls2_ref = polls2_ref.clone();
        async move {
            let mut items = std::collections::VecDeque::new();
            items.push_back(Ok(Bytes::from_static(b"part-a")));
            items.push_back(Ok(Bytes::from_static(b"part-b")));
            let stream: BoxStream<'static, Result<Bytes, FetchError>> =
                Box::pin(DropTrackingStream {
                    items,
                    poll_count: polls2_ref,
                    dropped: dropped2_ref,
                    pending_forever_after_empty: false,
                });
            Ok(FetchResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                url: req.url,
                body: stream,
            })
        }
    });
    let ep_drop = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(drop_mock);
    let obs_drop = RecordingObserver::new();
    let mut resp = native_request(
        &ep_drop,
        NativeRequest::new(Method::GET, "stream"),
        NativeContext::new().with_observer(obs_drop.clone()),
    )
    .await
    .unwrap();
    assert_eq!(resp.body.next().await.unwrap().unwrap().as_ref(), b"part-a");
    assert_eq!(polls2.load(Ordering::SeqCst), 1);
    drop(resp);
    assert!(dropped2.load(Ordering::SeqCst));
    assert_eq!(polls2.load(Ordering::SeqCst), 1);
    let fin_drop = obs_drop.finished_events();
    assert_eq!(fin_drop.len(), 1);
    assert_eq!(fin_drop[0].outcome, NativeExchangeOutcome::Aborted);
    assert!(!fin_drop[0].recording_complete);

    // 6. Timeout mid-stream fires NativeError::Timeout and single finish.
    let (timeout_mock, _) = MockFetch::new(|req| async move {
        let stream: BoxStream<'static, Result<Bytes, FetchError>> =
            Box::pin(futures::stream::pending());
        Ok(FetchResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            url: req.url,
            body: stream,
        })
    });
    let ep_to = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(timeout_mock);
    let obs_to = RecordingObserver::new();
    let mut resp = native_request(
        &ep_to,
        NativeRequest::new(Method::GET, "stream"),
        NativeContext::new()
            .with_timeout(Duration::from_millis(25))
            .with_observer(obs_to.clone()),
    )
    .await
    .unwrap();
    let err = resp.body.next().await.unwrap().unwrap_err();
    assert!(matches!(err, NativeError::Timeout(_)));
    assert!(resp.body.next().await.is_none());
    let fin_to = obs_to.finished_events();
    assert_eq!(fin_to.len(), 1);
    assert!(matches!(
        fin_to[0].outcome,
        NativeExchangeOutcome::Failed(_)
    ));

    // 7. Race abort() against EOF 50 times -> always exactly 1 finish per started exchange.
    let (fast_mock, _) = MockFetch::new(|req| async move {
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            HeaderMap::new(),
            req.url,
            Bytes::from_static(b"done"),
        ))
    });
    let ep_fast = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(fast_mock);
    for _ in 0..50 {
        let sig = AbortSignal::new();
        let obs_race = RecordingObserver::new();
        let sig_abort = sig.clone();
        let aborter = tokio::spawn(async move {
            tokio::task::yield_now().await;
            sig_abort.abort();
        });
        if let Ok(resp) = native_request(
            &ep_fast,
            NativeRequest::new(Method::GET, "fast"),
            NativeContext::new()
                .with_abort_signal(sig)
                .with_observer(obs_race.clone()),
        )
        .await
        {
            let _ = drain_body(resp.body).await;
        }
        let _ = aborter.await;
        let started_count = obs_race.started.lock().unwrap().len();
        let finished_count = obs_race.finished.lock().unwrap().len();
        assert_eq!(
            started_count, finished_count,
            "every started exchange must finish exactly once"
        );
        assert!(finished_count <= 1);
    }
}

#[tokio::test]
async fn n04_bounded_backpressure_and_observer_failures() {
    // Infinite 16 KiB stream polled through max_buffer_bytes = 4096.
    let polls = Arc::new(AtomicUsize::new(0));
    let polls_ref = polls.clone();
    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_ref = dropped.clone();

    struct InfiniteBigChunks {
        polls: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
    }
    impl Stream for InfiniteBigChunks {
        type Item = Result<Bytes, FetchError>;
        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let n = self.polls.fetch_add(1, Ordering::SeqCst) + 1;
            let fill = (n % 255) as u8;
            Poll::Ready(Some(Ok(Bytes::from(vec![fill; 16 * 1024]))))
        }
    }
    impl Drop for InfiniteBigChunks {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    let (mock_inf, _) = MockFetch::new(move |req| {
        let polls_ref = polls_ref.clone();
        let dropped_ref = dropped_ref.clone();
        async move {
            let stream: BoxStream<'static, Result<Bytes, FetchError>> =
                Box::pin(InfiniteBigChunks {
                    polls: polls_ref,
                    dropped: dropped_ref,
                });
            Ok(FetchResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                url: req.url,
                body: stream,
            })
        }
    });

    let ep = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .try_with_max_buffer_bytes(4096)
        .unwrap()
        .with_fetch(mock_inf);
    let sig = AbortSignal::new();
    let mut resp = native_request(
        &ep,
        NativeRequest::new(Method::GET, "inf"),
        NativeContext::new().with_abort_signal(sig.clone()),
    )
    .await
    .unwrap();

    // Reads 4 slices of 4096 bytes from the single 16 KiB upstream chunk -> upstream polled only once.
    for _ in 0..4 {
        let slice = resp.body.next().await.unwrap().unwrap();
        assert_eq!(slice.len(), 4096);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    }
    // 5th read pulls the second upstream chunk.
    let slice5 = resp.body.next().await.unwrap().unwrap();
    assert_eq!(slice5.len(), 4096);
    assert_eq!(polls.load(Ordering::SeqCst), 2);
    sig.abort();
    assert!(matches!(
        resp.body.next().await.unwrap().unwrap_err(),
        NativeError::Aborted(_)
    ));
    assert!(dropped.load(Ordering::SeqCst));

    // Failing observer & max_recorded_bytes cap do not interrupt caller body delivery.
    let (mock_chunks, _) = MockFetch::new(|req| async move {
        let stream: BoxStream<'static, Result<Bytes, FetchError>> =
            Box::pin(futures::stream::iter(vec![
                Ok(Bytes::from_static(b"hello-")),
                Ok(Bytes::from_static(b"world-")),
                Ok(Bytes::from_static(b"complete")),
            ]));
        Ok(FetchResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            url: req.url,
            body: stream,
        })
    });
    let ep_obs = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(mock_chunks);

    let failing_obs = RecordingObserver::failing_after_chunk(2);
    let r = native_request(
        &ep_obs,
        NativeRequest::new(Method::GET, "data"),
        NativeContext::new().with_observer(failing_obs.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        drain_body(r.body).await.unwrap().as_ref(),
        b"hello-world-complete"
    );
    assert_eq!(failing_obs.chunk_count(), 1);
    let fin = failing_obs.finished_events();
    assert_eq!(fin.len(), 1);
    assert_eq!(fin[0].outcome, NativeExchangeOutcome::Completed);
    assert!(!fin[0].recording_complete);
    assert_eq!(fin[0].incomplete_reason.as_deref(), Some("sink io failure"));

    let capped_obs = RecordingObserver::new();
    let r_cap = native_request(
        &ep_obs,
        NativeRequest::new(Method::GET, "data"),
        NativeContext::new()
            .with_observer(capped_obs.clone())
            .try_with_max_recorded_bytes(10)
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        drain_body(r_cap.body).await.unwrap().as_ref(),
        b"hello-world-complete"
    );
    assert_eq!(capped_obs.chunk_count(), 1);
    let fin_cap = capped_obs.finished_events();
    assert_eq!(fin_cap.len(), 1);
    assert_eq!(fin_cap[0].outcome, NativeExchangeOutcome::Completed);
    assert!(!fin_cap[0].recording_complete);
    assert!(
        fin_cap[0]
            .incomplete_reason
            .as_deref()
            .is_some_and(|s| s.contains("exceeded limit of 10 bytes"))
    );
}

#[tokio::test]
async fn n05_retry_policy_and_no_stream_stitching() {
    let connect_err = || FetchError::Connect(SourceError::from("connection refused".to_string()));

    // 1. POST with Safe but NO idempotency key -> 1 attempt only.
    let attempt_counter = Arc::new(AtomicUsize::new(0));
    let ac = attempt_counter.clone();
    let (mock_fail, _) = MockFetch::new(move |_| {
        let ac = ac.clone();
        async move {
            ac.fetch_add(1, Ordering::SeqCst);
            Err(FetchError::Connect(SourceError::from(
                "connection refused".to_string(),
            )))
        }
    });
    let ep_no_idem = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(mock_fail);
    let err = native_request(
        &ep_no_idem,
        NativeRequest::new(Method::POST, "responses").with_retry(NativeRetryPolicy::Safe(
            SafeRetryOptions::default()
                .with_max_retries(3)
                .with_initial_backoff(Duration::from_millis(1)),
        )),
        NativeContext::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        NativeError::Transport(FetchError::Connect(_))
    ));
    assert_eq!(attempt_counter.load(Ordering::SeqCst), 1);

    // 2. POST with idempotency_key in options but endpoint did NOT declare idempotency_header -> 1 attempt only.
    attempt_counter.store(0, Ordering::SeqCst);
    let _ = native_request(
        &ep_no_idem,
        NativeRequest::new(Method::POST, "responses").with_retry(NativeRetryPolicy::Safe(
            SafeRetryOptions::default()
                .with_max_retries(3)
                .with_initial_backoff(Duration::from_millis(1))
                .with_idempotency_key(
                    HeaderName::from_static("idempotency-key"),
                    HeaderValue::from_static("k-1"),
                ),
        )),
        NativeContext::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(attempt_counter.load(Ordering::SeqCst), 1);

    // 3. POST with BOTH endpoint idempotency_header and matching key -> retries connect error with distinct exchange IDs.
    let post_attempts = Arc::new(AtomicUsize::new(0));
    let pa = post_attempts.clone();
    let (mock_post_retry, post_calls) = MockFetch::new(move |req| {
        let pa = pa.clone();
        async move {
            let n = pa.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                Err(FetchError::Connect(SourceError::from(
                    "transient connect".to_string(),
                )))
            } else {
                Ok(FetchResponse::from_bytes(
                    StatusCode::OK,
                    HeaderMap::new(),
                    req.url,
                    req.body,
                ))
            }
        }
    });
    let ep_idem = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .try_with_idempotency_header("idempotency-key")
        .unwrap()
        .with_fetch(mock_post_retry);
    let obs_post = RecordingObserver::new();
    let r = native_request(
        &ep_idem,
        NativeRequest::new(Method::POST, "responses")
            .with_body(Bytes::from_static(b"same-body"))
            .with_retry(NativeRetryPolicy::Safe(
                SafeRetryOptions::default()
                    .with_max_retries(2)
                    .with_initial_backoff(Duration::from_millis(1))
                    .with_idempotency_key(
                        HeaderName::from_static("idempotency-key"),
                        HeaderValue::from_static("idem-123"),
                    ),
            )),
        NativeContext::new().with_observer(obs_post.clone()),
    )
    .await
    .unwrap();
    assert_eq!(drain_body(r.body).await.unwrap().as_ref(), b"same-body");
    {
        let pc = post_calls.lock().unwrap();
        assert_eq!(pc.len(), 2);
        assert_eq!(pc[0].body, pc[1].body);
        assert_eq!(
            pc[0]
                .headers
                .get("idempotency-key")
                .unwrap()
                .to_str()
                .unwrap(),
            "idem-123"
        );
        assert_eq!(
            pc[1]
                .headers
                .get("idempotency-key")
                .unwrap()
                .to_str()
                .unwrap(),
            "idem-123"
        );
    }
    let started = obs_post.started_events();
    let finished = obs_post.finished_events();
    assert_eq!(started.len(), 2);
    assert_eq!(finished.len(), 2);
    assert_ne!(started[0].exchange_id, started[1].exchange_id);
    assert_eq!((started[0].attempt, started[1].attempt), (1, 2));

    // 4. GET retries connect + 503 pre-head cleanly without mislabeling discarded 503 as Aborted.
    let get_attempts = Arc::new(AtomicUsize::new(0));
    let ga = get_attempts.clone();
    let (mock_get, _) = MockFetch::new(move |req| {
        let ga = ga.clone();
        async move {
            let n = ga.fetch_add(1, Ordering::SeqCst) + 1;
            match n {
                1 => Err(connect_err()),
                2 => Ok(FetchResponse::from_bytes(
                    StatusCode::SERVICE_UNAVAILABLE,
                    HeaderMap::new(),
                    req.url,
                    Bytes::from_static(b"busy"),
                )),
                _ => Ok(FetchResponse::from_bytes(
                    StatusCode::OK,
                    HeaderMap::new(),
                    req.url,
                    Bytes::from_static(b"ok"),
                )),
            }
        }
    });
    let ep_get = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(mock_get);
    let obs_get = RecordingObserver::new();
    let r_get = native_request(
        &ep_get,
        NativeRequest::new(Method::GET, "models").with_retry(NativeRetryPolicy::Safe(
            SafeRetryOptions::default()
                .with_max_retries(2)
                .with_initial_backoff(Duration::from_millis(1)),
        )),
        NativeContext::new()
            .with_timeout(Duration::from_secs(2))
            .with_observer(obs_get.clone()),
    )
    .await
    .unwrap();
    assert_eq!(drain_body(r_get.body).await.unwrap().as_ref(), b"ok");
    let fin_get = obs_get.finished_events();
    assert_eq!(fin_get.len(), 3);
    assert!(matches!(
        fin_get[0].outcome,
        NativeExchangeOutcome::Failed(_)
    ));
    assert_eq!(fin_get[1].outcome, NativeExchangeOutcome::Completed);
    assert_eq!(fin_get[2].outcome, NativeExchangeOutcome::Completed);
    assert!(fin_get[2].recording_complete);

    // 5. Shared timeout budget bounds retry backoff.
    let (mock_always_503, _) = MockFetch::new(|req| async move {
        Ok(FetchResponse::from_bytes(
            StatusCode::SERVICE_UNAVAILABLE,
            HeaderMap::new(),
            req.url,
            Bytes::from_static(b"busy"),
        ))
    });
    let ep_budget = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(mock_always_503);
    let err = native_request(
        &ep_budget,
        NativeRequest::new(Method::GET, "models").with_retry(NativeRetryPolicy::Safe(
            SafeRetryOptions::default()
                .with_max_retries(5)
                .with_initial_backoff(Duration::from_secs(5)),
        )),
        NativeContext::new().with_timeout(Duration::from_millis(30)),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, NativeError::Timeout(_)));

    // 6. Mid-body failure after head arrived never retries or stitches streams.
    let mid_calls = Arc::new(AtomicUsize::new(0));
    let mc = mid_calls.clone();
    let (mock_mid_io, _) = MockFetch::new(move |req| {
        let mc = mc.clone();
        async move {
            mc.fetch_add(1, Ordering::SeqCst);
            let stream: BoxStream<'static, Result<Bytes, FetchError>> =
                Box::pin(futures::stream::iter(vec![
                    Ok(Bytes::from_static(b"head-part")),
                    Err(FetchError::Io(SourceError::from(
                        "reset by peer".to_string(),
                    ))),
                ]));
            Ok(FetchResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                url: req.url,
                body: stream,
            })
        }
    });
    let ep_mid_io = NativeEndpoint::unauthenticated("https://api.example.com/v1")
        .unwrap()
        .with_fetch(mock_mid_io);
    let mut r_mid = native_request(
        &ep_mid_io,
        NativeRequest::new(Method::GET, "models").with_retry(NativeRetryPolicy::Safe(
            SafeRetryOptions::default()
                .with_max_retries(3)
                .with_initial_backoff(Duration::from_millis(1)),
        )),
        NativeContext::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        r_mid.body.next().await.unwrap().unwrap().as_ref(),
        b"head-part"
    );
    assert!(matches!(
        r_mid.body.next().await.unwrap().unwrap_err(),
        NativeError::Transport(FetchError::Io(_))
    ));
    assert!(r_mid.body.next().await.is_none());
    assert_eq!(mid_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sentinel_secrets_never_leak_across_debug_errors_or_observer() {
    const SENTINEL_RAW: &str = "sk-live-secret-9f8e7d/+=?";
    const SENTINEL_ENC: &str = "sk-live-secret-9f8e7d%2F%2B%3D%3F";

    let assert_no_leak = |text: &str, label: &str| {
        assert!(
            !text.contains(SENTINEL_RAW),
            "{label} leaked raw sentinel: {text}"
        );
        assert!(
            !text.contains(SENTINEL_ENC),
            "{label} leaked encoded sentinel: {text}"
        );
        assert!(
            !text.contains("basic-user:"),
            "{label} leaked userinfo: {text}"
        );
    };

    let (mock, _) = MockFetch::new(|req| async move {
        let mut h = HeaderMap::new();
        h.insert("x-custom-secret", HeaderValue::from_static(SENTINEL_RAW));
        h.insert("set-cookie", HeaderValue::from_static(SENTINEL_RAW));
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            h,
            req.url,
            Bytes::from_static(b"ok"),
        ))
    });

    let mut user_headers = BTreeMap::new();
    user_headers.insert("x-api-token".to_string(), Some(SENTINEL_RAW.to_string()));
    let provider_headers = ProviderHeaders::bearer(
        Credential::Explicit(Resolvable::Value(SENTINEL_RAW.to_string())),
        vec![("x-custom-fixed".to_string(), SENTINEL_RAW.to_string())],
        Some(user_headers.into_iter().collect()),
    );

    let endpoint = NativeEndpoint::with_provider_headers(
        &format!("https://basic-user:{SENTINEL_ENC}@api.example.com/v1"),
        provider_headers,
    )
    .unwrap()
    .try_with_protected_headers(["x-custom-fixed"])
    .unwrap()
    .with_fetch(mock);

    assert_no_leak(&format!("{endpoint:?}"), "endpoint Debug");

    let mut req_headers = HeaderMap::new();
    req_headers.insert("x-caller-secret", HeaderValue::from_static(SENTINEL_RAW));
    let request = NativeRequest::new(Method::POST, format!("responses?token={SENTINEL_ENC}&ok=1"))
        .with_headers(req_headers);

    assert_no_leak(&format!("{request:?}"), "request Debug");

    let obs = RecordingObserver::new();
    let response = native_request(
        &endpoint,
        request,
        NativeContext::new().with_observer(obs.clone()),
    )
    .await
    .unwrap();
    assert_no_leak(&format!("{response:?}"), "response Debug");
    let _ = drain_body(response.body).await.unwrap();

    let started = obs.started_events();
    assert_eq!(started.len(), 1);
    assert_no_leak(&format!("{:?}", started[0]), "observer started Debug");
    assert_no_leak(&started[0].sanitized_url, "observer sanitized_url");

    // Also test error Display/Debug never echoes header secrets or url secrets.
    let mut conflict = HeaderMap::new();
    conflict.insert("authorization", HeaderValue::from_static(SENTINEL_RAW));
    let err_hdr = native_request(
        &endpoint,
        NativeRequest::new(Method::GET, format!("models?key={SENTINEL_RAW}"))
            .with_headers(conflict),
        NativeContext::new(),
    )
    .await
    .unwrap_err();
    assert_no_leak(&format!("{err_hdr}"), "InvalidHeader Display");
    assert_no_leak(&format!("{err_hdr:?}"), "InvalidHeader Debug");

    let err_path = native_request(
        &endpoint,
        NativeRequest::new(Method::GET, format!("../evil?key={SENTINEL_RAW}")),
        NativeContext::new(),
    )
    .await
    .unwrap_err();
    assert_no_leak(&format!("{err_path}"), "InvalidPath Display");
    assert_no_leak(&format!("{err_path:?}"), "InvalidPath Debug");

    // Compile/smoke-check ReqwestFetch builder doesn't panic on unused proxy config.
    let _ = ReqwestFetch::default();
}
