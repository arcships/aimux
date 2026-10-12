//! Port of `google/src/google-evaluation.test.ts`.
//!
//! The upstream fixture `google/src/__fixtures__/evaluation.json` is inlined
//! below. Not ported: the `request.signal` identity assertion (the injected
//! [`Fetch`] sees a request, not the caller's `AbortSignal`).

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::AbortSignal;
use aimux_core::error::AiMuxError;
use aimux_core::evaluation_model::{
    EvaluationAnswer, EvaluationCallOptions, EvaluationModel, EvaluationQuestionType,
};
use aimux_core::provider::Provider;
use aimux_providers::google::{GoogleProviderSettings, create_google};

use mock_fetch::{Canned, MockFetch};

fn fixture() -> Value {
    json!({
        "candidates": [{
            "content": {
                "parts": [{ "text": "{\n  \"q0\": \"c1\",\n  \"q1\": 1.25\n}", "thoughtSignature": "sig" }],
                "role": "model"
            },
            "finishReason": "STOP",
            "index": 0
        }],
        "usageMetadata": {
            "promptTokenCount": 21,
            "candidatesTokenCount": 24,
            "totalTokenCount": 45,
            "promptTokensDetails": [{ "modality": "TEXT", "tokenCount": 21 }],
            "serviceTier": "standard"
        },
        "modelVersion": "gemini-3.5-flash-lite",
        "responseId": "a9-qapPZOLm4mtkPluzysAg"
    })
}

fn options() -> EvaluationCallOptions {
    serde_json::from_value(json!({
        "state": { "message": "A billing issue with a workaround." },
        "questions": {
            "department": {
                "type": "choice",
                "instructions": "Pick the team.",
                "criteria": { "technical": "Bugs", "billing": "Charges" }
            },
            "severity": {
                "type": "score",
                "instructions": ["Rate severity."],
                "criteria": ["Low", "Medium", "High"]
            }
        },
        "headers": null,
        "provider_options": null
    }))
    .unwrap()
}

fn setup(body: &Value) -> (Arc<dyn EvaluationModel>, Arc<MockFetch>) {
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
    .unwrap();
    let model = provider
        .evaluation_model("gemini-3.5-flash-lite")
        .unwrap()
        .unwrap();
    (model, mock)
}

fn with_candidate(mut body: Value, patch: Value) -> Value {
    body["candidates"][0]
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    body
}

fn parts(text: &str) -> Value {
    json!({ "content": { "role": "model", "parts": [{ "text": text }] } })
}

/// TS: uses Gemini structured output and forwards thinking options and cancellation
#[tokio::test]
async fn uses_structured_output_and_forwards_thinking_options() {
    let (model, mock) = setup(&fixture());
    let mut opts = options();
    opts.abort_signal = Some(AbortSignal::new());
    opts.headers = Some([("x-call".to_string(), "forwarded".to_string())].into());
    opts.provider_options = serde_json::from_value(
        json!({ "google": { "thinkingConfig": { "thinkingLevel": "high" } } }),
    )
    .ok();
    let result = model.do_evaluate(&opts).await.unwrap();
    assert_eq!(model.provider(), "google.evaluation");
    assert_eq!(
        model.supported_question_types(),
        [
            EvaluationQuestionType::Choice,
            EvaluationQuestionType::Score,
            EvaluationQuestionType::Boolean
        ]
    );
    assert_eq!(
        result.answers["department"],
        EvaluationAnswer::Choice {
            choice: "billing".into(),
            probabilities: None
        }
    );
    assert_eq!(
        result.answers["severity"],
        EvaluationAnswer::Score {
            score: 1.25,
            probabilities: None
        }
    );
    let usage = result.usage.unwrap();
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        (Some(21), Some(24))
    );
    let response = result.response.unwrap();
    assert_eq!(response.headers.unwrap()["x-request-id"], "req-test");
    assert_eq!(model.model_id(), "gemini-3.5-flash-lite");
    assert_eq!(response.id.as_deref(), Some("a9-qapPZOLm4mtkPluzysAg"));
    assert!(result.provider_metadata.unwrap().contains_key("google"));
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://example.com/v1beta/models/gemini-3.5-flash-lite:generateContent"
    );
    assert_eq!(seen[0].headers["x-goog-api-key"], "test-key");
    assert_eq!(seen[0].headers["x-provider"], "configured");
    assert_eq!(seen[0].headers["x-call"], "forwarded");
    let config = &seen[0].json_body()["generationConfig"];
    assert_eq!(config["responseMimeType"], "application/json");
    assert_eq!(config["thinkingConfig"]["thinkingLevel"], "high");
    let schema = &config["responseJsonSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["properties"]["q0"],
        json!({ "type": "string", "enum": ["c0", "c1"] })
    );
    assert_eq!(schema["properties"]["q1"]["type"], "number");
    assert_eq!(schema["required"], json!(["q0", "q1"]));
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["properties"]["q1"].get("minimum").is_none());
    assert!(schema["properties"]["q1"].get("maximum").is_none());
}

