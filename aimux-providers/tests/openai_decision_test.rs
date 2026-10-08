//! Official-contract tests against a local HTTP server; no live API access.
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use aimux_core::decision_model::*;
use aimux_core::recording::{self, RingRecorder};
use aimux_core::replay::{MockDecisionReplayModel, replay_decision_with_model};
use aimux_core::{AiMuxError, Provider};
use aimux_providers::{OpenAIConfig, OpenAIProvider};
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

mod common;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/openai_decisions.json")).unwrap()
}
fn options() -> DecisionCallOptions {
    serde_json::from_value(fixture()["options"].clone()).unwrap()
}
fn config(server: &MockServer) -> OpenAIConfig {
    OpenAIConfig::new("decision-secret")
        .with_base_url(format!("{}/v1", server.uri()))
        .with_org_id("org-test")
        .with_project("proj-test")
        .with_headers(HashMap::from([(
            "X-Client".into(),
            "decision-tests".into(),
        )]))
}
fn model(server: &MockServer) -> Box<dyn DecisionModel> {
    OpenAIProvider::new(config(server))
        .decision_model("gpt-6-luna")
        .unwrap()
}
async fn mount(server: &MockServer, response: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(server)
        .await;
}

#[tokio::test]
#[serial]
async fn native_wire_maps_all_questions_and_preserves_partial_refusal() {
    let server = MockServer::start().await;
    let mut expected = fixture()["request"].clone();
    expected["safety_identifier"] = json!("opaque-user");
    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .and(header("authorization", "Bearer decision-secret"))
        .and(header("openai-organization", "org-test"))
        .and(header("openai-project", "proj-test"))
        .and(header("x-client", "decision-tests"))
        .and(header("x-per-call", "yes"))
        .and(body_json(expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(&fixture()["response"]))
        .expect(1)
        .mount(&server)
        .await;
    let mut request = options();
    request.headers = Some(HashMap::from([("X-Per-Call".into(), "yes".into())]));
    request.provider_options = Some(HashMap::from([
        ("openai".into(), json!({"safety_identifier":"opaque-user"})),
        ("other-provider".into(), json!({"ignored":true})),
    ]));
    let result = decide(model(&server).as_ref(), request).await.unwrap();
    assert!(matches!(
        result.answers["restricted"],
        DecisionAnswer::Refusal
    ));
    assert!(matches!(
        result.answers["urgent"],
        DecisionAnswer::Boolean {
            probability_true: 0.925
        }
    ));
    assert_eq!(result.rounding, DecisionRounding::default());
    assert_eq!(result.probability_source, DecisionProbabilitySource::Native);
    assert_eq!(
        result.response.as_ref().unwrap().body.as_ref().unwrap(),
        &fixture()["response"]
    );
    assert_eq!(
        result.usage.as_ref().unwrap().input_tokens.no_cache,
        Some(70)
    );
    assert_eq!(
        result.provider_metadata.unwrap()["openai"]["future_metadata"]["preserved"],
        true
    );
}

#[tokio::test]
#[serial]
async fn response_order_and_usage_metadata_do_not_discard_answers() {
    for usage in [
        Value::Null,
        json!({}),
        json!({"input_tokens":12}),
        json!({"input_tokens":1,"output_tokens":0,"total_tokens":9,"input_tokens_details":{"cached_tokens":2,"cache_write_tokens":0}}),
    ] {
        let server = MockServer::start().await;
        let mut response = fixture()["response"].clone();
        response["usage"] = usage.clone();
        response["answers"].as_array_mut().unwrap().reverse();
        response["answers"][1]["probabilities"]
            .as_array_mut()
            .unwrap()
            .reverse();
        mount(&server, response).await;
        let result = decide(model(&server).as_ref(), options()).await.unwrap();
        assert_eq!(result.answers.len(), 4);
        assert_eq!(
            result.usage.and_then(|usage| usage.raw),
            if usage.is_null() { None } else { Some(usage) }
        );
    }
}

#[tokio::test]
#[serial]
async fn json_state_is_text_evidence_and_all_question_types_can_refuse() {
    let server = MockServer::start().await;
    let mut response = fixture()["response"].clone();
    for answer in response["answers"].as_array_mut().unwrap() {
        *answer = json!({"name":answer["name"], "type":"refusal"});
    }
    mount(&server, response).await;
    let mut request = options();
    request.state = json!({"messages":[{"role":"user","content":"ordinary JSON"}]});
    let result = decide(model(&server).as_ref(), request.clone())
        .await
        .unwrap();
    assert!(
        result
            .answers
            .values()
            .all(|a| matches!(a, DecisionAnswer::Refusal))
    );
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["input"], request.state.to_string());
}

