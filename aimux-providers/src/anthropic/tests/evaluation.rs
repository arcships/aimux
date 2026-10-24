use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::decision_model::{DecisionCallOptions, DecisionModel, DecisionUsage};
use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::shared::provider_namespace;

use crate::anthropic::options::CANONICAL;
use crate::anthropic::{AnthropicProviderSettings, create_anthropic};
use crate::test_support::mock_fetch::{Canned, RouteFetch, Seen};

const FIXTURE: &str = include_str!("fixtures/evaluation.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

/// `{ ...fixture, ...overrides }`.
fn fixture_with(overrides: Value) -> Value {
    let mut body = fixture();
    for (key, value) in overrides.as_object().unwrap() {
        body[key] = value.clone();
    }
    body
}

/// The upstream `options`: one state and a Choice and a Score question.
fn options() -> DecisionCallOptions {
    options_with_questions(json!({
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
    }))
}

fn options_with_questions(questions: Value) -> DecisionCallOptions {
    serde_json::from_value(json!({
        "state": { "message": "A billing issue with a workaround." },
        "questions": questions,
    }))
    .unwrap()
}

fn anthropic_options(options: Value) -> Option<aimux_core::shared::SharedProviderOptions> {
    Some(provider_namespace(CANONICAL, options).unwrap())
}

/// `setup(body)`: every request is answered with `body` (and a
/// `request-id` header); the model is `evaluationModel('claude-haiku-4-5-20251001')`.
fn setup(body: Value) -> (Arc<dyn DecisionModel>, Arc<RouteFetch>) {
    let fetch = RouteFetch::new(move |_| Canned {
        status: 200,
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("request-id".into(), "req-test".into()),
        ],
        body: serde_json::to_vec(&body).unwrap(),
    });
    let provider = create_anthropic(AnthropicProviderSettings {
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
    let model = provider
        .decision_model("claude-haiku-4-5-20251001")
        .unwrap()
        .unwrap();
    (model, fetch)
}

fn body_of(call: &Seen) -> Value {
    call.json_body()
}

/// `expect(actual).toMatchObject(expected)`: objects match by subset,
/// arrays element by element with the same length.
fn assert_match_object(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            for (key, expected) in expected {
                let actual = actual
                    .get(key)
                    .unwrap_or_else(|| panic!("missing {path}.{key}"));
                assert_match_object(actual, expected, &format!("{path}.{key}"));
            }
        }
        (Value::Array(actual), Value::Array(expected)) => {
            assert_eq!(actual.len(), expected.len(), "length of {path}");
            for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert_match_object(actual, expected, &format!("{path}[{index}]"));
            }
        }
        _ => assert_eq!(actual, expected, "at {path}"),
    }
}

/// upstream: @ai-sdk/anthropic src/anthropic-evaluation.test.ts › uses native Messages output with portable constraints and configured settings
#[tokio::test]
async fn uses_native_messages_output_with_portable_constraints_and_configured_settings() {
    let (model, fetch) = setup(fixture());
    let result = model
        .do_decide(&DecisionCallOptions {
            headers: Some(HashMap::from([(
                "x-call".to_string(),
                "forwarded".to_string(),
            )])),
            provider_options: anthropic_options(json!({ "structuredOutputMode": "outputFormat" })),
            ..options()
        })
        .await
        .unwrap();
    assert_eq!(model.provider(), "anthropic.evaluation");
    assert_eq!(
        serde_json::to_value(model.supported_question_types()).unwrap(),
        json!(["choice", "score", "boolean"])
    );
    assert_eq!(
        serde_json::to_value(&result.answers).unwrap(),
        json!({
            "department": { "type": "choice", "choice": "billing" },
            "severity": { "type": "score", "score": 1.25 },
        })
    );
    let fixture = fixture();
    assert_eq!(
        result.usage,
        Some(DecisionUsage {
            input_tokens: Some(fixture["usage"]["input_tokens"].as_u64().unwrap() as u32),
            output_tokens: Some(fixture["usage"]["output_tokens"].as_u64().unwrap() as u32),
        })
    );
    assert_eq!(
        result
            .response
            .as_ref()
            .and_then(|response| response.headers.as_ref())
            .and_then(|headers| headers.get("request-id"))
            .map(String::as_str),
        Some("req-test")
    );
    assert!(
        result
            .provider_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.contains_key(CANONICAL))
    );
    let calls = fetch.seen();
    assert_eq!(calls.len(), 1);
    let request = &calls[0];
    assert_eq!(request.url, "https://example.com/v1/messages");
    assert_eq!(
        request.headers.get("x-api-key").map(String::as_str),
        Some("test-key")
    );
    assert_eq!(
        request.headers.get("x-provider").map(String::as_str),
        Some("configured")
    );
    assert_eq!(
        request.headers.get("x-call").map(String::as_str),
        Some("forwarded")
    );
    let body = body_of(request);
    assert_match_object(
        &body,
        &json!({
            "model": "claude-haiku-4-5-20251001",
            "thinking": { "type": "disabled" },
            "output_config": {
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "q0": { "type": "string", "enum": ["c0", "c1"] },
                            "q1": { "type": "number" },
                        },
                        "required": ["q0", "q1"],
                        "additionalProperties": false,
                    },
                },
            },
        }),
        "body",
    );
    let q1 = &body["output_config"]["format"]["schema"]["properties"]["q1"];
    assert!(q1.get("minimum").is_none(), "{q1}");
    assert!(q1.get("maximum").is_none(), "{q1}");
}

