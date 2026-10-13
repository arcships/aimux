//! Offline contract tests using official examples and recorded live responses.
//! No live credentials or external requests are needed to replay them.

use std::sync::Arc;

use aimux_core::decision_model::*;
use aimux_core::{AiMuxError, Provider};
use aimux_providers::{JevConfig, JevProvider};
use indexmap::IndexMap;
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
        let input = usage["input_tokens"].as_u64().map(|n| n as u32);
        let output = usage["output_tokens"].as_u64().map(|n| n as u32);
        assert_eq!(
            result.usage,
            DecideUsage {
                input_tokens: input,
                output_tokens: output,
                total_tokens: input.zip(output).map(|(i, o)| i + o),
            }
        );
        assert_eq!(result.response.body.unwrap()["usage"], usage);
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
            // ai@7.0.128 src/decide/validate-decision.ts:233-242
            assert!(
                error.message.ends_with(
                    "Question \"department\" did not select a highest-probability option."
                ),
                "{}",
                error.message
            );
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
        let request: DecideOptions =
            serde_json::from_value(recording.input.options.clone()).unwrap();
        let result = decide(model(&server).as_ref(), request).await.unwrap();
        let result = serde_json::to_value(result).unwrap();
        let expected = recording.outcome.decision_result.unwrap();
        for field in ["answers", "rounding", "usage", "provider_metadata"] {
            assert_eq!(result[field], expected[field]);
        }
        assert_eq!(result["response"]["body"], expected["response"]["body"]);
        assert_eq!(result["response"]["modelId"], "jev-1.13.0");
    }
}

fn questions() -> DecisionQuestions {
    serde_json::from_value(json!({
        "is_urgent": {
            "type": "boolean",
            "instructions": "The message conveys urgency or time-sensitivity"
        },
        "department": {
            "type": "choice",
            "instructions": "Which team should handle this",
            "criteria": {"billing": null, "technical": null, "sales": null}
        },
        "frustration": {
            "type": "score",
            "instructions": "How frustrated the customer appears",
            "criteria": [
                "Calm, just stating facts",
                "Frustrated but civil",
                "Very angry, strong language"
            ]
        }
    }))
    .unwrap()
}

fn options() -> DecideOptions {
    let state: DecisionInput = serde_json::from_value(json!({"ticket": "Billed twice"})).unwrap();
    DecideOptions::new(state, questions())
}

