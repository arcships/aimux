//! Port of `provider-utils/src/evaluation-language-model.test.ts`.
//!
//! Not ported: the WORKFLOW_SERIALIZE / WORKFLOW_DESERIALIZE hook case (a
//! TypeScript workflow runtime feature) and the `specificationVersion`
//! constructor check (there is one language-model trait version here).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{Value, json};

use aimux_core::AbortSignal;
use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::evaluation_model::{
    EvaluationAnswer, EvaluationCallOptions, EvaluationModel, EvaluationQuestion,
    EvaluationQuestionType, EvaluationResult,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, UserPart};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::shared::ResponseInfo;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, InputTokenUsage, OutputTokenUsage, Usage, Warning,
};
use aimux_provider_utils::EvaluationLanguageModel;

struct MockLanguageModel {
    result: Mutex<Result<GenerateResult, AiMuxError>>,
    calls: Mutex<Vec<CallOptions>>,
    /// Aborted from inside `do_generate`, as the TS test does.
    abort_during_call: Option<AbortSignal>,
}

#[async_trait]
impl LanguageModel for MockLanguageModel {
    fn provider(&self) -> &str {
        "test.language"
    }
    fn model_id(&self) -> &str {
        "test-model"
    }
    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        self.calls.lock().unwrap().push(options.clone());
        if let Some(signal) = &self.abort_during_call {
            signal.abort();
        }
        self.result.lock().unwrap().clone()
    }
    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        unreachable!("evaluation never streams")
    }
}

fn questions(value: Value) -> IndexMap<String, EvaluationQuestion> {
    serde_json::from_value(value).unwrap()
}

fn base_questions() -> Value {
    json!({
        "category": {
            "type": "choice",
            "instructions": { "task": ["Pick the exact label"] },
            "criteria": {
                "Needs Review": { "meaning": "manual" },
                "Needs review": ["automatic"],
                "other": null
            }
        },
        "severity": {
            "type": "score",
            "instructions": ["Rate the impact"],
            "criteria": ["low", { "meaning": "medium" }, null]
        }
    })
}

fn options(questions_value: Value) -> EvaluationCallOptions {
    EvaluationCallOptions {
        state: serde_json::from_value(json!({ "text": "test", "events": [1, null] })).unwrap(),
        questions: questions(questions_value),
        abort_signal: None,
        headers: None,
        provider_options: None,
    }
}

fn generate_result(text: &str) -> GenerateResult {
    GenerateResult {
        content: vec![GenerateContent::Text {
            text: text.to_string(),
            provider_metadata: None,
        }],
        finish_reason: FinishReason {
            unified: FinishReasonUnified::Stop,
            raw: Some("end".into()),
        },
        usage: Usage {
            input_tokens: InputTokenUsage {
                total: Some(20),
                no_cache: Some(15),
                cache_read: Some(5),
                cache_write: Some(0),
            },
            output_tokens: OutputTokenUsage {
                total: Some(12),
                text: Some(8),
                reasoning: Some(4),
            },
            raw: None,
        },
        warnings: vec![Warning::Other {
            message: "test warning".into(),
        }],
        provider_metadata: serde_json::from_value(json!({ "test": { "original": true } })).ok(),
        request: None,
        response: Some(ResponseInfo {
            id: Some("response".into()),
            timestamp: Some("1970-01-01T00:00:00.000Z".into()),
            model_id: Some("resolved".into()),
            headers: Some([("x-request-id".to_string(), "id".to_string())].into()),
            body: Some(json!({ "raw": true })),
        }),
    }
}

fn setup_with(
    result: Result<GenerateResult, AiMuxError>,
    abort_during_call: Option<AbortSignal>,
) -> (EvaluationLanguageModel, Arc<MockLanguageModel>) {
    let language_model = Arc::new(MockLanguageModel {
        result: Mutex::new(result),
        calls: Mutex::default(),
        abort_during_call,
    });
    let model =
        EvaluationLanguageModel::new(language_model.clone(), Some("test.evaluation".into()));
    (model, language_model)
}

fn setup(text: &str) -> (EvaluationLanguageModel, Arc<MockLanguageModel>) {
    setup_with(Ok(generate_result(text)), None)
}

const DEFAULT_TEXT: &str = r#"{"q1":1.25,"q0":"c1"}"#;

fn user_json(call: &CallOptions) -> Value {
    let LanguageModelMessage::User { content, .. } = &call.prompt[1] else {
        panic!("expected a user message");
    };
    let UserPart::Text(part) = &content[0] else {
        panic!("expected text");
    };
    serde_json::from_str(&part.text).unwrap()
}

