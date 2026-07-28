//! End-to-end tests for the cassette replay infrastructure in
//! [`common::replay`].
//!
//! These tests mount the hand-written fixtures under `tests/replay_fixtures/`
//! onto a real `wiremock` `MockServer` and drive it over HTTP with `reqwest`,
//! verifying the matching rules from RFC 0003:
//!
//! - routing by method + path,
//! - feature-based selection (model, stream) within a shared path,
//! - fallback to the first cassette when no feature matches,
//! - verbatim return of recorded headers and body bytes (incl. SSE),
//! - 404 for paths with no cassette.
//!
//! The fixtures are prefixed `01_`..`04_` so filename sort order — and thus the
//! fallback target — is deterministic.

mod common;

use common::replay;
use serde_json::{Value, json};
use wiremock::MockServer;

/// Directory of hand-written cassettes used by these tests. Relative to the
/// crate root, which is the working directory for integration tests.
const FIXTURES: &str = "tests/replay_fixtures";

/// POST a JSON body to `uri` and return the response.
async fn post_json(uri: &str, body: &Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(uri)
        .json(body)
        .send()
        .await
        .expect("request should succeed")
}

/// `mount_cassettes` returns the number of cassettes loaded and mounted.
#[tokio::test]
async fn mounts_all_fixtures_and_returns_count() {
    let server = MockServer::start().await;
    let n = replay::mount_cassettes(&server, FIXTURES).await;
    // 01_text, 02_stream, 03_opus share /v1/messages; 04_models is /v1/models.
    assert_eq!(n, 4);
}

/// One `Mock` is registered per `(method, path)` group; GET /v1/models hits the
/// `04_models` cassette.
#[tokio::test]
async fn routes_by_method_and_path() {
    let (server, _) = replay::start(FIXTURES).await;

    let resp = reqwest::get(format!("{}/v1/models", server.uri()))
        .await
        .expect("GET should succeed");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("body is JSON");
    assert_eq!(body["object"], "list");
    assert_eq!(body["data"][0]["id"], "gpt-4o");
}

/// Within /v1/messages, a request whose `model` matches the `03_opus` cassette
/// selects that cassette even though `01_text` is first in file order.
#[tokio::test]
async fn selects_cassette_by_model() {
    let (server, _) = replay::start(FIXTURES).await;

    let resp = post_json(
        &format!("{}/v1/messages", server.uri()),
        &json!({
            "model": "claude-3-opus-20240229",
            "messages": [{ "role": "user", "content": "Hi" }]
        }),
    )
    .await;

    let body: Value = resp.json().await.expect("body is JSON");
    assert_eq!(body["id"], "msg_opus");
    assert_eq!(body["model"], "claude-3-opus-20240229");
}

/// `stream: true` breaks the tie between `01_text` and `02_stream` (same model)
/// in favour of the streaming cassette, and the SSE body comes back verbatim.
#[tokio::test]
async fn selects_cassette_by_stream_flag() {
    let (server, _) = replay::start(FIXTURES).await;

    let resp = post_json(
        &format!("{}/v1/messages", server.uri()),
        &json!({
            "model": "claude-3-haiku-20240307",
            "stream": true,
            "messages": [{ "role": "user", "content": "Hi" }]
        }),
    )
    .await;

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let body = resp.text().await.expect("body is text");
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: message_stop"));
    assert!(body.contains("\"text\":\"Hi\""));
}

/// Without `stream`, the `01_text` cassette wins the tie (same model, no stream
/// field in `01_text` so `02_stream`'s `stream: true` does not match).
#[tokio::test]
async fn non_stream_request_returns_text_cassette() {
    let (server, _) = replay::start(FIXTURES).await;

    let resp = post_json(
        &format!("{}/v1/messages", server.uri()),
        &json!({
            "model": "claude-3-haiku-20240307",
            "messages": [{ "role": "user", "content": "Hi" }]
        }),
    )
    .await;

    let body: Value = resp.json().await.expect("body is JSON");
    assert_eq!(body["id"], "msg_text");
    assert_eq!(body["content"][0]["text"], "Hello, World!");
}

