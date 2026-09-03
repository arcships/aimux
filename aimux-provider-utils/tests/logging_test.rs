//! Logging tests for the single-exchange HTTP throat.

use std::sync::{Arc, Mutex};

use aimux_provider_utils::logging::CaptureWriter;
use aimux_provider_utils::{
    HttpRequest, ProviderErrorParts, create_json_error_response_handler,
    create_json_response_handler, post_json_to_api,
};
use serde::Deserialize;
use serde_json::json;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Deserialize)]
struct Reply {
    #[allow(dead_code)]
    ok: bool,
}

fn request(url: String) -> HttpRequest {
    HttpRequest {
        url,
        headers: vec![("authorization".into(), "Bearer secret-token".into())],
        abort_signal: None,
        call_id: None,
        recording_context: None,

        ..Default::default()
    }
}

fn failed() -> aimux_provider_utils::ResponseHandler<aimux_core::AiMuxError> {
    create_json_error_response_handler(|_| ProviderErrorParts {
        message: "request failed".into(),
        provider_code: None,
    })
}

fn capture(level: &str) -> (Arc<Mutex<Vec<u8>>>, tracing::dispatcher::DefaultGuard) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = fmt()
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_new(level).unwrap())
        .with_writer(CaptureWriter(captured.clone()))
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (captured, guard)
}

#[tokio::test]
async fn debug_summary_sanitizes_url_and_never_logs_header_values() {
    let (captured, _guard) = capture("aimux_provider_utils=debug");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;

    post_json_to_api(
        request(format!("{}?api_key=must-not-appear", server.uri())),
        json!({"api_key": "must-not-appear"}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();

    let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(output.contains("request"));
    assert!(output.contains("response"));
    assert!(output.contains("status=200"));
    assert!(output.contains("header_count=1"));
    assert!(!output.contains("must-not-appear"));
    assert!(!output.contains("Bearer secret-token"));
}

#[tokio::test]
async fn trace_body_uses_shared_sensitive_key_policy() {
    // SAFETY: this integration-test process owns this environment variable.
    unsafe { std::env::set_var("AIMUX_LOG_BODY", "1") };
    let (captured, _guard) = capture("aimux_provider_utils=trace");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;

    post_json_to_api(
        request(server.uri()),
        json!({"api_key": "must-not-appear", "prompt": "hello"}),
        create_json_response_handler::<Reply>(),
        failed(),
    )
    .await
    .unwrap();

    let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(output.contains("request_body"));
    assert!(output.contains("[REDACTED]"));
    assert!(!output.contains("must-not-appear"));
    // SAFETY: paired with the set above.
    unsafe { std::env::remove_var("AIMUX_LOG_BODY") };
}