/// upstream: @ai-sdk/anthropic src/anthropic-evaluation.test.ts › allows provider options to enable thinking for evaluations
#[tokio::test]
async fn allows_provider_options_to_enable_thinking_for_evaluations() {
    let (model, fetch) = setup(fixture());
    model
        .do_decide(&DecisionCallOptions {
            provider_options: anthropic_options(
                json!({ "thinking": { "type": "enabled", "budgetTokens": 1024 } }),
            ),
            ..options()
        })
        .await
        .unwrap();
    assert_eq!(
        body_of(&fetch.seen()[0])["thinking"],
        json!({ "type": "enabled", "budget_tokens": 1024 })
    );
}

/// upstream: @ai-sdk/anthropic src/anthropic-evaluation.test.ts › selects native structured output automatically for supported models
#[tokio::test]
async fn selects_native_structured_output_automatically_for_supported_models() {
    let (model, fetch) = setup(fixture());
    model.do_decide(&options()).await.unwrap();
    assert_eq!(
        body_of(&fetch.seen()[0]).pointer("/output_config/format/type"),
        Some(&json!("json_schema"))
    );
}

/// upstream: @ai-sdk/anthropic src/anthropic-evaluation.test.ts › supports the existing JSON-tool fallback through provider options
#[tokio::test]
async fn supports_the_existing_json_tool_fallback_through_provider_options() {
    let (model, fetch) = setup(fixture_with(json!({
        "stop_reason": "tool_use",
        "content": [{
            "type": "tool_use",
            "id": "tool-test",
            "name": "json",
            "input": { "q0": "c1", "q1": 1.25 },
        }],
    })));
    let result = model
        .do_decide(&DecisionCallOptions {
            provider_options: anthropic_options(json!({ "structuredOutputMode": "jsonTool" })),
            ..options()
        })
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&result.answers["department"]).unwrap(),
        json!({ "type": "choice", "choice": "billing" })
    );
    let body = body_of(&fetch.seen()[0]);
    assert!(body.pointer("/output_config/format").is_none(), "{body}");
    assert_match_object(
        &body["tool_choice"],
        &json!({ "type": "any", "disable_parallel_tool_use": true }),
        "tool_choice",
    );
    let tools = body["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 1, "{tools:?}");
    assert_eq!(tools[0]["name"], "json");
}

/// upstream: @ai-sdk/anthropic src/anthropic-evaluation.test.ts › rejects %s even with valid JSON
#[tokio::test]
async fn rejects_s_even_with_valid_json() {
    for stop_reason in ["refusal", "max_tokens"] {
        let (model, _) = setup(fixture_with(json!({ "stop_reason": stop_reason })));
        let error = model.do_decide(&options()).await.unwrap_err();
        assert!(
            matches!(error, AiMuxError::InvalidResponseData { .. }),
            "{stop_reason}: {error:?}"
        );
    }
}

/// upstream: @ai-sdk/anthropic src/anthropic-evaluation.test.ts › validates score bounds locally
#[tokio::test]
async fn validates_score_bounds_locally() {
    let (model, _) = setup(fixture_with(json!({
        "content": [{ "type": "text", "text": "{\"q0\":\"c1\",\"q1\":3}" }],
    })));
    let error = model.do_decide(&options()).await.unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidResponseData { .. }),
        "{error:?}"
    );
}

/// upstream: @ai-sdk/anthropic src/anthropic-evaluation.test.ts › evaluates Boolean alongside Choice and Score using %s
#[tokio::test]
async fn evaluates_boolean_alongside_choice_and_score_using_s() {
    for structured_output_mode in ["outputFormat", "jsonTool"] {
        let values = json!({ "q0": "c1", "q1": 1.25, "q2": 0.02 });
        let json_tool = structured_output_mode == "jsonTool";
        let (model, fetch) = setup(fixture_with(json!({
            "stop_reason": if json_tool { "tool_use" } else { "end_turn" },
            "content": if json_tool {
                json!([{ "type": "tool_use", "id": "tool-test", "name": "json", "input": values }])
            } else {
                json!([{ "type": "text", "text": values.to_string() }])
            },
        })));
        let result = model
            .do_decide(&DecisionCallOptions {
                provider_options: anthropic_options(
                    json!({ "structuredOutputMode": structured_output_mode }),
                ),
                ..options_with_questions(json!({
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
                    "flag": { "type": "boolean", "instructions": "Is a refund requested?" },
                }))
            })
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.answers).unwrap(),
            json!({
                "department": { "type": "choice", "choice": "billing" },
                "severity": { "type": "score", "score": 1.25 },
                "flag": { "type": "boolean", "probability": 0.02 },
            }),
            "{structured_output_mode}"
        );
        let calls = fetch.seen();
        assert_eq!(calls.len(), 1, "{structured_output_mode}");
        let body = body_of(&calls[0]);
        let schema = if json_tool {
            &body["tools"][0]["input_schema"]
        } else {
            &body["output_config"]["format"]["schema"]
        };
        assert_match_object(
            schema,
            &json!({
                "properties": { "q2": { "type": "number" } },
                "required": ["q0", "q1", "q2"],
            }),
            structured_output_mode,
        );
        let q2 = &schema["properties"]["q2"];
        assert!(
            q2.get("minimum").is_none(),
            "{structured_output_mode}: {q2}"
        );
        assert!(
            q2.get("maximum").is_none(),
            "{structured_output_mode}: {q2}"
        );
    }
}