/// A request whose `model` matches no cassette falls back to the first
/// cassette on that path (`01_text`).
#[tokio::test]
async fn falls_back_to_first_when_no_feature_matches() {
    let (server, _) = replay::start(FIXTURES).await;

    let resp = post_json(
        &format!("{}/v1/messages", server.uri()),
        &json!({
            "model": "claude-3-sonnet-20240229",
            "messages": [{ "role": "user", "content": "Hi" }]
        }),
    )
    .await;

    let body: Value = resp.json().await.expect("body is JSON");
    assert_eq!(body["id"], "msg_text");
}

/// Recorded headers and body bytes are returned untouched — the SSE body ends
/// with the exact trailing bytes from the cassette.
#[tokio::test]
async fn returns_recorded_sse_bytes_verbatim() {
    let (server, _) = replay::start(FIXTURES).await;

    let resp = post_json(
        &format!("{}/v1/messages", server.uri()),
        &json!({
            "model": "claude-3-haiku-20240307",
            "stream": true,
            "messages": []
        }),
    )
    .await;

    let body = resp.text().await.expect("body is text");
    assert_eq!(
        body,
        "event: message_start\n\
         data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"model\":\"claude-3-haiku-20240307\"}}\n\n\
         event: content_block_delta\n\
         data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
         event: message_delta\n\
         data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n\
         event: message_stop\n\
         data: {\"type\":\"message_stop\"}\n\n"
    );
}

/// A non-JSON request body (e.g. plain text) still resolves to a cassette via
/// the fallback rule rather than 5xx-ing.
#[tokio::test]
async fn non_json_request_body_falls_back_to_first() {
    let (server, _) = replay::start(FIXTURES).await;

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/messages", server.uri()))
        .header("content-type", "text/plain")
        .body("not json at all")
        .send()
        .await
        .expect("POST should succeed");

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("body is JSON");
    assert_eq!(body["id"], "msg_text");
}

/// A request to a path with no cassette gets wiremock's default 404.
#[tokio::test]
async fn unmatched_path_returns_404() {
    let (server, _) = replay::start(FIXTURES).await;

    let resp = reqwest::get(format!("{}/no/such/path", server.uri()))
        .await
        .expect("GET should succeed");
    assert_eq!(resp.status(), 404);
}

/// A cassette can declare a non-200 status and it is returned faithfully.
#[tokio::test]
async fn returns_recorded_error_status() {
    use std::fs;

    // Build a throwaway cassette directory with a single 429 cassette so this
    // test is self-contained and doesn't depend on the shared fixtures.
    let dir = std::env::temp_dir().join("aimux_replay_test_429");
    fs::create_dir_all(&dir).expect("create temp dir");
    let cas = serde_json::json!({
        "source": "fixture",
        "provider": "anthropic",
        "scenario": "rate_limited",
        "request": {
            "path": "/v1/messages",
            "method": "POST",
            "headers": { "content-type": "application/json" },
            "body": { "model": "claude-3-haiku-20240307" }
        },
        "response": {
            "status": 429,
            "headers": { "content-type": "application/json", "retry-after": "5" },
            "body": "{\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"Too many requests\"}}"
        }
    });
    fs::write(
        dir.join("rate_limited.json"),
        serde_json::to_string_pretty(&cas).unwrap(),
    )
    .expect("write cassette");

    let (server, n) = replay::start(&dir).await;
    assert_eq!(n, 1);

    let resp = post_json(
        &format!("{}/v1/messages", server.uri()),
        &json!({ "model": "claude-3-haiku-20240307" }),
    )
    .await;
    assert_eq!(resp.status(), 429);
    assert_eq!(resp.headers().get("retry-after").unwrap(), "5");
    let body: Value = resp.json().await.expect("body is JSON");
    assert_eq!(body["error"]["type"], "rate_limit_error");

    let _ = fs::remove_dir_all(&dir);
}