fn model(server: &MockServer) -> Arc<dyn DecisionModel> {
    let config = JevConfig::new("test-key").with_endpoint(format!("{}/v1/systemone", server.uri()));
    JevProvider::new(config)
        .decision_model("jev-latest")
        .unwrap()
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
    assert_eq!(
        result.answers.keys().collect::<Vec<_>>(),
        ["is_urgent", "department", "frustration"]
    );
    assert_eq!(
        result.answers["is_urgent"],
        DecisionAnswer::Boolean { probability: 1.0 }
    );
    assert_eq!(
        result.answers["department"],
        DecisionAnswer::Choice {
            choice: "technical".into(),
            probabilities: Some(IndexMap::from([
                ("technical".to_string(), 0.85),
                ("sales".to_string(), 0.0),
                ("billing".to_string(), 0.15),
            ])),
        }
    );
    assert_eq!(
        result.answers["frustration"],
        DecisionAnswer::Score {
            score: 1.0,
            probabilities: Some(IndexMap::from([
                ("0".to_string(), 0.0),
                ("1".to_string(), 1.0),
                ("2".to_string(), 0.0),
            ])),
        }
    );
    assert_eq!(result.response.model_id.as_deref(), Some("jev-1.13.0"));
    assert_eq!(
        result.usage,
        DecideUsage {
            input_tokens: Some(392),
            output_tokens: Some(65),
            total_tokens: Some(457),
        }
    );
    assert_eq!(
        result.rounding,
        Some(DecisionRounding {
            probability_decimals: Some(2),
            score_decimals: Some(2),
        })
    );
    assert_eq!(result.response.body.as_ref().unwrap(), &fixture());
    assert_eq!(
        Value::Object(result.provider_metadata.as_ref().unwrap()["jev"].clone()),
        json!({
            "probability_source": "native",
            "answers": {"department": {"confidence": 0.78}, "frustration": {"confidence": 1.0}}
        })
    );
    assert_eq!(
        result.response.headers.as_ref().unwrap()["x-request-id"],
        "test-request"
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["state"], json!({"ticket": "Billed twice"}));
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
    let mut extra = fixture();
    extra["answers"]["unasked"] = json!({"type":"noul", "noul":0.5});
    responses.push(extra);
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
    let mut empty_choice = options();
    if let Some(DecisionQuestion::Choice { criteria, .. }) =
        empty_choice.questions.get_mut("department")
    {
        criteria.clear();
    }
    let mut too_many_choices = options();
    if let Some(DecisionQuestion::Choice { criteria, .. }) =
        too_many_choices.questions.get_mut("department")
    {
        *criteria = (0..256)
            .map(|index| (format!("option{index}"), None))
            .collect();
    }
    let mut too_many_levels = options();
    if let Some(DecisionQuestion::Score { criteria, .. }) =
        too_many_levels.questions.get_mut("frustration")
    {
        *criteria = (0..11)
            .map(|index| Some(format!("level{index}").into()))
            .collect();
    }
    let mut no_questions = options();
    no_questions.questions.clear();
    for options in [
        empty_choice,
        too_many_choices,
        too_many_levels,
        no_questions,
    ] {
        assert!(matches!(
            decide(model(&server).as_ref(), options).await,
            Err(AiMuxError::InvalidArgument(_))
        ));
    }
    let mut provider_options = options();
    provider_options.provider_options = Some(
        [(
            "jev".to_string(),
            json!({"x": 1}).as_object().unwrap().clone(),
        )]
        .into(),
    );
    assert!(matches!(
        decide(model(&server).as_ref(), provider_options).await,
        Err(AiMuxError::UnsupportedFunctionality(_))
    ));
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

/// `ai@7.0.128 src/decide/decision-provider.ts:7-11`: a provider without
/// decision models offers none, and the registry reports `NoSuchModel`
/// (`src/registry/provider-registry.ts:447-461`).
#[test]
fn non_decision_provider_offers_no_decision_model() {
    let provider = aimux_providers::create_provider(
        "deepseek",
        aimux_providers::PresetSettings {
            api_key: Some("test".to_string().into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(provider.decision_model("gpt-test").is_none());
    let registry = aimux_core::create_provider_registry(
        std::collections::BTreeMap::from([("deepseek".to_string(), provider)]),
        Default::default(),
    );
    assert!(matches!(
        registry.decision_model("deepseek:gpt-test"),
        Err(AiMuxError::NoSuchModel { model_type, .. }) if model_type == "decisionModel"
    ));
}

/// `@ai-sdk/provider@4.0.22 src/decision-model/v4/decision-model-v4.ts:6-16`.
#[test]
fn official_defaults_and_model_properties() {
    let config = JevConfig::new("test-key");
    assert_eq!(config.endpoint, "https://api.typesafe.ai/v1/systemone");
    let model = JevProvider::new(config)
        .decision_model("jev-latest")
        .unwrap()
        .unwrap();
    assert_eq!(model.specification_version(), "v4");
    assert_eq!(model.provider(), "jev");
    assert_eq!(model.model_id(), "jev-latest");
    assert_eq!(model.supported_question_types(), DecisionQuestionType::ALL);
}

#[tokio::test]
async fn official_requests_accept_single_and_255_choices_and_more_than_20_questions() {
    for choice_count in [1, 255] {
        let server = MockServer::start().await;
        let id = "问题.with punctuation";
        let labels: Vec<String> = (0..choice_count).map(|i| format!("label{i}")).collect();
        let mut questions = DecisionQuestions::new();
        questions.insert(
            id.into(),
            DecisionQuestion::Choice {
                instructions: "a".repeat(1001).into(),
                criteria: labels.iter().map(|label| (label.clone(), None)).collect(),
            },
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
            questions.insert(
                id.clone(),
                DecisionQuestion::Boolean {
                    instructions: "Urgent?".into(),
                    criteria: None,
                },
            );
            answers.insert(id, json!({"type":"noul", "noul":1.0}));
        }
        let state: DecisionInput = serde_json::from_value(json!([{"ticket": "Help"}])).unwrap();
        mount(
            &server,
            200,
            json!({"model":"jev-1.13.0", "answers":answers,
            "usage":{"input_tokens":100,"output_tokens":10}}),
        )
        .await;
        decide(
            model(&server).as_ref(),
            DecideOptions::new(state, questions),
        )
        .await
        .unwrap();
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
    let request: DecideOptions = serde_json::from_value(fixture["request"].clone()).unwrap();
    let server = MockServer::start().await;
    mount(&server, 200, fixture["response"].clone()).await;
    let model = model(&server);
    let result = decide(model.as_ref(), request).await.unwrap();
    assert_eq!(result.rounding.unwrap().probability_decimals, Some(2));
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let questions = &fixture["request"]["questions"];
    assert_eq!(
        body["questions"]["urgent"]["instructions"],
        questions["urgent"]["instructions"]
    );
    assert_eq!(
        body["questions"]["urgent"]["criteria"],
        questions["urgent"]["criteria"]
    );
    assert_eq!(
        body["questions"]["department"]["criteria"],
        questions["department"]["criteria"]
    );
    assert_eq!(
        body["questions"]["severity"]["criteria"],
        questions["severity"]["criteria"]
    );
    assert_eq!(
        serde_json::to_value(&result.answers["severity"]).unwrap(),
        json!({"type": "score", "score": 0.25, "probabilities": {"0": 0.75, "1": 0.25}})
    );
}
