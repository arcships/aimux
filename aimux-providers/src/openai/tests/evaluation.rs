use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::openai::{OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_core::decision_model::{
    DecisionCallOptions, DecisionModel, DecisionQuestionType, DecisionQuestions,
};
use aimux_core::error::AiMuxError;
use aimux_provider_utils::DecisionLanguageModel;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/evaluation.json")).unwrap()
}

fn questions(value: Value) -> DecisionQuestions {
    serde_json::from_value(value).unwrap()
}

fn base_questions() -> Value {
    json!({
        "department": {
            "type": "choice",
            "instructions": "Pick the team.",
            "criteria": { "technical": "Bugs", "billing": "Charges" },
        },
        "severity": {
            "type": "score",
            "instructions": "Rate severity.",
            "criteria": ["Low", "Medium", "High"],
        },
    })
}

/// The upstream `options`.
fn options() -> DecisionCallOptions {
    DecisionCallOptions::new(
        "A billing issue with a workaround.",
        questions(base_questions()),
    )
}

/// `setup(body)`: the evaluation model on a mock transport answering `body`.
fn setup(body: &Value) -> (DecisionLanguageModel, Arc<MockFetch>) {
    let mut response = Canned::json(body);
    response
        .headers
        .push(("x-request-id".into(), "req-test".into()));
    let fetch = MockFetch::new(vec![response]);
    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some("https://example.com/v1".into()),
        headers: Some(HashMap::from([(
            "x-provider".to_string(),
            Some("configured".to_string()),
        )])),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    (provider.evaluation("gpt-5.6-luna"), fetch)
}

/// `toMatchObject`: every key of `expected` matches in `actual` (objects
/// recursively, arrays element by element with the same length).
fn assert_matches_object(actual: &Value, expected: &Value) {
    fn matches(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => expected
                .iter()
                .all(|(key, value)| actual.get(key).is_some_and(|a| matches(a, value))),
            (Value::Array(actual), Value::Array(expected)) => {
                actual.len() == expected.len()
                    && actual.iter().zip(expected).all(|(a, e)| matches(a, e))
            }
            _ => actual == expected,
        }
    }
    assert!(
        matches(actual, expected),
        "expected {actual:#} to match {expected:#}"
    );
}

/// upstream: @ai-sdk/openai src/openai-evaluation.test.ts › evaluates through the configured Responses API with strict structured output
#[tokio::test]
async fn evaluates_through_the_configured_responses_api_with_strict_structured_output() {
    let fixture = fixture();
    let (model, fetch) = setup(&fixture);

    let result = model
        .do_decide(&DecisionCallOptions {
            headers: Some(HashMap::from([(
                "x-call".to_string(),
                "forwarded".to_string(),
            )])),
            ..options()
        })
        .await
        .unwrap();

    assert_eq!(model.provider(), "openai.evaluation");
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
        serde_json::to_value(result.usage).unwrap(),
        json!({
            "inputTokens": fixture["usage"]["input_tokens"],
            "outputTokens": fixture["usage"]["output_tokens"],
        })
    );
    assert_eq!(
        result
            .response
            .as_ref()
            .and_then(|response| response.headers.as_ref())
            .and_then(|headers| headers.get("x-request-id"))
            .map(String::as_str),
        Some("req-test")
    );
    assert!(
        result
            .provider_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.contains_key("openai"))
    );
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "https://example.com/v1/responses");
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer test-key")
    );
    assert_eq!(
        seen[0].headers.get("x-provider").map(String::as_str),
        Some("configured")
    );
    assert_eq!(
        seen[0].headers.get("x-call").map(String::as_str),
        Some("forwarded")
    );
    assert_matches_object(
        &seen[0].json_body(),
        &json!({
            "model": "gpt-5.6-luna",
            "reasoning": { "effort": "none" },
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "evaluation",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "required": ["q0", "q1"],
                        "additionalProperties": false,
                        "properties": {
                            "q0": { "type": "string", "enum": ["c0", "c1"] },
                            "q1": { "type": "number" },
                        },
                    },
                },
            },
        }),
    );
}

/// upstream: @ai-sdk/openai src/openai-evaluation.test.ts › allows provider options to override the default reasoning effort
#[tokio::test]
async fn allows_provider_options_to_override_the_default_reasoning_effort() {
    let (model, fetch) = setup(&fixture());

    model
        .do_decide(&DecisionCallOptions {
            provider_options: Some(
                serde_json::from_value(json!({ "openai": { "reasoningEffort": "high" } })).unwrap(),
            ),
            ..options()
        })
        .await
        .unwrap();

    assert_matches_object(
        &fetch.seen()[0].json_body()["reasoning"],
        &json!({ "effort": "high" }),
    );
}

/// upstream: @ai-sdk/openai src/openai-evaluation.test.ts › evaluates Boolean alongside Choice and Score in one Responses request
#[tokio::test]
async fn evaluates_boolean_alongside_choice_and_score_in_one_responses_request() {
    let mut body = fixture();
    body["output"][0]["content"][0]["text"] = json!(r#"{"q0":"c1","q1":1.25,"q2":0.02}"#);
    let (model, fetch) = setup(&body);
    let mut all_questions = base_questions();
    all_questions["flag"] = json!({ "type": "boolean", "instructions": "Is a refund requested?" });

    let result = model
        .do_decide(&DecisionCallOptions {
            questions: questions(all_questions),
            ..options()
        })
        .await
        .unwrap();

    assert_eq!(
        serde_json::to_value(&result.answers).unwrap(),
        json!({
            "department": { "type": "choice", "choice": "billing" },
            "severity": { "type": "score", "score": 1.25 },
            "flag": { "type": "boolean", "probability": 0.02 },
        })
    );
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_matches_object(
        &seen[0].json_body()["text"]["format"]["schema"],
        &json!({
            "properties": { "q2": { "type": "number" } },
            "required": ["q0", "q1", "q2"],
        }),
    );
}

/// upstream: @ai-sdk/openai src/openai-evaluation.test.ts › rejects truncated output even when its text is valid JSON
#[tokio::test]
async fn rejects_truncated_output_even_when_its_text_is_valid_json() {
    let mut body = fixture();
    body["status"] = json!("incomplete");
    body["incomplete_details"] = json!({ "reason": "max_output_tokens" });
    let (model, _fetch) = setup(&body);

    let error = model.do_decide(&options()).await.unwrap_err();

    assert!(
        matches!(error, AiMuxError::InvalidResponseData { .. }),
        "{error:?}"
    );
}

/// upstream: @ai-sdk/openai src/openai-evaluation.test.ts › rejects a refusal instead of returning partial answers
#[tokio::test]
async fn rejects_a_refusal_instead_of_returning_partial_answers() {
    let mut body = fixture();
    body["output"] = json!([{
        "type": "message",
        "id": "msg-refusal",
        "role": "assistant",
        "status": "completed",
        "content": [{ "type": "refusal", "refusal": "Cannot evaluate this." }],
    }]);
    let (model, _fetch) = setup(&body);

    let error = model.do_decide(&options()).await.unwrap_err();

    assert!(matches!(error, AiMuxError::ApiCall(_)), "{error:?}");
}