/// TS: defaults to the minimum thinking level through the provider reasoning mapping
#[tokio::test]
async fn defaults_to_the_minimum_thinking_level() {
    let (model, mock) = setup(&fixture());
    model.do_evaluate(&options()).await.unwrap();
    assert_eq!(
        mock.seen()[0].json_body()["generationConfig"]["thinkingConfig"],
        json!({ "thinkingLevel": "minimal" })
    );
}

/// TS: ignores thought text while counting reasoning tokens in usage
#[tokio::test]
async fn ignores_thought_text_while_counting_reasoning_tokens() {
    let mut body = fixture();
    let answer = body["candidates"][0]["content"]["parts"][0].clone();
    body["candidates"][0]["content"] = json!({ "role": "model", "parts": [{ "thought": true, "text": "Internal reasoning" }, answer] });
    body["usageMetadata"]["thoughtsTokenCount"] = json!(10);
    body["usageMetadata"]["totalTokenCount"] = json!(55);
    let (model, _) = setup(&body);
    let result = model.do_evaluate(&options()).await.unwrap();
    assert_eq!(
        result.answers["severity"],
        EvaluationAnswer::Score {
            score: 1.25,
            probabilities: None
        }
    );
    assert_eq!(result.usage.unwrap().output_tokens, Some(24 + 10));
}

/// TS: rejects %s even with valid JSON
#[tokio::test]
async fn rejects_unfinished_generations_even_with_valid_json() {
    for finish_reason in ["SAFETY", "MAX_TOKENS"] {
        let (model, _) = setup(&with_candidate(
            fixture(),
            json!({ "finishReason": finish_reason }),
        ));
        let error = model.do_evaluate(&options()).await.unwrap_err();
        assert!(
            matches!(error, AiMuxError::InvalidResponseData(_)),
            "got {error:?}"
        );
    }
}

/// TS: validates score bounds locally
#[tokio::test]
async fn validates_score_bounds_locally() {
    let (model, _) = setup(&with_candidate(fixture(), parts(r#"{"q0":"c1","q1":3}"#)));
    let error = model.do_evaluate(&options()).await.unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidResponseData(_)),
        "got {error:?}"
    );
}

/// TS: evaluates Boolean alongside Choice and Score in one Gemini request
#[tokio::test]
async fn evaluates_boolean_alongside_choice_and_score_in_one_request() {
    let (model, mock) = setup(&with_candidate(
        fixture(),
        parts(r#"{"q0":"c1","q1":1.25,"q2":0.02}"#),
    ));
    let mut opts = options();
    opts.questions.insert(
        "flag".into(),
        serde_json::from_value(
            json!({ "type": "boolean", "instructions": "Is a refund requested?" }),
        )
        .unwrap(),
    );
    let result = model.do_evaluate(&opts).await.unwrap();
    assert_eq!(
        result.answers["flag"],
        EvaluationAnswer::Boolean { probability: 0.02 }
    );
    assert_eq!(
        result.answers["department"],
        EvaluationAnswer::Choice {
            choice: "billing".into(),
            probabilities: None
        }
    );
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    let schema = &seen[0].json_body()["generationConfig"]["responseJsonSchema"];
    assert_eq!(schema["properties"]["q2"]["type"], "number");
    assert_eq!(schema["required"], json!(["q0", "q1", "q2"]));
}

/// TS: rejects an aborted evaluation without a request
#[tokio::test]
async fn rejects_an_aborted_evaluation_without_a_request() {
    let (model, mock) = setup(&fixture());
    let signal = AbortSignal::new();
    signal.abort();
    let mut opts = options();
    opts.abort_signal = Some(signal);
    let error = model.do_evaluate(&opts).await.unwrap_err();
    assert!(matches!(error, AiMuxError::Aborted(_)));
    assert!(mock.seen().is_empty());
}
