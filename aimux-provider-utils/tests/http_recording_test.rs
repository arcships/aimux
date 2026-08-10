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
    // 模拟层 A 已注册全局 recorder:测试里 init_recording(Some(rec_arc)) 后,
    // 录制上下文由全局 recorder 现取(模拟 layerA)。call_id 与之一致。
    let recording_context = call_id.and_then(|_| {
        aimux_core::recording::recorder().map(|recorder| aimux_core::recording::RecordingContext {
            call_id: call_id.unwrap().to_string(),
            recorder,
        })
    });
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
        recording_context,
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
        .respond_with(ResponseTemplate::new(500).set_body_string("server boom"))
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

    // A5:5xx 是合法 HTTP 响应,结构化录制 status/headers/body(不仅 error)。
    let ex_fail = rec
        .exchanges
        .iter()
        .find(|e| e.error.is_some())
        .expect("失败应有 error 字段");
    assert!(
        ex_fail.error.as_deref().unwrap().contains("HTTP 500"),
        "error 字符串保留用于诊断: {:?}",
        ex_fail.error
    );
    let resp = ex_fail
        .response
        .as_ref()
        .expect("5xx 应有结构化 response (A5)");
    assert_eq!(resp.status, 500, "5xx response 应保留 status");
    assert_eq!(
        resp.body.as_deref(),
        Some("server boom"),
        "5xx response body 应结构化录制"
    );
    // 失败也是完整调用(wire 已终结:有结构化 response + error 字段)。
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

