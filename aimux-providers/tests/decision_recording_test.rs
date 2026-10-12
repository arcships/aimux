//! Unified recording/replay lifecycle for native decisions. All HTTP is local.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use aimux_core::Provider;
use aimux_core::decision_model::*;
use aimux_core::recording::{
    self, JsonlRecorder, OutcomeStatus, Recording, RecordingOperation, RingRecorder,
};
use aimux_core::replay::{MockDecisionReplayModel, replay_decision_with_model};
use aimux_providers::{JevConfig, JevProvider};
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

mod common;

struct StopRecording;
impl Drop for StopRecording {
    fn drop(&mut self) {
        recording::init_recording(None);
    }
}

fn example() -> Recording {
    serde_json::from_str(
        include_str!("fixtures/jev_systemone_live.jsonl")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap()
}
fn options() -> DecisionCallOptions {
    serde_json::from_value(example().input.options).unwrap()
}
fn response() -> Value {
    example().outcome.decision_result.unwrap()["response"]["body"].clone()
}
fn model(server: &MockServer) -> Box<dyn DecisionModel> {
    let config =
        JevConfig::new("live-key-secret").with_endpoint(format!("{}/v1/systemone", server.uri()));
    JevProvider::new(config)
        .decision_model("jev-latest")
        .unwrap()
}
async fn mount(server: &MockServer, response: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response)
                .insert_header("set-cookie", "session=response-secret"),
        )
        .mount(server)
        .await;
}

