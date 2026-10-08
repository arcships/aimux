//! Offline contract tests using official examples and recorded live responses.
//! No live credentials or external requests are needed to replay them.
use std::time::Duration;

use aimux_core::decision_model::*;
use aimux_core::{AiMuxError, Provider};
use aimux_providers::{JevConfig, JevProvider};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

fn fixture() -> Value {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/jev_systemone.json")).unwrap();
    fixture["response"].clone()
}

#[tokio::test]
async fn optional_usage_counts_do_not_discard_valid_answers() {
    for usage in [
        json!({}),
        json!({"input_tokens":null,"output_tokens":null}),
        json!({"input_tokens":12}),
        json!({"output_tokens":7}),
        json!({"input_tokens":null,"output_tokens":7}),
    ] {
        let server = MockServer::start().await;
        let mut response = fixture();
        response["usage"] = usage.clone();
        mount(&server, 200, response).await;
        let result = decide(model(&server).as_ref(), options()).await.unwrap();
        assert_eq!(result.answers.len(), 3);
        let actual = result.usage.unwrap();
        assert_eq!(
            actual.input_tokens.total,
            usage["input_tokens"].as_u64().map(|n| n as u32)
        );
        assert_eq!(
            actual.output_tokens.total,
            usage["output_tokens"].as_u64().map(|n| n as u32)
        );
        assert_eq!(actual.raw, Some(usage));
    }
    let server = MockServer::start().await;
    let mut response = fixture();
    response.as_object_mut().unwrap().remove("usage");
    mount(&server, 200, response).await;
    assert!(decide(model(&server).as_ref(), options()).await.is_err());
}