fn schema(call: &CallOptions) -> Value {
    match &call.response_format {
        Some(ResponseFormat::Json { schema, .. }) => schema.clone().unwrap(),
        other => panic!("expected json response format, got {other:?}"),
    }
}

fn mixed(extra: Value) -> EvaluationCallOptions {
    let mut all = base_questions();
    all.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    options(all)
}

fn flag_boolean() -> Value {
    json!({ "flag": { "type": "boolean", "instructions": "Yes?" } })
}

/// TS: maps exact caller labels and fractional scores without inventing distributions
#[tokio::test]
async fn maps_exact_labels_and_fractional_scores() {
    let (model, mock) = setup(DEFAULT_TEXT);
    let result = model.do_evaluate(&options(base_questions())).await.unwrap();
    assert_eq!(
        result.answers["category"],
        EvaluationAnswer::Choice {
            choice: "Needs review".into(),
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
    assert_eq!(mock.calls.lock().unwrap().len(), 1);
}

/// TS: uses a portable flat schema with required fields and internal option codes
#[tokio::test]
async fn uses_flat_schema_with_internal_option_codes() {
    let (model, mock) = setup(DEFAULT_TEXT);
    let opts = options(base_questions());
    model.do_evaluate(&opts).await.unwrap();
    let calls = mock.calls.lock().unwrap();
    let call = &calls[0];
    let Some(ResponseFormat::Json { name, .. }) = &call.response_format else {
        panic!("expected json");
    };
    assert_eq!(name.as_deref(), Some("evaluation"));
    let schema = schema(call);
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["properties"]["q0"],
        json!({ "type": "string", "enum": ["c0", "c1", "c2"] })
    );
    assert_eq!(schema["properties"]["q1"]["type"], "number");
    assert!(
        schema["properties"]["q1"]["description"]
            .as_str()
            .unwrap()
            .contains("0 to 2")
    );
    assert_eq!(schema["required"], json!(["q0", "q1"]));
    assert_eq!(schema["additionalProperties"], false);
    let base = base_questions();
    assert_eq!(
        user_json(call),
        json!({
            "state": { "text": "test", "events": [1, null] },
            "questions": {
                "q0": {
                    "id": "category",
                    "type": "choice",
                    "instructions": base["category"]["instructions"],
                    "criteria": {
                        "c0": { "label": "Needs Review", "description": { "meaning": "manual" } },
                        "c1": { "label": "Needs review", "description": ["automatic"] },
                        "c2": { "label": "other", "description": null }
                    }
                },
                "q1": {
                    "id": "severity",
                    "type": "score",
                    "instructions": base["severity"]["instructions"],
                    "criteria": base["severity"]["criteria"]
                }
            }
        })
    );
    assert!(call.temperature.is_none() && call.tools.is_none());
    assert_eq!(
        call.reasoning,
        Some(aimux_core::types::ReasoningEffort::None)
    );
}

/// TS: preserves usage, response, warnings, and provider metadata
#[tokio::test]
async fn preserves_usage_response_warnings_and_metadata() {
    let (model, _) = setup(DEFAULT_TEXT);
    let output = model.do_evaluate(&options(base_questions())).await.unwrap();
    let expected = generate_result(DEFAULT_TEXT);
    let usage = output.usage.unwrap();
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        (Some(20), Some(12))
    );
    assert_eq!(output.warnings.len(), 1);
    assert_eq!(
        serde_json::to_value(&output.response).unwrap(),
        serde_json::to_value(&expected.response).unwrap()
    );
    assert_eq!(output.provider_metadata, expected.provider_metadata);
}

/// TS: forwards cancellation, headers, and provider options unchanged
#[tokio::test]
async fn forwards_cancellation_headers_and_provider_options() {
    let (model, mock) = setup(DEFAULT_TEXT);
    let signal = AbortSignal::new();
    let mut opts = options(base_questions());
    opts.abort_signal = Some(signal.clone());
    opts.headers = Some([("test".to_string(), "header".to_string())].into());
    opts.provider_options = serde_json::from_value(json!({ "test": { "reasoning": "low" } })).ok();
    model.do_evaluate(&opts).await.unwrap();
    let calls = mock.calls.lock().unwrap();
    assert!(calls[0].abort_signal.is_some());
    assert_eq!(calls[0].headers, opts.headers);
    assert_eq!(calls[0].provider_options, opts.provider_options);
    assert_eq!(
        calls[0].reasoning,
        Some(aimux_core::types::ReasoningEffort::None)
    );
}

