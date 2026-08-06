//! Integration tests for `http::send` / `http::send_stream` — verifies that
//! the HTTP layer's retry + status handling is correctly wired (RFC-0009).
//!
//! Uses wiremock to spin up a local mock server. Providers never touch reqwest
//! types — they hand `http::send` a pure-data `HttpRequest` and get back an
//! `HttpResponse` (or `HttpStreamResponse`).

use std::time::{Duration, Instant};

use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::AiMuxError;
use aimux_provider_utils::{
    DEFAULT_ERROR_STRUCTURE, HttpBody, HttpMethod, HttpRequest, RetryConfig, send, send_stream,
};

/// Short config for tests: 1ms initial delay (jitter → 0ms effective),
/// up to 2 retries.
fn fast_config() -> RetryConfig {
    RetryConfig {
        max_retries: 2,
        initial_delay: Duration::from_millis(1),
        backoff_factor: 2,
    }
}

/// Build a JSON POST request to `url`.
fn json_post(url: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url: url.to_string(),
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: HttpBody::Json(serde_json::json!({"q": "hi"})),
        abort_signal: None,
        call_id: None,
        recording_context: None,
    }
}

#[tokio::test]
async fn returns_response_on_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let resp = send(json_post(&url), fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();

    assert_eq!(resp.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn retries_on_429_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after-ms", "1"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let resp = send(json_post(&url), fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();

    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn retries_on_500_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let resp = send(json_post(&url), fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();

    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn does_not_retry_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "message": "Invalid API key", "type": "invalid_request_error" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let err = send(json_post(&url), fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap_err();

    assert!(matches!(err, AiMuxError::Auth(_)), "got {err:?}");
}

#[tokio::test]
async fn does_not_retry_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": { "message": "model not found", "type": "not_found" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let err = send(json_post(&url), fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap_err();

    assert!(matches!(err, AiMuxError::ModelNotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn exhausts_retries_on_persistent_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after-ms", "1"))
        .expect(3)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let err = send(json_post(&url), fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap_err();

    assert!(matches!(err, AiMuxError::RateLimited { .. }), "got {err:?}");
}

#[tokio::test]
async fn exhausts_retries_on_persistent_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(503).set_body_string("Service Unavailable"))
        .expect(3)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let err = send(json_post(&url), fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap_err();

    assert!(matches!(err, AiMuxError::ApiCall(_)), "got {err:?}");
}

#[tokio::test]
async fn send_stream_returns_byte_stream() {
    // send_stream should return a byte stream that the caller can consume
    // independently of reqwest types.
    let sse_body = "data: {\"choices\":[]}\n\n";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let resp = send_stream(json_post(&url), fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();

    assert_eq!(resp.status, 200);

    // Consume the byte stream — it yields Bytes, not reqwest types.
    let mut collected = Vec::new();
    let mut stream = resp.body;
    while let Some(chunk) = stream.next().await {
        collected.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(collected, sse_body.as_bytes());
}

// ─────────────────────────────────────────────────────────────────────────────
// RFC-0016 H1/H3: abort + per-call timeout
// ─────────────────────────────────────────────────────────────────────────────

use aimux_core::shared::AbortSignal;
use aimux_provider_utils::{RequestTimeout, send_stream_timed, send_timed};

#[tokio::test]
async fn abort_before_send_fails_immediately() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(0) // must never be reached
        .mount(&server)
        .await;

    let signal = AbortSignal::new();
    signal.abort();

    let mut req = json_post(&format!("{}/v1/chat", server.uri()));
    req.abort_signal = Some(signal);

    let err = send(req, fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap_err();
    assert!(matches!(err, AiMuxError::Aborted), "got {err:?}");
}

#[tokio::test]
async fn abort_mid_request_cancels() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": true}))
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;

    let signal = AbortSignal::new();
    let signal_clone = signal.clone();

    let url = format!("{}/v1/chat", server.uri());
    let request = json_post(&url);
    let handle = tokio::spawn(async move {
        let mut req = request;
        req.abort_signal = Some(signal_clone);
        send(req, fast_config(), &DEFAULT_ERROR_STRUCTURE).await
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = Instant::now();
    signal.abort();

    // Bounded join: if cancellation regresses to hanging, the outer timeout
    // fails the test instead of hanging the suite.
    let result = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("cancelled call must finish within 2s")
        .expect("task must not panic");
    assert!(matches!(result, Err(AiMuxError::Aborted)), "got {result:?}");
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "abort should cancel well before the 300ms server delay"
    );
}

#[tokio::test]
async fn send_timed_total_timeout_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": true}))
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let err = send_timed(
        json_post(&url),
        fast_config(),
        &DEFAULT_ERROR_STRUCTURE,
        Some(RequestTimeout {
            total_ms: Some(100),
            ..Default::default()
        }),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, AiMuxError::Timeout(_)), "got {err:?}");
    assert!(err.to_string().contains("total timeout"), "got {err:?}");
}

#[tokio::test]
async fn send_timed_total_timeout_covers_retries() {
    // A retryable 429 whose response takes longer than the total budget:
    // the total deadline must bound the whole call (connect + response +
    // retry backoff), not just one attempt.
    //
    // The response is delayed past the budget on purpose: a fast 429 would
    // race with the jittered retry backoff (Full Jitter ∈ [0, retry-after)),
    // making this test flaky depending on the random delay drawn.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after-ms", "200")
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let err = send_timed(
        json_post(&url),
        fast_config(),
        &DEFAULT_ERROR_STRUCTURE,
        Some(RequestTimeout {
            total_ms: Some(100),
            ..Default::default()
        }),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, AiMuxError::Timeout(_)), "got {err:?}");
}

#[tokio::test]
async fn send_timed_within_budget_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": true}))
                .set_delay(Duration::from_millis(50)),
        )
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let resp = send_timed(
        json_post(&url),
        fast_config(),
        &DEFAULT_ERROR_STRUCTURE,
        Some(RequestTimeout {
            total_ms: Some(5000),
            ..Default::default()
        }),
    )
    .await
    .unwrap();

    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn send_stream_timed_first_chunk_timeout() {
    // wiremock's `set_delay` delays the whole response (headers included), so
    // this test covers "the first-byte budget includes response-header
    // latency" — the first-chunk timer counts from request start. The
    // body-pending wakeup path itself is covered by the unit tests
    // (`timeout_stream_yields_after_first_chunk_deadline` and
    // `timeout_stream_enforces_chunk_idle_deadline`), which use a pending
    // inner stream.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string("data: {\"choices\":[]}\n\n")
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let resp = match send_stream_timed(
        json_post(&url),
        fast_config(),
        &DEFAULT_ERROR_STRUCTURE,
        Some(RequestTimeout {
            first_chunk_ms: Some(100),
            ..Default::default()
        }),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => panic!("unexpected error: {e:?}"),
    };

    let mut stream = resp.body;
    let first = stream.next().await.expect("stream must yield an error");
    let err = first.unwrap_err();
    assert!(matches!(err, AiMuxError::Timeout(_)), "got {err:?}");
    assert!(
        err.to_string().contains("first chunk timeout"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn send_stream_timed_total_timeout_on_connect() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("data: x\n\n")
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let err = match send_stream_timed(
        json_post(&url),
        fast_config(),
        &DEFAULT_ERROR_STRUCTURE,
        Some(RequestTimeout {
            total_ms: Some(100),
            ..Default::default()
        }),
    )
    .await
    {
        Ok(_) => panic!("expected timeout, got success"),
        Err(e) => e,
    };

    assert!(matches!(err, AiMuxError::Timeout(_)), "got {err:?}");
}

#[tokio::test]
async fn send_timed_zero_total_fails_immediately() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(0) // must never be reached
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let err = send_timed(
        json_post(&url),
        fast_config(),
        &DEFAULT_ERROR_STRUCTURE,
        Some(RequestTimeout {
            total_ms: Some(0),
            ..Default::default()
        }),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AiMuxError::Timeout(_)), "got {err:?}");
}

#[tokio::test]
async fn abort_wins_over_total_timeout() {
    // Abort fires strictly before the 100ms total deadline, so the call must
    // report Aborted — verifying the documented "abort wins" semantics for
    // the connect phase (biased select inside send_request). A true
    // same-instant tie is not exercised here (that depends on tokio's
    // internal poll order); see RFC-0016 §7.6 S5.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": true}))
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    let signal = AbortSignal::new();
    let signal_clone = signal.clone();
    let url = format!("{}/v1/chat", server.uri());
    let request = json_post(&url);
    let handle = tokio::spawn(async move {
        let mut req = request;
        req.abort_signal = Some(signal_clone);
        send_timed(
            req,
            fast_config(),
            &DEFAULT_ERROR_STRUCTURE,
            Some(RequestTimeout {
                total_ms: Some(100),
                ..Default::default()
            }),
        )
        .await
    });

    // Abort just before the 100ms deadline so both fire around the same time.
    tokio::time::sleep(Duration::from_millis(90)).await;
    signal.abort();

    let result = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("must finish")
        .expect("task must not panic");
    assert!(matches!(result, Err(AiMuxError::Aborted)), "got {result:?}");
}

/// Regression (audit round 3, B1): a large *valid-UTF-8* error body must be
/// truncated with a marker — a silent truncation would make callers believe
/// the response body was complete.
#[tokio::test]
async fn truncates_large_error_body_with_marker() {
    let server = MockServer::start().await;
    let big_body = "x".repeat(100 * 1024);
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(429).set_body_string(big_body))
        .up_to_n_times(3)
        .mount(&server)
        .await;

    let url = format!("{}/v1/chat", server.uri());
    let no_retry = RetryConfig {
        max_retries: 0,
        ..RetryConfig::default()
    };
    let err = send(json_post(&url), no_retry, &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap_err();

    match err {
        AiMuxError::RateLimited { message, .. } => {
            assert!(
                message.contains("(truncated"),
                "large error body must be marked as truncated, got len={}",
                message.len()
            );
            assert!(
                message.len() < 70 * 1024,
                "truncated message must stay bounded, got {} bytes",
                message.len()
            );
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}