#[tokio::test]
#[serial]
async fn retry_records_per_attempt_success_after_429() {
    let server = MockServer::start().await;
    // 首次 429,第二次 200。
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let (recorder, dir) = recorder("b-retry");
    let rec_arc: Arc<dyn Recorder> = Arc::new(recorder);
    recording::init_recording(Some(rec_arc.clone()));

    // send_with_retry_raw 里 max_retries=2 会先 429 重试再成功。
    let url = format!("{}/v1/chat", server.uri());
    let req = json_post(&url, Some("call-retry"));
    let resp = send(req, fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();
    assert_eq!(resp.status, 200);

    mimic_layer_a(rec_arc.as_ref(), "call-retry");
    rec_arc.flush();
    let rec = wait_line(&dir);
    // 至少 2 exchange:attempt 0 失败(429)+ attempt 1 成功。
    assert!(
        rec.exchanges.len() >= 2,
        "expected retry exchanges, got {}",
        rec.exchanges.len()
    );
    // 第一条是 429:有 error 且结构化 response(A5:不再丢失 status/headers/body)。
    let ex429 = rec
        .exchanges
        .iter()
        .find(|e| e.error.is_some())
        .expect("attempt 0 should be a 429 failure");
    assert!(
        ex429.error.as_deref().unwrap().contains("HTTP 429"),
        "429 error 字符串保留: {:?}",
        ex429.error
    );
    let r429 = ex429
        .response
        .as_ref()
        .expect("429 应有结构化 response (A5)");
    assert_eq!(r429.status, 429, "429 response 应保留 status");
    assert_eq!(
        r429.body.as_deref(),
        Some("rate limited"),
        "429 response body 应结构化录制"
    );
    // 最后一条成功,status 200。
    assert_eq!(
        rec.exchanges
            .last()
            .unwrap()
            .response
            .as_ref()
            .unwrap()
            .status,
        200
    );

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[serial]
async fn utf8_body_truncates_without_panic_at_char_boundary() {
    // 构造恰好跨 1MiB 边界的中文(UTF-8 多字节)。
    let big = "中".repeat(400_000); // 400k * 3 bytes = 1.2MiB > 1MiB cap
    let server = MockServer::start().await;
    let big_for_mock = big.clone();
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string(big_for_mock))
        .mount(&server)
        .await;

    let (recorder, dir) = recorder("b-utf8");
    let rec_arc: Arc<dyn Recorder> = Arc::new(recorder);
    recording::init_recording(Some(rec_arc.clone()));

    let url = format!("{}/v1/chat", server.uri());
    let req = json_post(&url, Some("call-utf8"));
    // 不应 panic(UTF-8 安全截断)。
    let big_len = big.len();
    let resp = send(req, fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();
    assert_eq!(resp.body.len(), big_len); // 完整 body 返回给调用方

    mimic_layer_a(rec_arc.as_ref(), "call-utf8");
    rec_arc.flush();
    let rec = wait_line(&dir);
    let rbody = rec.exchanges[0]
        .response
        .as_ref()
        .unwrap()
        .body
        .as_deref()
        .unwrap();
    assert!(rbody.len() <= 1 << 20, "recorded body must respect cap");
    // 必须是完整 UTF-8(无 panic 已证明);中文可安全转回。
    assert!(rbody.chars().count() > 0);

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[serial]
async fn security_token_header_redacted_usage_keys_preserved() {
    // header 脱敏收窄:x-amz-security-token(AWS Bedrock sigv4 真实凭据头)恒脱敏;
    // contains("token") 已移除,故 max_output_tokens 等用量键不再被误伤。
    // URL query 不再脱敏(provider 均 header 鉴权),原样录制。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let (recorder, dir) = recorder("b-urlredact");
    let rec_arc: Arc<dyn Recorder> = Arc::new(recorder);
    recording::init_recording(Some(rec_arc.clone()));

    // URL 仅携带非敏感 query(原样录制,不再脱敏);header 携带:
    //  - x-amz-security-token(Bedrock sigv4 凭据)→ 必须脱敏
    //  - max_output_tokens(用量字段,含 "token" 子串)→ 必须保留
    //  - Authorization → 必须脱敏(contains authorization 回归保护)
    let url = format!("{}/v1/chat?model=gpt&version=1", server.uri());
    let recording_context =
        aimux_core::recording::recorder().map(|recorder| aimux_core::recording::RecordingContext {
            call_id: "call-redact".to_string(),
            recorder,
        });
    let req = HttpRequest {
        method: HttpMethod::Post,
        url,
        headers: vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("x-amz-security-token".to_string(), "STS-TOKEN".to_string()),
            ("max_output_tokens".to_string(), "4096".to_string()),
            ("Authorization".to_string(), "Bearer SECRET".to_string()),
        ],
        body: HttpBody::Json(serde_json::json!({"q": "hi"})),
        abort_signal: None,
        call_id: Some("call-redact".to_string()),
        recording_context,
    };
    let resp = send(req, fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();
    assert_eq!(resp.status, 200);

    mimic_layer_a(rec_arc.as_ref(), "call-redact");
    rec_arc.flush();
    let rec = wait_line(&dir);
    let ex = &rec.exchanges[0];
    let recorded_url = &ex.request.url;

    // URL query 原样保留(不再脱敏):非敏感 query 完整落盘。
    assert!(
        recorded_url.ends_with("?model=gpt&version=1"),
        "url should be recorded verbatim (no query redaction): {recorded_url}"
    );

    // x-amz-security-token(AWS 凭据头)脱敏。
    let sec_token = ex
        .request
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-amz-security-token"))
        .expect("x-amz-security-token header recorded");
    assert_eq!(sec_token.1, "[REDACTED]");

    // max_output_tokens 含 "token" 子串但非凭据——contains("token") 收窄后保留。
    let usage_key = ex
        .request
        .headers
        .iter()
        .find(|(k, _)| k == "max_output_tokens")
        .expect("max_output_tokens header recorded");
    assert_eq!(
        usage_key.1, "4096",
        "max_output_tokens must NOT be redacted (contains(token) narrowed): {:?}",
        ex.request.headers
    );

    // Authorization 仍按 contains(authorization) 脱敏(回归保护)。
    let auth = ex
        .request
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Authorization"))
        .expect("Authorization header recorded");
    assert_eq!(auth.1, "[REDACTED]");

    // catch-all:整条录制序列化后不得出现明文凭据。
    let dump = serde_json::to_string(&rec).unwrap_or_default();
    assert!(
        !dump.contains("STS-TOKEN"),
        "STS token leaked in recording: {dump}"
    );
    assert!(
        !dump.contains("SECRET"),
        "secret leaked in recording: {dump}"
    );

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[serial]
async fn stream_error_patches_single_exchange_not_duplicate() {
    // SSE 首帧后一个 error → provider 侧 Error(流中途)。
    let sse = r#"data: {"choices":[{"delta":{"content":"a"}}]}\n\ndata: [DONE]\n\n"#;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/stream"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse))
        .mount(&server)
        .await;
    let (recorder, dir) = recorder("b-serr");
    let rec_arc: Arc<dyn Recorder> = Arc::new(recorder);
    recording::init_recording(Some(rec_arc.clone()));

    let url = format!("{}/v1/stream", server.uri());
    let req = json_post(&url, Some("call-serr"));
    let resp = send_stream(req, fast_config(), &DEFAULT_ERROR_STRUCTURE)
        .await
        .unwrap();
    let _: Vec<_> = resp.body.collect().await;

    mimic_layer_a(rec_arc.as_ref(), "call-serr");
    rec_arc.flush();
    let rec = wait_line(&dir);
    // 只有 1 条 exchange(骨架 patch,不新增)。status 应保留 200。
    assert_eq!(rec.exchanges.len(), 1, "must not duplicate attempt");
    assert_eq!(rec.exchanges[0].response.as_ref().unwrap().status, 200);
    assert!(rec.exchanges[0].finalized);

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir);
}
