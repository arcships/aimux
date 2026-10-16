//! A failed decision, end to end: the model is resolved from its registry
//! string (the path a binding or the CLI takes), the failure is recorded as
//! one completed recording with outcome status Error and no decision result,
//! and that recording replays offline as the same failure, never as an
//! answer. The per-provider tests (`openai_decision_test.rs`,
//! `runtime_decision_test.rs`) already call `decide` against a local server.

use std::collections::BTreeMap;
use std::sync::Arc;

use aimux_core::decision_model::{DecideOptions, decide};
use aimux_core::recording::{self, OutcomeStatus, Recording, RingRecorder};
use aimux_core::replay::MockDecisionReplayModel;
use aimux_core::{AiMuxError, Provider};
use aimux_providers::{OpenAIProviderSettings, PresetSettings, create_openai, create_provider};
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Stop;
impl Drop for Stop {
    fn drop(&mut self) {
        recording::init_recording(None);
    }
}

fn openai_fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/openai_decisions.json")).unwrap()
}

fn runtime_fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/runtime_decisions.json")).unwrap()
}

/// Resolve `id` through a registry holding `provider` under `name`, decide
/// with a ring recorder installed, and return the error with the one
/// recording.
async fn decide_recorded(
    name: &str,
    provider: Arc<dyn Provider>,
    id: &str,
    options: DecideOptions,
) -> (AiMuxError, Recording) {
    let registry = aimux_core::create_provider_registry(
        BTreeMap::from([(name.to_string(), provider)]),
        Default::default(),
    );
    let model = registry.decision_model(id).unwrap();
    let recorder = Arc::new(RingRecorder::new());
    recording::init_recording(Some(recorder.clone()));
    let _stop = Stop;
    let error = match decide(model.as_ref(), options).await {
        Ok(_) => panic!("{id}: the decision must fail"),
        Err(error) => error,
    };
    recording::init_recording(None);
    let mut recordings = recorder.completed();
    assert_eq!(recordings.len(), 1, "{id}: one recording");
    (error, recordings.remove(0))
}

/// The recording of a failed decision and its offline replay.
async fn assert_recorded_failure(
    recording: Recording,
    secret: &str,
    options: DecideOptions,
    expected: &str,
) {
    assert_eq!(recording.outcome.status, OutcomeStatus::Error);
    assert!(recording.outcome.decision_result.is_none());
    assert_eq!(recording.exchanges.len(), 1);
    assert!(!serde_json::to_string(&recording).unwrap().contains(secret));

    let offline = MockDecisionReplayModel::new(
        recording.provider.provider.clone(),
        recording.provider.model_id.clone(),
        vec![recording],
    )
    .unwrap();
    let replayed = match decide(&offline, options).await {
        Ok(_) => panic!("a failed recording must not replay as an answer"),
        Err(error) => error,
    };
    assert!(replayed.to_string().contains(expected), "{replayed}");
}

/// A refused question fails `decide("openai:<id>")`, the native Decisions API
/// (`ai@7.0.128 src/decide/validate-decision.ts:197-202`: every answer or
/// none; aimux openai/decision.rs:325-331).
#[tokio::test]
#[serial]
async fn a_refusal_fails_the_registry_resolved_openai_decision() {
    let server = MockServer::start().await;
    let fixture = openai_fixture();
    let mut response = fixture["response"].clone();
    response["answers"][0] = json!({"name": "urgent", "type": "refusal"});
    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;
    let openai = create_openai(OpenAIProviderSettings {
        api_key: Some("decision-secret".into()),
        base_url: Some(format!("{}/v1", server.uri())),
        ..Default::default()
    })
    .unwrap();
    let options: DecideOptions = serde_json::from_value(fixture["options"].clone()).unwrap();

    let (error, recording) = decide_recorded(
        "openai",
        Arc::new(openai),
        "openai:gpt-6-luna",
        options.clone(),
    )
    .await;

    let AiMuxError::ApiCall(api) = &error else {
        panic!("expected HTTP context, got {error}");
    };
    assert!(
        api.message
            .ends_with("OpenAI declined to answer question \"urgent\""),
        "{}",
        api.message
    );
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "not retried"
    );
    assert_recorded_failure(recording, "decision-secret", options, "declined to answer").await;
}

/// A question vLLM skipped under `ask_if` fails `decide("vllm:<id>")`
/// (aimux runtime_decision.rs:369-383).
#[tokio::test]
#[serial]
async fn a_skipped_question_fails_the_registry_resolved_vllm_decision() {
    let server = MockServer::start().await;
    let fixture = runtime_fixture();
    let mut response = fixture["response"].clone();
    response["answers"]["severity"] = Value::Null;
    response["diagnostics"] = json!({"skipped": {"severity": {"because": "team", "was": "support", "wanted": ["billing"]}}});
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;
    let vllm = create_provider(
        "vllm",
        PresetSettings {
            api_key: Some("runtime-secret".to_string().into()),
            base_url: Some(format!("{}/v1", server.uri())),
            ..Default::default()
        },
    )
    .unwrap();
    let options: DecideOptions = serde_json::from_value(fixture["options"].clone()).unwrap();

    let (error, recording) =
        decide_recorded("vllm", vllm, "vllm:served-decision", options.clone()).await;

    let AiMuxError::ApiCall(api) = &error else {
        panic!("expected HTTP context, got {error}");
    };
    assert!(
        api.message.contains("vllm skipped question \"severity\""),
        "{}",
        api.message
    );
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "not retried"
    );
    assert_recorded_failure(recording, "runtime-secret", options, "skipped question").await;
}