#[tokio::test]
async fn official_choice_requires_a_maximum_probability_and_accepts_ties() {
    for (probabilities, selected, valid) in [
        (
            json!({"billing":1.0,"technical":0.0,"sales":0.0}),
            "technical",
            false,
        ),
        (
            json!({"billing":0.34,"technical":0.33,"sales":0.33}),
            "technical",
            false,
        ),
        (
            json!({"billing":0.5,"technical":0.5,"sales":0.0}),
            "technical",
            true,
        ),
        (
            json!({"billing":0.5,"technical":0.5,"sales":0.0}),
            "billing",
            true,
        ),
        (
            json!({"billing":1.0,"technical":0.0,"sales":0.0}),
            "billing",
            true,
        ),
    ] {
        let server = MockServer::start().await;
        let mut response = fixture();
        response["answers"]["department"]["probabilities"] = probabilities;
        response["answers"]["department"]["choice"] = json!(selected);
        mount(&server, 200, response).await;
        let result = decide(model(&server).as_ref(), options()).await;
        assert_eq!(result.is_ok(), valid);
        if !valid {
            let AiMuxError::ApiCall(error) = result.unwrap_err() else {
                panic!("expected HTTP context");
            };
            assert_eq!(error.status_code, Some(200));
            assert!(!error.is_retryable);
            assert!(error.message.contains("highest-probability"));
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn recorded_official_live_responses_replay_through_core_validation() {
    for line in include_str!("fixtures/jev_systemone_live.jsonl").lines() {
        let recording: aimux_core::recording::Recording = serde_json::from_str(line).unwrap();
        let server = MockServer::start().await;
        common::replay::mount_recording(&server, &recording).await;
        let request: DecisionCallOptions =
            serde_json::from_value(recording.input.options.clone()).unwrap();
        let result = decide(model(&server).as_ref(), request).await.unwrap();
        let result = serde_json::to_value(result).unwrap();
        let expected = recording.outcome.decision_result.unwrap();
        for field in ["answers", "model", "rounding", "usage"] {
            assert_eq!(result[field], expected[field]);
        }
        assert_eq!(result["response"]["body"], expected["response"]["body"]);
    }
}

fn options() -> DecisionCallOptions {
    DecisionCallOptions::new(
        json!({"ticket": "Billed twice"}),
        vec![
            DecisionQuestion::Boolean {
                criteria: None,
                id: "is_urgent".into(),
                instructions: "The message conveys urgency or time-sensitivity".into(),
            },
            DecisionQuestion::Choice {
                id: "department".into(),
                instructions: "Which team should handle this".into(),
                options: ["billing", "technical", "sales"]
                    .into_iter()
                    .map(|label| DecisionOption {
                        value: None,
                        label: label.into(),
                        description: None,
                    })
                    .collect(),
            },
            DecisionQuestion::Score {
                id: "frustration".into(),
                instructions: "How frustrated the customer appears".into(),
                levels: [
                    "Calm, just stating facts",
                    "Frustrated but civil",
                    "Very angry, strong language",
                ]
                .into_iter()
                .map(DecisionDescription::from)
                .collect(),
            },
        ],
    )
}

fn model(server: &MockServer) -> Box<dyn DecisionModel> {
    let mut config =
        JevConfig::new("test-key").with_endpoint(format!("{}/v1/systemone", server.uri()));
    config.retry_config.initial_delay = Duration::ZERO;
    JevProvider::new(config)
        .decision_model("jev-latest")
        .unwrap()
}

async fn mount(server: &MockServer, status: u16, response: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("Authorization", "Bearer test-key"))
        .respond_with(
            ResponseTemplate::new(status)
                .set_body_json(response)
                .insert_header("X-Request-ID", "test-request"),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn official_contract_round_trip_preserves_types_and_raw_metadata() {
    let server = MockServer::start().await;
    mount(&server, 200, fixture()).await;
    let model = model(&server);
    let result = decide(model.as_ref(), options()).await.unwrap();
    assert!(
        matches!(result.answers["is_urgent"], DecisionAnswer::Boolean { probability_true } if probability_true == 1.0)
    );
    assert!(
        matches!(&result.answers["department"], DecisionAnswer::Choice { selected, probabilities: Some(p), .. } if selected == "technical" && p.len() == 3)
    );
    assert!(
        matches!(&result.answers["frustration"], DecisionAnswer::Score { expected_value, probabilities: Some(p), .. } if *expected_value == 1.0 && p == &[0.0, 1.0, 0.0])
    );
    assert_eq!(result.model, "jev-1.13.0");
    assert_eq!(result.model_version, None);
    assert_eq!(result.usage.as_ref().unwrap().input_tokens.total, Some(392));
    assert_eq!(result.usage.as_ref().unwrap().output_tokens.total, Some(65));
    assert_eq!(
        result.usage.as_ref().unwrap().raw,
        Some(fixture()["usage"].clone())
    );
    assert_eq!(
        result.response.as_ref().unwrap().body.as_ref().unwrap(),
        &fixture()
    );
    assert_eq!(result.provider_metadata.as_ref().unwrap()["jev"], fixture());
    assert_eq!(
        result.response.as_ref().unwrap().headers.as_ref().unwrap()["x-request-id"],
        "test-request"
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["state"], options().state);
    assert_eq!(
        body["questions"]["is_urgent"],
        json!({"type":"noul", "instructions":"The message conveys urgency or time-sensitivity"})
    );
    assert_eq!(
        body["questions"]["department"]["criteria"],
        json!({"billing":null, "technical":null, "sales":null})
    );
    assert_eq!(
        body["questions"]["frustration"]["criteria"],
        json!([
            "Calm, just stating facts",
            "Frustrated but civil",
            "Very angry, strong language"
        ])
    );
}

#[tokio::test]
async fn malformed_answers_fail_with_http_context_without_retry() {
    let mut responses = Vec::new();
    let mut missing = fixture();
    missing["answers"]
        .as_object_mut()
        .unwrap()
        .remove("department");
    responses.push(missing);
    let mut unknown = fixture();
    unknown["answers"]["department"]["choice"] = json!("unknown");
    responses.push(unknown);
    let mut range = fixture();
    range["answers"]["is_urgent"]["noul"] = json!(1.2);
    responses.push(range);
    let mut keys = fixture();
    keys["answers"]["frustration"]["probabilities"] = json!({"0":0,"1":1.0,"4":0});
    responses.push(keys);
    let mut wrong_type = fixture();
    wrong_type["answers"]["department"] = json!({"type":"noul", "noul":0.8});
    responses.push(wrong_type);
    let mut distribution = fixture();
    distribution["answers"]["department"]["probabilities"]["technical"] = json!(0.4);
    responses.push(distribution);
    let mut missing_distribution = fixture();
    missing_distribution["answers"]["department"]
        .as_object_mut()
        .unwrap()
        .remove("probabilities");
    responses.push(missing_distribution);
    let mut legend = fixture();
    legend["answers"]["frustration"]["legend"]["0"] = json!("Other");
    responses.push(legend);
    let mut score = fixture();
    score["answers"]["frustration"]["score"] = json!(4);
    responses.push(score);
    let mut contradictory_score = fixture();
    contradictory_score["answers"]["frustration"]["score"] = json!(0);
    responses.push(contradictory_score);
    let mut missing_confidence = fixture();
    missing_confidence["answers"]["department"]
        .as_object_mut()
        .unwrap()
        .remove("confidence");
    responses.push(missing_confidence);
    let mut missing_usage = fixture();
    missing_usage.as_object_mut().unwrap().remove("usage");
    responses.push(missing_usage);
    for response in responses {
        let server = MockServer::start().await;
        mount(&server, 200, response).await;
        let error = decide(model(&server).as_ref(), options())
            .await
            .unwrap_err();
        let AiMuxError::ApiCall(error) = error else {
            panic!("expected contextual API error")
        };
        assert_eq!(error.status_code, Some(200));
        assert!(error.url.ends_with("/v1/systemone"));
        assert_eq!(error.request_body_values["model"], "jev-latest");
        assert!(error.response_body.is_some());
        assert!(!error.is_retryable);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn invalid_requests_never_reach_http() {
    let server = MockServer::start().await;
    let mut duplicate = options();
    duplicate.questions.push(duplicate.questions[0].clone());
    let mut empty_choice = options();
    if let DecisionQuestion::Choice { options, .. } = &mut empty_choice.questions[1] {
        options.clear();
    }
    let mut duplicate_label = options();
    if let DecisionQuestion::Choice { options, .. } = &mut duplicate_label.questions[1] {
        options.push(options[0].clone());
    }
    let mut too_many_choices = options();
    if let DecisionQuestion::Choice { options, .. } = &mut too_many_choices.questions[1] {
        *options = (0..256)
            .map(|index| DecisionOption {
                value: None,
                label: format!("option{index}"),
                description: None,
            })
            .collect();
    }
    let mut too_many_levels = options();
    if let DecisionQuestion::Score { levels, .. } = &mut too_many_levels.questions[2] {
        *levels = (0..11)
            .map(|index| format!("level{index}").into())
            .collect();
    }
    for state in [Value::Null, json!(true), json!(42)] {
        let mut invalid_state = options();
        invalid_state.state = state;
        assert!(matches!(
            decide(model(&server).as_ref(), invalid_state).await,
            Err(AiMuxError::InvalidArgument(_))
        ));
    }
    for options in [
        duplicate,
        empty_choice,
        duplicate_label,
        too_many_choices,
        too_many_levels,
    ] {
        assert!(matches!(
            decide(model(&server).as_ref(), options).await,
            Err(AiMuxError::InvalidArgument(_))
        ));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn official_provider_failures_use_standard_retry_rules_and_preserve_codes() {
    for (status, code, attempts) in [
        (401, "authentication_error", 1),
        (422, "validation_error", 1),
        (429, "rate_limit", 3),
        (529, "overloaded", 3),
        (503, "unavailable", 3),
        (499, "cancelled", 1),
    ] {
        let server = MockServer::start().await;
        mount(
            &server,
            status,
            json!({"error":{"message":"test failure", "code":code}}),
        )
        .await;
        let error = decide(model(&server).as_ref(), options())
            .await
            .unwrap_err();
        let error = match error {
            AiMuxError::Retry(error) => error.last_error().clone(),
            error => error,
        };
        let AiMuxError::ApiCall(error) = error else {
            panic!("expected API error")
        };
        assert_eq!(error.status_code, Some(status));
        assert_eq!(error.provider_code.as_deref(), Some(code));
        assert_eq!(server.received_requests().await.unwrap().len(), attempts);
    }
}

#[test]
fn non_decision_provider_returns_unsupported() {
    let provider = aimux_providers::provider_handle("deepseek", Some("test".into()), None).unwrap();
    assert!(matches!(
        provider.decision_model("gpt-test"),
        Err(AiMuxError::UnsupportedFunctionality(_))
    ));
}

#[test]
fn official_defaults_and_capabilities() {
    let config = JevConfig::new("test-key");
    assert_eq!(config.endpoint, "https://api.typesafe.ai/v1/systemone");
    let model = JevProvider::new(config)
        .decision_model("jev-latest")
        .unwrap();
    let capabilities = model.capabilities();
    assert_eq!(capabilities.max_questions, None);
    assert_eq!(capabilities.min_choices, Some(1));
    assert_eq!(capabilities.max_choices, Some(255));
    assert_eq!(capabilities.max_score_levels, Some(10));
}

#[tokio::test]
async fn official_requests_accept_single_and_255_choices_and_more_than_20_questions() {
    for choice_count in [1, 255] {
        let server = MockServer::start().await;
        let id = "问题.with punctuation";
        let labels: Vec<String> = (0..choice_count).map(|i| format!("label{i}")).collect();
        let mut request = DecisionCallOptions::new(
            json!([{"ticket": "Help"}]),
            vec![DecisionQuestion::Choice {
                id: id.into(),
                instructions: "a".repeat(1001).into(),
                options: labels
                    .iter()
                    .map(|label| DecisionOption {
                        value: None,
                        label: label.clone(),
                        description: None,
                    })
                    .collect(),
            }],
        );
        let probabilities: serde_json::Map<String, Value> = labels
            .iter()
            .enumerate()
            .map(|(i, label)| (label.clone(), json!(if i == 0 { 1.0 } else { 0.0 })))
            .collect();
        let mut answers = serde_json::Map::from_iter([(
            id.to_string(),
            json!({"type":"choice", "choice":labels[0],
                "probabilities": probabilities, "confidence":1.0}),
        )]);
        for i in 0..21 {
            let id = format!("q{i}");
            request.questions.push(DecisionQuestion::Boolean {
                criteria: None,
                id: id.clone(),
                instructions: "Urgent?".into(),
            });
            answers.insert(id, json!({"type":"noul", "noul":1.0}));
        }
        mount(
            &server,
            200,
            json!({"model":"jev-1.13.0", "answers":answers,
            "usage":{"input_tokens":100,"output_tokens":10}}),
        )
        .await;
        decide(model(&server).as_ref(), request).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body["questions"][id]["criteria"].as_object().unwrap().len(),
            choice_count
        );
        assert_eq!(body["questions"][id]["criteria"][&labels[0]], Value::Null);
        assert_eq!(body["questions"].as_object().unwrap().len(), 22);
    }
}

#[tokio::test]
async fn structured_official_fields_and_legends_survive_without_stringification() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../contract-tests/fixtures/decision-native.json"
    ))
    .unwrap();
    let request: DecisionCallOptions = serde_json::from_value(fixture["request"].clone()).unwrap();
    let server = MockServer::start().await;
    mount(&server, 200, fixture["response"].clone()).await;
    let model = model(&server);
    let result = decide(model.as_ref(), request).await.unwrap();
    assert_eq!(result.rounding, model.capabilities().rounding);
    assert_eq!(result.rounding.probability_decimals, Some(2));
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["questions"]["urgent"]["instructions"],
        fixture["request"]["questions"][0]["instructions"]
    );
    assert_eq!(
        body["questions"]["urgent"]["criteria"],
        fixture["request"]["questions"][0]["criteria"]
    );
    assert_eq!(
        body["questions"]["department"]["criteria"]["billing"],
        fixture["request"]["questions"][1]["options"][0]["description"]
    );
    assert_eq!(
        body["questions"]["severity"]["criteria"],
        fixture["request"]["questions"][2]["levels"]
    );
    let result = serde_json::to_value(result).unwrap();
    assert_eq!(
        result["answers"]["severity"]["levels"],
        fixture["request"]["questions"][2]["levels"]
    );
}