#[tokio::test]
#[serial]
async fn malformed_answers_fail_with_http_context() {
    let mut cases = Vec::new();
    for (pointer, value) in [
        ("/answers/0/name", json!("unknown")),
        ("/answers/0/name", Value::Null),
        ("/answers/0/probability", json!(1.1)),
        ("/answers/1/choice", json!(true)),
        ("/answers/1/choice", json!("unknown")),
        ("/answers/1/probabilities/1/value", json!("billing")),
        ("/answers/1/probabilities/1/probability", json!(0.5)),
        ("/answers/2/probabilities/1/value", json!(0)),
        ("/answers/2/probabilities/1/value", json!(9)),
        ("/answers/2/probabilities/1/label", json!("wrong")),
        ("/answers/2/score", json!(0.1)),
        ("/answers/3/name", json!("urgent")),
        ("/answers/3/type", json!("skipped")),
    ] {
        let mut response = fixture()["response"].clone();
        *response.pointer_mut(pointer).unwrap() = value;
        cases.push(response);
    }
    let mut missing = fixture()["response"].clone();
    missing["answers"].as_array_mut().unwrap().pop();
    cases.push(missing);
    let mut extra = fixture()["response"].clone();
    extra["answers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"extra","type":"refusal"}));
    cases.push(extra);
    for response in cases {
        let server = MockServer::start().await;
        mount(&server, response).await;
        let error = decide(model(&server).as_ref(), options())
            .await
            .unwrap_err();
        let AiMuxError::ApiCall(error) = error else {
            panic!("expected HTTP context: {error}");
        };
        assert_eq!(error.status_code, Some(200));
        assert!(!error.is_retryable);
        assert!(error.response_body.is_some());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
#[serial]
async fn unsupported_native_fields_and_limits_fail_before_http() {
    let server = MockServer::start().await;
    let mut cases = Vec::new();
    let mut guidance = fixture()["options"].clone();
    guidance["questions"][0]["instructions"] = json!({"rule":"structured"});
    cases.push(guidance);
    let mut criteria = fixture()["options"].clone();
    criteria["questions"][0]["criteria"] = json!({"true":"yes"});
    cases.push(criteria);
    let mut levels = fixture()["options"].clone();
    levels["questions"][2]["levels"][0] = json!({"label":"low"});
    cases.push(levels);
    let mut choice = fixture()["options"].clone();
    choice["questions"][1]["options"]
        .as_array_mut()
        .unwrap()
        .pop();
    cases.push(choice);
    let mut opts = fixture()["options"].clone();
    opts["provider_options"] = json!({"openai":{"temperature":1}});
    cases.push(opts);
    for request in cases {
        let request = serde_json::from_value(request).unwrap();
        assert!(decide(model(&server).as_ref(), request).await.is_err());
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
#[serial]
async fn retry_and_standard_provider_errors_use_the_shared_transport() {
    let server = MockServer::start().await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    Mock::given(path("/v1/decisions"))
        .respond_with(move |_: &Request| {
            if counter.fetch_add(1, Ordering::Relaxed) == 0 {
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "0")
                    .set_body_json(json!({"error":{"message":"busy","code":"rate_limit_exceeded"}}))
            } else {
                ResponseTemplate::new(200).set_body_json(&fixture()["response"])
            }
        })
        .mount(&server)
        .await;
    let mut config = config(&server);
    config.retry_config.initial_delay = Duration::ZERO;
    let model = OpenAIProvider::new(config)
        .decision_model("gpt-6-luna")
        .unwrap();
    let mut request = options();
    request.max_retries = Some(1);
    decide(model.as_ref(), request).await.unwrap();
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
    server.reset().await;
    Mock::given(path("/v1/decisions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error":{"message":"bad key","code":"invalid_api_key"}})),
        )
        .mount(&server)
        .await;
    let AiMuxError::ApiCall(error) = decide(model.as_ref(), options()).await.unwrap_err() else {
        panic!("API error");
    };
    assert_eq!(error.provider_code.as_deref(), Some("invalid_api_key"));
    assert!(!error.is_retryable);
}

#[test]
fn factory_is_explicit_and_preserves_the_provider_boundary() {
    let provider = aimux_providers::provider_handle("openai", Some("key".into()), None).unwrap();
    let model = provider.decision_model("deployment-alias").unwrap();
    assert_eq!(model.model_id(), "deployment-alias");
    assert!(provider.decision_model(" ").is_err());
    let other = aimux_providers::provider_handle("deepseek", Some("key".into()), None).unwrap();
    assert!(matches!(
        other.decision_model("any"),
        Err(AiMuxError::UnsupportedFunctionality(_))
    ));
    assert_eq!(model.capabilities().min_choices, Some(2));
}

struct StopRecording;
impl Drop for StopRecording {
    fn drop(&mut self) {
        recording::init_recording(None);
    }
}

#[tokio::test]
#[serial]
async fn unified_recording_replays_refusals_and_rebuilds_the_same_provider_config() {
    let server = MockServer::start().await;
    mount(&server, fixture()["response"].clone()).await;
    let recorder = Arc::new(RingRecorder::new());
    recording::init_recording(Some(recorder.clone()));
    let _stop = StopRecording;
    let result = decide(model(&server).as_ref(), options()).await.unwrap();
    recording::init_recording(None);
    let records = recorder.completed();
    assert_eq!(records.len(), 1);
    assert!(records[0].complete);
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("decision-secret")
    );
    let mock = MockDecisionReplayModel::new("openai", "gpt-6-luna", records.clone()).unwrap();
    let replay = decide(&mock, options()).await.unwrap();
    assert_eq!(
        serde_json::to_value(replay.answers).unwrap(),
        serde_json::to_value(&result.answers).unwrap()
    );
    let replay_server = MockServer::start().await;
    common::replay::mount_recording(&replay_server, &records[0]).await;
    let mut snapshot = records[0].provider.clone();
    snapshot.base_url = Some(format!("{}/v1", replay_server.uri()));
    let rebuilt =
        aimux_providers::rebuild_decision_provider(&snapshot, Some("replacement-key")).unwrap();
    let again = replay_decision_with_model(&records[0], rebuilt.as_ref())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(again.answers).unwrap(),
        serde_json::to_value(result.answers).unwrap()
    );
    let request = &replay_server.received_requests().await.unwrap()[0];
    assert_eq!(request.headers["authorization"], "Bearer replacement-key");
    assert_eq!(request.headers["openai-project"], "proj-test");
    assert_eq!(request.headers["x-client"], "decision-tests");
}