#[tokio::test]
#[serial]
async fn ring_records_one_complete_call_and_replays_exact_inputs_with_redaction() {
    let server = MockServer::start().await;
    mount(&server, response()).await;
    let recorder = Arc::new(RingRecorder::new());
    recording::init_recording(Some(recorder.clone()));
    let _stop = StopRecording;
    let mut request = options();
    request.headers = Some(std::collections::HashMap::from([(
        "X-Api-Key".into(),
        "input-secret".into(),
    )]));
    let result = decide(model(&server).as_ref(), request.clone())
        .await
        .unwrap();
    let records = recorder.completed();
    assert_eq!(records.len(), 1);
    assert!(records[0].complete && records[0].transport_closed);
    assert_eq!(records[0].input.operation, RecordingOperation::Decision);
    assert!(records[0].input.prompt.is_empty());
    assert!(records[0].input.decision_capabilities.is_some());
    assert_eq!(
        serde_json::to_value(&records[0].provider).unwrap(),
        json!({
            "providerId": "jev", "provider": "jev", "modelId": "jev-latest"
        })
    );
    assert_eq!(records[0].exchanges.len(), 1);
    assert_eq!(records[0].exchanges[0].attempt, 1);
    assert_eq!(records[0].exchanges[0].exchange_index, 1);
    let serialized = serde_json::to_string(&records).unwrap();
    for secret in ["live-key-secret", "input-secret", "response-secret"] {
        assert!(!serialized.contains(secret));
    }
    assert!(serialized.contains("[REDACTED]"));
    let mock = MockDecisionReplayModel::new("jev", "jev-latest", records.clone()).unwrap();
    recording::init_recording(None);
    let replayed = decide(&mock, request.clone()).await.unwrap();
    assert_eq!(
        serde_json::to_value(replayed.answers).unwrap(),
        serde_json::to_value(result.answers).unwrap()
    );
    let matching_request = request.clone();
    request.state = json!("a different input");
    assert!(decide(&mock, request).await.is_err());
    let mut incomplete = records;
    incomplete[0].complete = false;
    let mock = MockDecisionReplayModel::new("jev", "jev-latest", incomplete).unwrap();
    assert!(decide(&mock, matching_request).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
#[serial]
async fn jsonl_flush_has_no_duplicate_placeholders_and_supports_offline_replay() {
    let directory = std::env::temp_dir().join(format!(
        "aimux-decision-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let recorder = Arc::new(JsonlRecorder::new(&directory));
    recording::init_recording(Some(recorder.clone()));
    let _stop = StopRecording;
    let server = MockServer::start().await;
    mount(&server, response()).await;
    decide(model(&server).as_ref(), options()).await.unwrap();
    recording::init_recording(None);
    recorder.try_flush().unwrap();
    drop(recorder);
    let file = directory.join("recordings.jsonl");
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.lines().count(), 1);
    let rec: Recording = serde_json::from_str(text.trim()).unwrap();
    assert!(rec.complete);
    let mock = MockDecisionReplayModel::from_jsonl(&file).unwrap();
    replay_decision_with_model(&rec, &mock).await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[serial]
async fn retries_share_call_id_with_distinct_attempts_and_replay_wire_exchanges() {
    struct RetryResponse(AtomicUsize);
    impl Respond for RetryResponse {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(529).set_body_json(json!({"error":"busy"}))
            } else {
                ResponseTemplate::new(200).set_body_json(response())
            }
        }
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(RetryResponse(AtomicUsize::new(0)))
        .mount(&server)
        .await;
    let recorder = Arc::new(RingRecorder::new());
    recording::init_recording(Some(recorder.clone()));
    let _stop = StopRecording;
    let mut request = options();
    request.max_retries = Some(1);
    decide(model(&server).as_ref(), request.clone())
        .await
        .unwrap();
    let records = recorder.completed();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0]
            .exchanges
            .iter()
            .map(|e| e.attempt)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(records[0].outcome.status, OutcomeStatus::Success);
    recording::init_recording(None);
    let replay_server = MockServer::start().await;
    common::replay::mount_recording(&replay_server, &records[0]).await;
    decide(model(&replay_server).as_ref(), request)
        .await
        .unwrap();
    assert_eq!(replay_server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
#[serial]
async fn failed_response_and_timeout_have_complete_error_recordings() {
    let recorder = Arc::new(RingRecorder::new());
    recording::init_recording(Some(recorder.clone()));
    let _stop = StopRecording;
    let server = MockServer::start().await;
    mount(
        &server,
        json!({"model":"jev-1.13.0", "answers":{}, "usage":{"input_tokens":1,"output_tokens":1}}),
    )
    .await;
    assert!(decide(model(&server).as_ref(), options()).await.is_err());
    let delayed = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response())
                .set_delay(Duration::from_millis(200)),
        )
        .mount(&delayed)
        .await;
    let mut request = options();
    request.timeout = Some(aimux_core::options::TimeoutConfiguration {
        total_ms: Some(30),
        ..Default::default()
    });
    assert!(decide(model(&delayed).as_ref(), request).await.is_err());
    let records = recorder.completed();
    assert_eq!(records.len(), 2);
    for rec in records {
        assert!(rec.complete);
        assert_eq!(rec.outcome.status, OutcomeStatus::Error);
        assert!(rec.outcome.error_value.is_some());
        assert!(rec.outcome.decision_result.is_none());
        assert_eq!(rec.exchanges.len(), 1);
        let mock = MockDecisionReplayModel::new("jev", "jev-latest", vec![rec.clone()]).unwrap();
        let request = serde_json::from_value(rec.input.options.clone()).unwrap();
        let error = mock.do_decide(&request).await.unwrap_err();
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            rec.outcome.error_value.unwrap()
        );
    }
}

#[tokio::test]
#[serial]
async fn recorded_native_calls_replay_offline_and_rebuild_uses_registered_endpoint() {
    recording::init_recording(None);
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/jev_systemone_live.jsonl");
    let mock = MockDecisionReplayModel::from_jsonl(file).unwrap();
    for line in include_str!("fixtures/jev_systemone_live.jsonl").lines() {
        let rec: Recording = serde_json::from_str(line).unwrap();
        let result = replay_decision_with_model(&rec, &mock).await.unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            rec.outcome.decision_result.clone().unwrap()
        );
        let server = MockServer::start().await;
        common::replay::mount_recording(&server, &rec).await;
        let mut config =
            JevConfig::new("test-key").with_endpoint(format!("{}/v1/systemone", server.uri()));
        config.headers.insert("X-Routing".into(), "billing".into());
        let registry = aimux_core::create_provider_registry(
            std::collections::BTreeMap::from([(
                "jev".into(),
                Arc::new(JevProvider::new(config)) as Arc<dyn Provider>,
            )]),
            Default::default(),
        );
        let model = aimux_providers::rebuild_decision_provider(&rec.provider, &registry).unwrap();
        replay_decision_with_model(&rec, model.as_ref())
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].headers["x-routing"], "billing");
        assert_eq!(requests[0].headers["authorization"], "Bearer test-key");
    }
}

#[tokio::test]
#[serial]
async fn abort_keeps_the_original_recorder_snapshot_after_global_replacement() {
    struct AbortResponse {
        signal: aimux_core::AbortSignal,
        replacement: Arc<RingRecorder>,
    }
    impl Respond for AbortResponse {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            recording::init_recording(Some(self.replacement.clone()));
            self.signal.abort();
            ResponseTemplate::new(200)
                .set_body_json(response())
                .set_delay(Duration::from_millis(100))
        }
    }
    let original = Arc::new(RingRecorder::new());
    let replacement = Arc::new(RingRecorder::new());
    recording::init_recording(Some(original.clone()));
    let _stop = StopRecording;
    let signal = aimux_core::AbortSignal::new();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(AbortResponse {
            signal: signal.clone(),
            replacement: replacement.clone(),
        })
        .mount(&server)
        .await;
    let mut request = options();
    request.abort_signal = Some(signal);
    assert!(matches!(
        decide(model(&server).as_ref(), request).await,
        Err(aimux_core::AiMuxError::Aborted(_))
    ));
    let records = original.completed();
    assert_eq!(records.len(), 1);
    assert!(records[0].complete && records[0].exchanges[0].finalized);
    assert_eq!(records[0].outcome.status, OutcomeStatus::Error);
    assert!(replacement.completed().is_empty());
}