/// TS: preserves Boolean P(true) %s without thresholding in a mixed evaluation
#[tokio::test]
async fn preserves_boolean_probability_in_mixed_evaluation() {
    for probability in [0.0, 0.02, 0.5, 0.98, 1.0] {
        let text = json!({ "q2": probability, "q1": 1.25, "q0": "c1" }).to_string();
        let (model, mock) = setup(&text);
        let flag = json!({
            "type": "boolean",
            "instructions": { "task": ["Is a refund requested?"] },
            "criteria": {
                "true": { "meaning": "A refund is requested" },
                "false": ["No refund requested"]
            }
        });
        let result: EvaluationResult = model
            .do_evaluate(&mixed(json!({ "flag": flag })))
            .await
            .unwrap();
        assert_eq!(
            model.supported_question_types(),
            [
                EvaluationQuestionType::Choice,
                EvaluationQuestionType::Score,
                EvaluationQuestionType::Boolean
            ]
        );
        assert_eq!(
            result.answers["flag"],
            EvaluationAnswer::Boolean { probability }
        );
        assert_eq!(
            result.answers.keys().collect::<Vec<_>>(),
            ["category", "severity", "flag"]
        );
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let schema = schema(&calls[0]);
        let q2 = &schema["properties"]["q2"];
        assert_eq!(q2["type"], "number");
        assert!(q2["description"].as_str().unwrap().contains("probability"));
        assert!(q2.get("minimum").is_none() && q2.get("maximum").is_none());
        assert_eq!(schema["required"], json!(["q0", "q1", "q2"]));
        let mut expected = json!({ "id": "flag" });
        expected
            .as_object_mut()
            .unwrap()
            .extend(flag.as_object().unwrap().clone());
        assert_eq!(user_json(&calls[0])["questions"]["q2"], expected);
        let LanguageModelMessage::System { content, .. } = &calls[0].prompt[0] else {
            panic!("expected system");
        };
        assert!(content.contains("not confidence in whichever outcome"));
    }
}

