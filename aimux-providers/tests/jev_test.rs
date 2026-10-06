//! Offline contract tests using a documented official example. No live credentials.
use std::time::Duration;

use aimux_core::decision_model::*;
use aimux_core::{AiMuxError, Provider};
use aimux_providers::{JevConfig, JevProvider};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture() -> Value {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/jev_systemone.json")).unwrap();
    fixture["response"].clone()
}

fn options() -> DecisionCallOptions {
    DecisionCallOptions::new(
        json!({"ticket": "Billed twice"}),
        vec![
            DecisionQuestion::Boolean {
                id: "needs_human".into(),
                instructions: "Does this need a human?".into(),
            },
            DecisionQuestion::Choice {
                id: "queue".into(),
                instructions: "Which team?".into(),
                options: ["billing", "technical", "sales"]
                    .into_iter()
                    .map(|label| DecisionOption {
                        label: label.into(),
                        description: None,
                    })
                    .collect(),
            },
            DecisionQuestion::Score {
                id: "anger".into(),
                instructions: "How angry?".into(),
                levels: ["Calm", "Mildly annoyed", "Frustrated", "Angry"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            },
        ],
    )
}

fn model(server: &MockServer) -> Box<dyn DecisionModel> {
    let mut config =
        JevConfig::new("test-key").with_endpoint(format!("{}/api/v1/systemone/", server.uri()));
    config.retry_config.initial_delay = Duration::ZERO;
    JevProvider::new(config).decision_model("jev-1.13").unwrap()
}

async fn mount(server: &MockServer, status: u16, response: Value) {
    Mock::given(method("POST"))
        .and(path("/api/v1/systemone/"))
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
        matches!(result.answers["needs_human"], DecisionAnswer::Boolean { probability_true } if probability_true == 0.89)
    );
    assert!(
        matches!(&result.answers["queue"], DecisionAnswer::Choice { selected, probabilities: Some(p), .. } if selected == "billing" && p.len() == 3)
    );
    assert!(
        matches!(&result.answers["anger"], DecisionAnswer::Score { expected_value, probabilities: Some(p), .. } if *expected_value == 1.89 && p == &[0.0, 0.11, 0.89, 0.0])
    );
    assert_eq!(result.model_version.as_deref(), Some("jev-1.13-20260917"));
    assert_eq!(result.usage.as_ref().unwrap().input_tokens.total, Some(503));
    assert_eq!(
        result.usage.as_ref().unwrap().raw.as_ref().unwrap()["charged_tokens"],
        503
    );
    assert_eq!(
        result.response.as_ref().unwrap().body.as_ref().unwrap(),
        &fixture()
    );
    assert_eq!(
        result.provider_metadata.as_ref().unwrap()["jev"]["id"],
        "dec_contract_fixture"
    );
    assert_eq!(
        result.response.as_ref().unwrap().headers.as_ref().unwrap()["x-request-id"],
        "test-request"
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["state"], options().state);
    assert_eq!(
        body["questions"]["needs_human"],
        json!({"type":"noul", "instructions":"Does this need a human?"})
    );
    assert_eq!(
        body["questions"]["queue"]["criteria"],
        json!({"billing":"billing", "technical":"technical", "sales":"sales"})
    );
    assert_eq!(
        body["questions"]["anger"]["criteria"],
        json!(["Calm", "Mildly annoyed", "Frustrated", "Angry"])
    );
}

#[tokio::test]
async fn malformed_answers_fail_with_http_context_without_retry() {
    let mut responses = Vec::new();
    let mut missing = fixture();
    missing["answers"].as_object_mut().unwrap().remove("queue");
    responses.push(missing);
    let mut unknown = fixture();
    unknown["answers"]["queue"]["choice"] = json!("unknown");
    responses.push(unknown);
    let mut range = fixture();
    range["answers"]["needs_human"]["noul"] = json!(1.2);
    responses.push(range);
    let mut keys = fixture();
    keys["answers"]["anger"]["probabilities"] = json!({"0":0,"1":0.11,"2":0.89,"4":0});
    responses.push(keys);
    let mut wrong_type = fixture();
    wrong_type["answers"]["queue"] = json!({"type":"noul", "noul":0.8});
    responses.push(wrong_type);
    let mut distribution = fixture();
    distribution["answers"]["queue"]["probabilities"]["billing"] = json!(0.4);
    responses.push(distribution);
    let mut missing_distribution = fixture();
    missing_distribution["answers"]["queue"]
        .as_object_mut()
        .unwrap()
        .remove("probabilities");
    responses.push(missing_distribution);
    let mut legend = fixture();
    legend["answers"]["anger"]["legend"]["0"] = json!("Other");
    responses.push(legend);
    let mut score = fixture();
    score["answers"]["anger"]["score"] = json!(4);
    responses.push(score);
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
        assert!(error.url.ends_with("/api/v1/systemone/"));
        assert_eq!(error.request_body_values["model"], "jev-1.13");
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
    let mut invalid_id = options();
    invalid_id.questions[0] = DecisionQuestion::Boolean {
        id: "bad.id".into(),
        instructions: "test".into(),
    };
    let mut short_choice = options();
    if let DecisionQuestion::Choice { options, .. } = &mut short_choice.questions[1] {
        options.truncate(1);
    }
    let mut duplicate_label = options();
    if let DecisionQuestion::Choice { options, .. } = &mut duplicate_label.questions[1] {
        options.push(options[0].clone());
    }
    let mut too_many = options();
    for index in 0..20 {
        too_many.questions.push(DecisionQuestion::Boolean {
            id: format!("q{index}"),
            instructions: "test".into(),
        });
    }
    let mut null = options();
    null.state = Value::Null;
    for options in [
        duplicate,
        invalid_id,
        short_choice,
        duplicate_label,
        too_many,
        null,
    ] {
        assert!(matches!(
            decide(model(&server).as_ref(), options).await,
            Err(AiMuxError::InvalidArgument(_))
        ));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn provider_failures_use_jev_retry_rules_and_preserve_codes() {
    for (status, code, attempts) in [
        (401, "invalid_api_key", 1),
        (409, "request_in_progress", 1),
        (429, "api_key_spend_limit_exceeded", 1),
        (429, "rate_limit_exceeded", 3),
        (503, "service_unavailable", 3),
        (499, "request_cancelled", 3),
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
    let provider = aimux_providers::OpenAIProvider::new(aimux_providers::OpenAIConfig::new("test"));
    assert!(matches!(
        provider.decision_model("gpt-test"),
        Err(AiMuxError::UnsupportedFunctionality(_))
    ));
}
