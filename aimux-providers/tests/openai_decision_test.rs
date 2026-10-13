//! Official-contract tests against a local HTTP server; no live API access.
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use aimux_core::decision_model::*;
use aimux_core::recording::{self, RingRecorder};
use aimux_core::replay::{MockDecisionReplayModel, replay_decision_with_model};
use aimux_core::{AiMuxError, Provider};
use aimux_providers::{OpenAIProvider, OpenAIProviderSettings, PresetSettings, create_openai};
use indexmap::IndexMap;
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

mod common;

fn object(value: Value) -> serde_json::Map<String, Value> {
    value
        .as_object()
        .cloned()
        .expect("provider options must be an object")
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/openai_decisions.json")).unwrap()
}
fn options() -> DecideOptions {
    serde_json::from_value(fixture()["options"].clone()).unwrap()
}
fn provider(server: &MockServer, api_key: &str) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some(api_key.into()),
        base_url: Some(format!("{}/v1", server.uri())),
        organization: Some("org-test".into()),
        project: Some("proj-test".into()),
        headers: Some(HashMap::from([(
            "X-Client".into(),
            Some("decision-tests".into()),
        )])),
        ..Default::default()
    })
    .unwrap()
}
fn model(server: &MockServer) -> Arc<dyn DecisionModel> {
    provider(server, "decision-secret")
        .decision_model("gpt-6-luna")
        .unwrap()
        .unwrap()
}
/// The caller's registry at replay time: the same OpenAI settings at the
/// replay server, with another key.
fn rebuild(
    record: &aimux_core::recording::ProviderRecord,
    server: &MockServer,
) -> Arc<dyn DecisionModel> {
    let registry = aimux_core::create_provider_registry(
        std::collections::BTreeMap::from([(
            "openai".into(),
            Arc::new(provider(server, "replacement-key")) as Arc<dyn Provider>,
        )]),
        Default::default(),
    );
    aimux_providers::rebuild_decision_provider(record, &registry).unwrap()
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
async fn native_wire_maps_all_questions_to_v4_answers() {
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
    request.provider_options = Some(aimux_core::shared::SharedProviderOptions::from([
        (
            "openai".into(),
            object(json!({"safety_identifier":"opaque-user"})),
        ),
        ("other-provider".into(), object(json!({"ignored":true}))),
    ]));
    let result = decide(model(&server).as_ref(), request).await.unwrap();
    assert_eq!(
        result.answers.keys().collect::<Vec<_>>(),
        ["urgent", "team", "severity"]
    );
    assert_eq!(
        result.answers["urgent"],
        DecisionAnswer::Boolean { probability: 0.925 }
    );
    assert_eq!(
        result.answers["team"],
        DecisionAnswer::Choice {
            choice: "support".into(),
            probabilities: Some(IndexMap::from([
                ("billing".to_string(), 0.0625),
                ("support".to_string(), 0.9375),
            ])),
        }
    );
    assert_eq!(
        result.answers["severity"],
        DecisionAnswer::Score {
            score: 1.5,
            probabilities: Some(IndexMap::from([
                ("0".to_string(), 0.125),
                ("1".to_string(), 0.25),
                ("2".to_string(), 0.625),
            ])),
        }
    );
    assert_eq!(result.rounding, None);
    assert_eq!(
        Value::Object(result.provider_metadata.unwrap()["openai"].clone()),
        json!({
            "probability_source": "native",
            "answers": {"team": {"confidence": 0.9}, "severity": {"confidence": 0.6}}
        })
    );
    assert_eq!(
        result.response.body.as_ref().unwrap(),
        &fixture()["response"]
    );
    assert_eq!(result.response.model_id.as_deref(), Some("gpt-6-luna"));
    assert_eq!(
        result.usage,
        DecideUsage {
            input_tokens: Some(100),
            output_tokens: Some(0),
            total_tokens: Some(100),
        }
    );
}

/// A decision returns every answer or none (`ai@7.0.128
/// src/decide/validate-decision.ts:197-202`): a refused question fails the
/// call, with the exchange's HTTP context.
#[tokio::test]
#[serial]
async fn a_refused_question_fails_the_decision() {
    let server = MockServer::start().await;
    let mut response = fixture()["response"].clone();
    response["answers"][0] = json!({"name": "urgent", "type": "refusal"});
    mount(&server, response).await;
    let AiMuxError::ApiCall(error) = decide(model(&server).as_ref(), options())
        .await
        .unwrap_err()
    else {
        panic!("expected HTTP context");
    };
    assert_eq!(error.status_code, Some(200));
    assert!(!error.is_retryable);
    assert!(
        error
            .message
            .ends_with("OpenAI declined to answer question \"urgent\""),
        "{}",
        error.message
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
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
        assert_eq!(
            result.answers.keys().collect::<Vec<_>>(),
            ["urgent", "team", "severity"]
        );
        let count = |key: &str| usage.get(key).and_then(Value::as_u64).map(|n| n as u32);
        assert_eq!(result.usage.input_tokens, count("input_tokens"));
        assert_eq!(result.usage.output_tokens, count("output_tokens"));
    }
}

#[tokio::test]
#[serial]
async fn json_state_is_sent_as_text_evidence() {
    let server = MockServer::start().await;
    mount(&server, fixture()["response"].clone()).await;
    let mut request = options();
    request.state =
        serde_json::from_value(json!({"messages":[{"role":"user","content":"ordinary JSON"}]}))
            .unwrap();
    decide(model(&server).as_ref(), request.clone())
        .await
        .unwrap();
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(
        body["input"],
        serde_json::to_string(&request.state).unwrap()
    );
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
        ("/answers/2/name", json!("urgent")),
        ("/answers/2/type", json!("skipped")),
        ("/answers/2/type", json!("refusal")),
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
    guidance["questions"]["urgent"]["instructions"] = json!({"rule":"structured"});
    cases.push(guidance);
    let mut criteria = fixture()["options"].clone();
    criteria["questions"]["urgent"]["criteria"] = json!({"true":"yes"});
    cases.push(criteria);
    let mut levels = fixture()["options"].clone();
    levels["questions"]["severity"]["criteria"][0] = json!({"label":"low", "unsupported":true});
    cases.push(levels);
    let mut choice = fixture()["options"].clone();
    choice["questions"]["team"]["criteria"]
        .as_object_mut()
        .unwrap()
        .remove("support");
    cases.push(choice);
    let mut opts = fixture()["options"].clone();
    opts["provider_options"] = json!({"openai":{"temperature":1}});
    cases.push(opts);
    let mut values = fixture()["options"].clone();
    values["provider_options"] = json!({"openai":{"choice_values":{"urgent":{"yes":true}}}});
    cases.push(values);
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
    let model = model(&server);
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

/// The native model is the provider's decision model, which the registry
/// takes before the evaluation model (`ai@7.0.128
/// src/registry/provider-registry.ts:447-461`); a provider without one
/// offers none.
#[test]
fn factory_is_explicit_and_preserves_the_provider_boundary() {
    let provider = aimux_providers::create_provider(
        "openai",
        PresetSettings {
            api_key: Some("key".to_string().into()),
            ..Default::default()
        },
    )
    .unwrap();
    let model = provider
        .decision_model("deployment-alias")
        .unwrap()
        .unwrap();
    assert_eq!(model.model_id(), "deployment-alias");
    assert_eq!(model.provider(), "openai");
    assert_eq!(model.supported_question_types(), DecisionQuestionType::ALL);
    assert!(matches!(
        provider.decision_model(" "),
        Some(Err(AiMuxError::InvalidArgument(_)))
    ));
    let registry = aimux_core::create_provider_registry(
        std::collections::BTreeMap::from([("openai".to_string(), provider)]),
        Default::default(),
    );
    assert_eq!(
        registry
            .decision_model("openai:gpt-6-luna")
            .unwrap()
            .provider(),
        "openai"
    );
    let other = aimux_providers::create_provider(
        "deepseek",
        PresetSettings {
            api_key: Some("key".to_string().into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(other.decision_model("any").is_none());
}

/// `@ai-sdk/openai@4.0.83 src/openai-provider.ts:355-358`: the evaluation
/// model decides through the Responses model as `"{name}.evaluation"`.
#[tokio::test]
#[serial]
async fn evaluation_model_decides_through_the_responses_api() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 1_700_000_000,
            "status": "completed",
            "model": "gpt-6-luna",
            "output": [{
                "id": "msg_1",
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "{\"q0\":0.75,\"q1\":\"c1\",\"q2\":1.5}",
                    "annotations": []
                }]
            }],
            "usage": {"input_tokens": 30, "output_tokens": 5, "total_tokens": 35}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let model = provider(&server, "decision-secret").evaluation("gpt-6-luna");
    assert_eq!(model.provider(), "openai.evaluation");
    assert_eq!(model.model_id(), "gpt-6-luna");
    let result = decide(&model, options()).await.unwrap();
    assert_eq!(
        result.answers["urgent"],
        DecisionAnswer::Boolean { probability: 0.75 }
    );
    assert_eq!(
        result.answers["team"],
        DecisionAnswer::Choice {
            choice: "support".into(),
            probabilities: None,
        }
    );
    assert_eq!(result.usage.total_tokens, Some(35));
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["text"]["format"]["name"], "decision");
}

struct StopRecording;
impl Drop for StopRecording {
    fn drop(&mut self) {
        recording::init_recording(None);
    }
}

#[tokio::test]
#[serial]
async fn unified_recording_replays_and_rebuilds_from_the_registered_provider() {
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
    let rebuilt = rebuild(&records[0].provider, &replay_server);
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

fn openai_options(request: &mut DecideOptions) -> &mut serde_json::Map<String, Value> {
    request
        .provider_options
        .as_mut()
        .unwrap()
        .get_mut("openai")
        .unwrap()
}

#[tokio::test]
#[serial]
async fn images_typed_choices_and_score_descriptions_roundtrip_and_key_replay() {
    let contract: Value = serde_json::from_str(include_str!(
        "../../contract-tests/fixtures/decision-openai-full.json"
    ))
    .unwrap();
    let server = MockServer::start().await;
    Mock::given(path("/v1/decisions"))
        .and(body_json(contract["request"].clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(&contract["response"]))
        .mount(&server)
        .await;
    let request: DecideOptions = serde_json::from_value(contract["options"].clone()).unwrap();
    let recorder = Arc::new(RingRecorder::new());
    recording::init_recording(Some(recorder.clone()));
    let _stop = StopRecording;
    let result = decide(model(&server).as_ref(), request.clone())
        .await
        .unwrap();
    recording::init_recording(None);
    assert_eq!(
        result.answers["choice"],
        DecisionAnswer::Choice {
            choice: "boolean_true".into(),
            probabilities: Some(IndexMap::from([
                ("boolean_true".to_string(), 0.75),
                ("text_true".to_string(), 0.25),
            ])),
        }
    );
    assert_eq!(
        result.answers["score"],
        DecisionAnswer::Score {
            score: 0.75,
            probabilities: Some(IndexMap::from([
                ("0".to_string(), 0.25),
                ("1".to_string(), 0.75),
            ])),
        }
    );
    // The native value the server chose is provider metadata of the answer.
    assert_eq!(
        result.provider_metadata.as_ref().unwrap()["openai"]["answers"]["choice"],
        json!({"confidence": 0.7, "value": true})
    );
    let records = recorder.completed();
    let mock = MockDecisionReplayModel::new("openai", "gpt-6-luna", records.clone()).unwrap();
    decide(&mock, request.clone()).await.unwrap();
    let mut changed = request.clone();
    openai_options(&mut changed).remove("images");
    assert!(decide(&mock, changed).await.is_err());
    let mut changed = request.clone();
    openai_options(&mut changed)["images"][0]["detail"] = json!("high");
    assert!(decide(&mock, changed).await.is_err());
    let replay_server = MockServer::start().await;
    common::replay::mount_recording(&replay_server, &records[0]).await;
    let rebuilt = rebuild(&records[0].provider, &replay_server);
    let again = replay_decision_with_model(&records[0], rebuilt.as_ref())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(result.answers).unwrap(),
        serde_json::to_value(again.answers).unwrap()
    );
}

#[tokio::test]
#[serial]
async fn malformed_image_answers_bound_error_context_but_preserve_wire_and_response_evidence() {
    use base64::Engine;

    let bytes = vec![42; 8192];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let server = MockServer::start().await;
    let raw = json!({"model":"gpt-6-luna","answers":[],"metadata":{"api_key":"response-secret"}});
    mount(&server, raw.clone()).await;
    let mut request = options();
    request.provider_options = Some(HashMap::from([(
        "openai".into(),
        object(json!({"images": [{"data": encoded, "media_type": "image/png", "detail": "low"}]})),
    )]));
    let error = decide(model(&server).as_ref(), request).await.unwrap_err();
    let AiMuxError::ApiCall(error) = error else {
        panic!("expected contextual API error");
    };
    let data_url = format!("data:image/png;base64,{encoded}");
    assert_eq!(error.status_code, Some(200));
    assert!(!error.is_retryable);
    assert_eq!(
        error.request_body_values["input"][0]["content"][0]["image_url"],
        format!("[STRING {} bytes]", data_url.len())
    );
    assert_eq!(error.request_body_values["model"], "gpt-6-luna");
    assert_eq!(
        error.data.as_ref().unwrap()["metadata"]["api_key"],
        "[REDACTED]"
    );
    assert_eq!(
        serde_json::from_str::<Value>(error.response_body.as_ref().unwrap()).unwrap(),
        raw
    );
    let calls = server.received_requests().await.unwrap();
    assert_eq!(calls.len(), 1);
    let wire: Value = calls[0].body_json().unwrap();
    assert_eq!(wire["input"][0]["content"][0]["image_url"], data_url);
    assert_eq!(wire["input"][0]["content"][0]["detail"], "low");
}

#[tokio::test]
#[serial]
async fn bad_media_and_duplicate_native_values_are_rejected_before_http() {
    let contract: Value = serde_json::from_str(include_str!(
        "../../contract-tests/fixtures/decision-openai-full.json"
    ))
    .unwrap();
    let server = MockServer::start().await;
    for (pointer, value) in [
        ("/provider_options/openai/images/0/data", json!("bad!")),
        ("/provider_options/openai/images/0/detail", json!("unknown")),
        (
            "/provider_options/openai/images/0/media_type",
            json!("text/plain"),
        ),
        (
            "/provider_options/openai/choice_values/choice/text_true",
            json!(true),
        ),
        ("/questions/score/criteria/1/label", json!("low")),
    ] {
        let mut request = contract["options"].clone();
        *request.pointer_mut(pointer).unwrap() = value;
        assert!(
            decide(
                model(&server).as_ref(),
                serde_json::from_value(request).unwrap()
            )
            .await
            .is_err()
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
