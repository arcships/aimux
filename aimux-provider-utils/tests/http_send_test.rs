//! Integration tests for `http::send` / `http::send_stream` — verifies that
//! the HTTP layer's retry + status handling is correctly wired (RFC-0009).
//!
//! Uses wiremock to spin up a local mock server. Providers never touch reqwest
//! types — they hand `http::send` a pure-data `HttpRequest` and get back an
//! `HttpResponse` (or `HttpStreamResponse`).

use std::time::Duration;

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
    assert!(
        err.to_string().contains("aborted"),
        "got {err:?}"
    );
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
    signal.abort();

    let result = handle.await.unwrap();
    assert!(result.is_err(), "expected abort error, got {result:?}");
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
    // A 429 (retryable) + a slow success: the total deadline must bound the
    // whole retry sequence, not just one attempt.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after-ms", "200"))
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
    // Response starts streaming only after 500ms — the first-chunk deadline
    // (100ms) must fire while waiting for the first byte.
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
    assert!(err.to_string().contains("stream timeout"), "got {err:?}");
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
