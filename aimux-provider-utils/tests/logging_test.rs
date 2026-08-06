//! RFC-0014 logging integration tests: capture formatted output with a
//! `tracing` subscriber and assert the retry/failed chain, redaction, and
//! `AIMUX_LOG_BODY` body logging behave as designed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use aimux_provider_utils::http::{HttpBody, HttpMethod, HttpRequest, send, send_stream};
use aimux_provider_utils::logging::CaptureWriter;
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::retry::RetryConfig;
use futures::StreamExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn retry_cfg() -> RetryConfig {
    RetryConfig {
        max_retries: 1,
        initial_delay: Duration::from_millis(1),
        backoff_factor: 2,
    }
}

fn request(url: String) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url,
        headers: vec![(
            "Authorization".to_string(),
            "Bearer secret-token".to_string(),
        )],
        body: HttpBody::Json(serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "api_key": "sk-must-not-leak",
        })),
        abort_signal: None,
        call_id: None,
    }
}

/// 429 → retry (WARN) → 429 → failed (ERROR)：断言事件与字段、断言 header/body
/// 在默认级别下不泄露。
#[tokio::test]
async fn retry_chain_logged_with_redaction() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = fmt()
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_new("aimux_provider_utils=warn").unwrap())
        .with_writer(CaptureWriter(captured.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
        .expect(2)
        .mount(&server)
        .await;

    let res = send(
        request(format!("{}/v1/test", server.uri())),
        retry_cfg(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await;
    assert!(res.is_err());

    let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(output.contains("retry"), "missing retry event:\n{output}");
    assert!(output.contains("failed"), "missing failed event:\n{output}");
    assert!(
        output.contains("status=429"),
        "missing status field:\n{output}"
    );
    assert!(
        output.contains("reason=rate_limited"),
        "missing reason field:\n{output}"
    );
    assert!(
        !output.contains("Bearer secret-token"),
        "header value leaked:\n{output}"
    );
    assert!(
        !output.contains("sk-must-not-leak"),
        "request body leaked at warn level:\n{output}"
    );
}

/// debug 级别：request/response 摘要事件出现，URL 无 query、header 只记数量。
#[tokio::test]
async fn debug_level_shows_request_response_summary() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = fmt()
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_new("aimux_provider_utils=debug").unwrap())
        .with_writer(CaptureWriter(captured.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
        .mount(&server)
        .await;

    let mut url = format!("{}/v1/test", server.uri());
    url.push_str("?api_key=should-not-appear");
    let res = send(request(url), retry_cfg(), &DEFAULT_ERROR_STRUCTURE).await;
    assert!(res.is_ok());

    let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(
        output.contains("request"),
        "missing request event:\n{output}"
    );
    assert!(
        output.contains("response"),
        "missing response event:\n{output}"
    );
    assert!(
        output.contains("status=200"),
        "missing status=200:\n{output}"
    );
    assert!(
        !output.contains("api_key=should-not-appear"),
        "URL query leaked:\n{output}"
    );
    assert!(
        output.contains("header_count=1"),
        "header_count missing:\n{output}"
    );
}

/// trace + AIMUX_LOG_BODY=1：请求体出现但 api_key 已打码。
#[tokio::test]
async fn body_logging_redacts_secrets() {
    // SAFETY: single test touching this env var; serialized implicitly by
    // being the only test that reads it.
    unsafe { std::env::set_var("AIMUX_LOG_BODY", "1") };
    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = fmt()
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_new("aimux_provider_utils=trace").unwrap())
        .with_writer(CaptureWriter(captured.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
        .mount(&server)
        .await;

    let res = send(
        request(format!("{}/v1/test", server.uri())),
        retry_cfg(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await;
    assert!(res.is_ok());

    let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(
        output.contains("request_body"),
        "missing request_body trace:\n{output}"
    );
    assert!(
        output.contains("\"api_key\":\"***\""),
        "api_key not redacted:\n{output}"
    );
    assert!(
        !output.contains("sk-must-not-leak"),
        "secret leaked in body log:\n{output}"
    );
    unsafe { std::env::remove_var("AIMUX_LOG_BODY") };
}

/// 流式路径：stream_connected / stream_first_byte / stream_end 事件。
#[tokio::test]
async fn stream_events_are_emitted() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = fmt()
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_new("aimux_provider_utils=debug").unwrap())
        .with_writer(CaptureWriter(captured.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: hello\n\n"))
        .mount(&server)
        .await;

    let resp = send_stream(
        request(format!("{}/v1/stream", server.uri())),
        retry_cfg(),
        &DEFAULT_ERROR_STRUCTURE,
    )
    .await
    .expect("stream connect failed");
    // 消费完整字节流，触发 first_byte / end 事件。
    let mut body = resp.body;
    while let Some(chunk) = body.next().await {
        assert!(chunk.is_ok());
    }

    let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(
        output.contains("stream_connected"),
        "missing connect event:\n{output}"
    );
    assert!(
        output.contains("stream_first_byte"),
        "missing ttfb event:\n{output}"
    );
    assert!(
        output.contains("stream_end"),
        "missing end event:\n{output}"
    );
}
