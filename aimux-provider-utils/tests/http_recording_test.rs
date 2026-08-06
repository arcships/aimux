//! RFC-0023 层 B(http 咽喉点)录制集成测试。
//!
//! 覆盖:send 成功/失败 exchange、send_stream 骨架 + 终结补全、脱敏、
//! transport_closed → barrier 写出。全局录制器单例,用例 `#[serial]` 串行。

use futures::StreamExt;
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::recording::{
    self, JsonlRecorder, OutcomeRecord, OutcomeStatus, Recorder, Recording,
};
use aimux_provider_utils::{
    DEFAULT_ERROR_STRUCTURE, HttpBody, HttpMethod, HttpRequest, RetryConfig, send, send_stream,
};

fn fast_config() -> RetryConfig {
    RetryConfig {
        max_retries: 1,
        initial_delay: Duration::from_millis(1),
        backoff_factor: 2,
    }
}

fn json_post(url: &str, call_id: Option<&str>) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url: url.to_string(),
        headers: vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), "Bearer secret".to_string()),
            ("X-Custom".to_string(), "value".to_string()),
        ],
        body: HttpBody::Json(serde_json::json!({"q": "hi"})),
        abort_signal: None,
        call_id: call_id.map(|s| s.to_string()),
    }
}

fn recorder(tag: &str) -> (JsonlRecorder, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("aimux-brec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rec = JsonlRecorder::new(&dir);
    (rec, dir)
}

fn wait_line(dir: &std::path::Path) -> Recording {
    let path = dir.join("recordings.jsonl");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(s) = std::fs::read_to_string(&path)
            && let Some(l) = s.lines().next()
            && !l.trim().is_empty()
            && let Ok(r) = serde_json::from_str::<Recording>(l.trim())
        {
            return r;
        }
        if std::time::Instant::now() > deadline {
            panic!("no recording line at {}", path.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 模拟层 A 的 outcome + transport closed(barrier 所需)。
fn mimic_layer_a(rec: &dyn Recorder, call_id: &str) {
    rec.record_outcome(
        call_id,
        &OutcomeRecord {
            status: OutcomeStatus::Success,
            finish_reason: Some("stop".into()),
            error: None,
            usage: None,
        },
    );
    rec.record_transport_closed(call_id);
}

#[tokio::test]
#[serial]
async fn send_records_success_exchange_and_closing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let (recorder, dir) = recorder("send-ok");
    let rec_arc: Arc<dyn Recorder> = Arc::new(recorder);
    recording::init_recording(Some(rec_arc.clone()));

    let url = format!("{}/v1/chat", server.uri());
    let req = json_post(&url, Some("call-b1"));
    let resp = send(req, fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();
    assert_eq!(resp.status, 200);

    mimic_layer_a(rec_arc.as_ref(), "call-b1");
    rec_arc.flush();
    let rec = wait_line(&dir);

    assert_eq!(rec.call_id, "call-b1");
    assert_eq!(rec.exchanges.len(), 1);
    let ex = &rec.exchanges[0];
    assert!(ex.finalized, "non-stream exchange final");
    // Authorization 脱敏;自定义头保留。
    let auth = ex
        .request
        .headers
        .iter()
        .find(|(k, _)| k == "Authorization")
        .unwrap();
    assert_eq!(auth.1, "[REDACTED]");
    assert!(
        ex.request
            .headers
            .iter()
            .any(|(k, v)| k == "X-Custom" && v == "value")
    );
    // 响应体。
    let resp_rec = ex.response.as_ref().unwrap();
    assert!(resp_rec.body.as_deref().unwrap().contains("ok"));
    assert!(rec.complete);

    aimux_core::recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[serial]
async fn send_records_failure_without_closing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let (recorder, dir) = recorder("b-fail");
    let rec_arc: Arc<dyn Recorder> = Arc::new(recorder);
    recording::init_recording(Some(rec_arc.clone()));

    let url = format!("{}/v1/chat", server.uri());
    let req = json_post(&url, Some("call-b2"));
    let err = send(req, fast_config(), &DEFAULT_ERROR_STRUCTURE).await;
    assert!(err.is_err());

    // 失败无 closed(等下骨架等待),层 A 的 outcome 若有则写出 ▼
    mimic_layer_a(rec_arc.as_ref(), "call-b2");
    rec_arc.flush();
    let rec = wait_line(&dir);

    assert!(
        rec.exchanges.iter().any(|e| e.error.is_some()),
        "失败应有 error 字段"
    );
    // 失败也是完整调用(wire 已终结:无响应但有 error 字段)。
    assert!(rec.complete, "失败调用也应标记 complete");
    aimux_core::recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[serial]
async fn stream_finalizes_skeleton_with_accumulated_body() {
    let sse = "data: {\"delta\":\"hel\"}\n\n\
               data: {\"delta\":\"lo\"}\n\n\
               data: [DONE]\n\n";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;
    let (recorder, dir) = recorder("b-stream");
    let rec_arc: Arc<dyn Recorder> = Arc::new(recorder);
    recording::init_recording(Some(rec_arc.clone()));

    let url = format!("{}/v1/stream", server.uri());
    let req = json_post(&url, Some("call-b3"));
    let resp = send_stream(req, fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();
    // 消费完整流。
    let _: Vec<_> = resp.body.collect().await;

    mimic_layer_a(rec_arc.as_ref(), "call-b3"); // 层 A outcome+closed
    rec_arc.flush();
    let rec = wait_line(&dir);

    assert_eq!(rec.exchanges.len(), 1);
    let ex = &rec.exchanges[0];
    assert!(ex.finalized);
    let r = ex.response.as_ref().unwrap();
    let stream_text = r.body.as_deref().unwrap_or("");
    assert!(
        stream_text.contains(r#"data: {"delta":"hel"}"#),
        "原始 SSE 应被累积: {r:?}"
    );
    assert!(r.stream_chunks.unwrap_or(0) >= 1, "至少一个 chunk 计数");
    assert!(r.stream_chunks.unwrap_or(0) >= 1);
    assert!(rec.complete);
    aimux_core::recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[serial]
async fn abandoned_stream_still_finalizes() {
    let sse = "data: {\"delta\":\"x\"}\n\n";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/stream"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse))
        .mount(&server)
        .await;
    let (recorder, dir) = recorder("b-abandon");
    let rec_arc: Arc<dyn Recorder> = Arc::new(recorder);
    recording::init_recording(Some(rec_arc.clone()));

    let url = format!("{}/v1/stream", server.uri());
    let req = json_post(&url, Some("call-b4"));
    let resp = send_stream(req, fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();
    drop(resp.body); // 提前放弃 → ObservedByteStream Drop 兜底标记 "abandoned"

    mimic_layer_a(rec_arc.as_ref(), "call-b4");
    rec_arc.flush();
    let rec = wait_line(&dir);
    let ex = &rec.exchanges[0];
    assert!(ex.finalized, "Drop 应兜底 finalized(即使无完整 body)");
    aimux_core::recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir);
}
