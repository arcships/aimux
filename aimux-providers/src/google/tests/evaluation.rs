//! `evaluationModel` (`google-evaluation.test.ts`). In aimux the evaluation
//! model is the provider's `decision_model` and `doEvaluate` is
//! `do_decide`. The other cases of this file keep their ports in
//! `aimux-providers/tests/google_evaluation_test.rs`.
//!
//! Not asserted: `request.signal` being the caller's `AbortSignal` (the
//! transport sees a request, not the caller's signal object).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::AbortSignal;
use aimux_core::decision_model::{DecisionCallOptions, DecisionModel, DecisionQuestionType};
use aimux_core::provider::Provider;

use crate::google::{GoogleProviderSettings, create_google};
use crate::test_support::mock_fetch::{Canned, MockFetch};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/evaluation.json")).expect("fixture is JSON")
}

fn options() -> Value {
    json!({
        "state": { "message": "A billing issue with a workaround." },
        "questions": {
            "department": {
                "type": "choice",
                "instructions": "Pick the team.",
                "criteria": { "technical": "Bugs", "billing": "Charges" },
            },
            "severity": {
                "type": "score",
                "instructions": ["Rate severity."],
                "criteria": ["Low", "Medium", "High"],
            },
        },
    })
}

fn call_options(value: Value) -> DecisionCallOptions {
    serde_json::from_value(value).expect("decision options deserialize")
}

fn setup(body: &Value) -> (Arc<dyn DecisionModel>, Arc<MockFetch>) {
    let mock = MockFetch::new(vec![Canned {
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("x-request-id".into(), "req-test".into()),
        ],
        ..Canned::json(body)
    }]);
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some("https://example.com/v1beta".into()),
        headers: Some(HashMap::from([(
            "x-provider".to_string(),
            Some("configured".to_string()),
        )])),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("valid settings");
    let model = provider
        .decision_model("gemini-3.5-flash-lite")
        .expect("the provider has decision models")
        .expect("model is created");
    (model, mock)
}

/// upstream: @ai-sdk/google src/google-evaluation.test.ts › uses Gemini structured output and forwards thinking options and cancellation
#[tokio::test]
async fn uses_gemini_structured_output_and_forwards_thinking_options_and_cancellation() {
    let fixture = fixture();
    let (model, mock) = setup(&fixture);
    let mut options = call_options(options());
    options.abort_signal = Some(AbortSignal::new());
    options.headers = Some([("x-call".to_string(), "forwarded".to_string())].into());
    options.provider_options = Some(
        serde_json::from_value(
            json!({ "google": { "thinkingConfig": { "thinkingLevel": "high" } } }),
        )
        .unwrap(),
    );

    let result = model.do_decide(&options).await.unwrap();

    assert_eq!(model.provider(), "google.evaluation");
    assert_eq!(
        model.supported_question_types(),
        [
            DecisionQuestionType::Choice,
            DecisionQuestionType::Score,
            DecisionQuestionType::Boolean,
        ]
    );
    assert_eq!(
        serde_json::to_value(&result.answers).unwrap(),
        json!({
            "department": { "type": "choice", "choice": "billing" },
            "severity": { "type": "score", "score": 1.25 },
        })
    );
    assert_eq!(
        serde_json::to_value(result.usage.as_ref().unwrap()).unwrap(),
        json!({
            "inputTokens": fixture["usageMetadata"]["promptTokenCount"],
            "outputTokens": fixture["usageMetadata"]["candidatesTokenCount"],
        })
    );
    let response = result.response.as_ref().unwrap();
    assert_eq!(
        response.headers.as_ref().unwrap()["x-request-id"],
        "req-test"
    );
    assert_eq!(model.model_id(), "gemini-3.5-flash-lite");
    assert_eq!(
        response.id.as_deref(),
        fixture["responseId"].as_str(),
        "response id"
    );
    assert!(
        result
            .provider_metadata
            .as_ref()
            .unwrap()
            .contains_key("google")
    );
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://example.com/v1beta/models/gemini-3.5-flash-lite:generateContent"
    );
    assert_eq!(seen[0].headers["x-goog-api-key"], "test-key");
    assert_eq!(seen[0].headers["x-provider"], "configured");
    assert_eq!(seen[0].headers["x-call"], "forwarded");
    let body = seen[0].json_body();
    let generation_config = &body["generationConfig"];
    assert_eq!(generation_config["responseMimeType"], "application/json");
    assert_eq!(generation_config["thinkingConfig"]["thinkingLevel"], "high");
    let schema = &generation_config["responseJsonSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["properties"]["q0"],
        json!({ "type": "string", "enum": ["c0", "c1"] })
    );
    assert_eq!(schema["properties"]["q1"]["type"], "number");
    assert_eq!(schema["required"], json!(["q0", "q1"]));
    assert_eq!(schema["additionalProperties"], json!(false));
    assert!(schema["properties"]["q1"].get("minimum").is_none());
    assert!(schema["properties"]["q1"].get("maximum").is_none());
}

/// upstream: @ai-sdk/google src/google-evaluation.test.ts › evaluates Boolean alongside Choice and Score in one Gemini request
#[tokio::test]
async fn evaluates_boolean_alongside_choice_and_score_in_one_gemini_request() {
    let mut body = fixture();
    body["candidates"][0]["content"]["parts"] =
        json!([{ "text": r#"{"q0":"c1","q1":1.25,"q2":0.02}"# }]);
    let (model, mock) = setup(&body);
    let mut value = options();
    value["questions"]["flag"] =
        json!({ "type": "boolean", "instructions": "Is a refund requested?" });

    let result = model.do_decide(&call_options(value)).await.unwrap();

    assert_eq!(
        serde_json::to_value(&result.answers).unwrap(),
        json!({
            "department": { "type": "choice", "choice": "billing" },
            "severity": { "type": "score", "score": 1.25 },
            "flag": { "type": "boolean", "probability": 0.02 },
        })
    );
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    let schema = &seen[0].json_body()["generationConfig"]["responseJsonSchema"];
    assert_eq!(schema["properties"]["q2"]["type"], "number");
    assert_eq!(schema["required"], json!(["q0", "q1", "q2"]));
}