/// TS: supports Boolean questions without criteria
#[tokio::test]
async fn supports_boolean_without_criteria() {
    let (model, _) = setup(r#"{"q0":0.25}"#);
    let opts = EvaluationCallOptions {
        state: serde_json::from_value(json!("test")).unwrap(),
        questions: questions(flag_boolean()),
        ..options(json!({}))
    };
    let result = model.do_evaluate(&opts).await.unwrap();
    assert_eq!(
        result.answers["flag"],
        EvaluationAnswer::Boolean { probability: 0.25 }
    );
}

fn assert_invalid_response(error: &AiMuxError) {
    assert!(
        matches!(error, AiMuxError::InvalidResponseData(_)),
        "got {error:?}"
    );
}

/// TS: rejects invalid Boolean probability %s without returning partial answers
#[tokio::test]
async fn rejects_invalid_boolean_probability() {
    for value in [
        "-0.01", "1.01", "1e999", "-1e999", "null", "true", "false", "\"0.5\"", "{}", "[]",
    ] {
        let (model, _) = setup(&format!(r#"{{"q0":"c1","q1":1.25,"q2":{value}}}"#));
        let error = model.do_evaluate(&mixed(flag_boolean())).await.unwrap_err();
        assert_invalid_response(&error);
    }
}

/// TS: rejects missing or extra answers for mixed Boolean evaluations: %s
#[tokio::test]
async fn rejects_missing_or_extra_answers_for_mixed_boolean() {
    for text in [
        r#"{"q0":"c1","q1":1.25}"#,
        r#"{"q0":"c1","q1":1.25,"q2":0.5,"extra":0.5}"#,
    ] {
        let (model, _) = setup(text);
        let error = model.do_evaluate(&mixed(flag_boolean())).await.unwrap_err();
        assert_invalid_response(&error);
    }
}

/// TS: rejects %s even with valid JSON
#[tokio::test]
async fn rejects_unfinished_generations_even_with_valid_json() {
    for unified in [
        FinishReasonUnified::Length,
        FinishReasonUnified::ContentFilter,
        FinishReasonUnified::ToolCalls,
        FinishReasonUnified::Error,
        FinishReasonUnified::Other,
    ] {
        let mut result = generate_result(DEFAULT_TEXT);
        result.finish_reason = FinishReason {
            unified,
            raw: Some("raw".into()),
        };
        let (model, _) = setup_with(Ok(result), None);
        let error = model
            .do_evaluate(&options(base_questions()))
            .await
            .unwrap_err();
        assert_invalid_response(&error);
    }
}

/// TS: rejects malformed evaluation output %s
#[tokio::test]
async fn rejects_malformed_evaluation_output() {
    for text in [
        "",
        "not JSON",
        "```json\n{}\n```",
        "null",
        "[]",
        "{}",
        r#"{"q0":"c1"}"#,
        r#"{"q0":"c1","q1":1,"extra":true}"#,
        r#"{"q0":"C1","q1":1}"#,
        r#"{"q0":"c01","q1":1}"#,
        r#"{"q0":"c3","q1":1}"#,
        r#"{"q0":"Needs review","q1":1}"#,
        r#"{"q0":1,"q1":1}"#,
        r#"{"q0":"c1","q1":"1"}"#,
        r#"{"q0":"c1","q1":-0.01}"#,
        r#"{"q0":"c1","q1":2.01}"#,
        r#"{"q0":"c1","q1":1e999}"#,
        r#"{"q0":"c1","q1":null}"#,
        r#"{"q0":"c1","q1":1,"__proto__":{}}"#,
    ] {
        let (model, _) = setup(text);
        let error = model
            .do_evaluate(&options(base_questions()))
            .await
            .unwrap_err();
        assert_invalid_response(&error);
    }
}

/// TS: joins text parts and ignores reasoning
#[tokio::test]
async fn joins_text_parts_and_ignores_reasoning() {
    let mut result = generate_result("");
    let text = |t: &str| GenerateContent::Text {
        text: t.to_string(),
        provider_metadata: None,
    };
    result.content = vec![
        GenerateContent::Reasoning(aimux_core::result::ReasoningOutput {
            text: "internal".into(),
            provider_metadata: None,
        }),
        text(r#"{"q0":"c1","#),
        text(r#""q1":1}"#),
    ];
    let (model, _) = setup_with(Ok(result), None);
    let output = model.do_evaluate(&options(base_questions())).await.unwrap();
    assert_eq!(
        output.answers["severity"],
        EvaluationAnswer::Score {
            score: 1.0,
            probabilities: None
        }
    );
}

/// TS: preserves unusual IDs without using them as schema property names
#[tokio::test]
async fn preserves_unusual_ids() {
    let (model, _) = setup(DEFAULT_TEXT);
    let base = base_questions();
    let weird = json!({ "__proto__": base["category"], "a.b [✓]": base["severity"] });
    let output = model.do_evaluate(&options(weird)).await.unwrap();
    assert_eq!(
        output.answers.keys().collect::<Vec<_>>(),
        ["__proto__", "a.b [✓]"]
    );
}

/// TS: does not retry or wrap language-model errors
#[tokio::test]
async fn does_not_retry_or_wrap_language_model_errors() {
    let error = AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code: Some(429),
        is_retryable: true,
        ..ApiCallError::new("retry later", "https://example.com", json!({}))
    }));
    let (model, mock) = setup_with(Err(error), None);
    let got = model
        .do_evaluate(&options(base_questions()))
        .await
        .unwrap_err();
    assert!(matches!(&got, AiMuxError::ApiCall(d) if d.message == "retry later"));
    assert_eq!(mock.calls.lock().unwrap().len(), 1);
}

/// TS: rejects pre-aborted requests without invoking the model
#[tokio::test]
async fn rejects_pre_aborted_requests() {
    let (model, mock) = setup(DEFAULT_TEXT);
    let signal = AbortSignal::new();
    signal.abort();
    let mut opts = options(base_questions());
    opts.abort_signal = Some(signal);
    let error = model.do_evaluate(&opts).await.unwrap_err();
    assert!(matches!(error, AiMuxError::Aborted(_)));
    assert!(mock.calls.lock().unwrap().is_empty());
}

/// TS: checks cancellation after the model returns
#[tokio::test]
async fn checks_cancellation_after_the_model_returns() {
    let signal = AbortSignal::new();
    let (model, _) = setup_with(Ok(generate_result(DEFAULT_TEXT)), Some(signal.clone()));
    let mut opts = options(base_questions());
    opts.abort_signal = Some(signal);
    let error = model.do_evaluate(&opts).await.unwrap_err();
    assert!(matches!(error, AiMuxError::Aborted(_)));
}

/// TS: rejects invalid rubric shapes before model I/O
#[tokio::test]
async fn rejects_invalid_rubric_shapes_before_model_io() {
    let base = base_questions();
    for invalid in [
        json!({}),
        json!({ "category": { "type": "choice", "instructions": base["category"]["instructions"], "criteria": {} } }),
        json!({ "severity": { "type": "score", "instructions": base["severity"]["instructions"], "criteria": ["only"] } }),
    ] {
        let (model, mock) = setup(DEFAULT_TEXT);
        let opts = EvaluationCallOptions {
            state: serde_json::from_value(json!("text")).unwrap(),
            questions: questions(invalid),
            ..options(json!({}))
        };
        let error = model.do_evaluate(&opts).await.unwrap_err();
        assert!(
            matches!(error, AiMuxError::InvalidArgument(_)),
            "got {error:?}"
        );
        assert!(mock.calls.lock().unwrap().is_empty());
    }
}
