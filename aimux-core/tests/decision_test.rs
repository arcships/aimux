//! `decide` and the `DecisionModelV4` shapes, pinned to
//! `ai@7.0.128 src/decide/{decide,validate-decision}.ts` and
//! `@ai-sdk/provider@4.0.22 src/decision-model/v4`.
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::decision_model::*;
use aimux_core::error::ApiCallError;
use aimux_core::shared::ResponseInfo;
use aimux_core::{AbortSignal, AiMuxError};
use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::json;

/// The answer of each call, by zero-based call index.
type Answers = Box<dyn FnMut(usize) -> Result<DecisionResult, AiMuxError> + Send>;

/// A model that answers with what the test gives it.
struct TestModel {
    calls: AtomicUsize,
    pending: bool,
    supported: &'static [DecisionQuestionType],
    answers: Mutex<Answers>,
    seen: Mutex<Vec<DecisionCallOptions>>,
}

impl TestModel {
    fn new(
        answers: impl FnMut(usize) -> Result<DecisionResult, AiMuxError> + Send + 'static,
    ) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            pending: false,
            supported: DecisionQuestionType::ALL,
            answers: Mutex::new(Box::new(answers)),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn pending() -> Self {
        Self {
            pending: true,
            ..Self::new(|_| Ok(valid()))
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl DecisionModel for TestModel {
    fn provider(&self) -> &str {
        "test-provider"
    }
    fn model_id(&self) -> &str {
        "test-model"
    }
    fn supported_question_types(&self) -> &[DecisionQuestionType] {
        self.supported
    }
    async fn do_decide(&self, options: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        self.seen.lock().unwrap().push(options.clone());
        if self.pending {
            std::future::pending::<()>().await;
        }
        (self.answers.lock().unwrap())(call)
    }
}

fn questions() -> DecisionQuestions {
    serde_json::from_value(json!({
        "choice": {"type": "choice", "instructions": "choose", "criteria": {"yes": null, "no": "Not now"}},
        "score": {"type": "score", "instructions": "score", "criteria": ["low", "high"]},
        "boolean": {"type": "boolean", "instructions": "true?"}
    }))
    .unwrap()
}

fn options() -> DecideOptions {
    DecideOptions::new("text", questions())
}

fn answers(value: serde_json::Value) -> IndexMap<String, DecisionAnswer> {
    serde_json::from_value(value).unwrap()
}

fn result(answers: IndexMap<String, DecisionAnswer>) -> DecisionResult {
    DecisionResult {
        answers,
        rounding: None,
        usage: None,
        warnings: Vec::new(),
        provider_metadata: None,
        response: None,
    }
}

fn valid() -> DecisionResult {
    result(answers(json!({
        "choice": {"type": "choice", "choice": "yes"},
        "score": {"type": "score", "score": 0.6},
        "boolean": {"type": "boolean", "probability": 0.25}
    })))
}

fn retryable() -> AiMuxError {
    let mut error = ApiCallError::new("busy", "https://example.test/decide", json!({}));
    error.status_code = Some(503);
    error.is_retryable = true;
    AiMuxError::ApiCall(Box::new(error))
}

fn message(error: AiMuxError) -> String {
    match error {
        AiMuxError::InvalidArgument { message, .. }
        | AiMuxError::InvalidResponseData { message, .. }
        | AiMuxError::UnsupportedFunctionality(message) => message,
        other => panic!("unexpected error {other:?}"),
    }
}

/// `question.ts`: Choice criteria are an option map, Score criteria an
/// ordered list, Boolean criteria optional `true`/`false` descriptions; a
/// description is a string, object, array or null.
#[test]
fn questions_have_the_v4_shape() {
    let questions = questions();
    assert_eq!(
        questions.keys().collect::<Vec<_>>(),
        ["choice", "score", "boolean"]
    );
    let DecisionQuestion::Choice { criteria, .. } = &questions["choice"] else {
        panic!("expected choice");
    };
    assert_eq!(criteria["yes"], None);
    assert_eq!(criteria["no"], Some(DecisionInput::Text("Not now".into())));
    assert_eq!(
        questions["boolean"].question_type(),
        DecisionQuestionType::Boolean
    );
    let structured: DecisionQuestion = serde_json::from_value(json!({
        "type": "boolean",
        "instructions": {"question": "urgent?"},
        "criteria": {"true": ["blocked"], "false": null}
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&structured).unwrap(),
        json!({"type": "boolean", "instructions": {"question": "urgent?"}, "criteria": {"true": ["blocked"], "false": null}})
    );
    for description in [json!(true), json!(42)] {
        assert!(
            serde_json::from_value::<DecisionQuestion>(
                json!({"type": "choice", "instructions": "x", "criteria": {"a": description}})
            )
            .is_err()
        );
    }
    assert!(
        serde_json::from_value::<DecisionQuestion>(
            json!({"type": "choice", "instructions": 7, "criteria": {"a": null}})
        )
        .is_err()
    );
}

/// `decision-model-v4.ts:6-16`: the specification version is "v4".
#[test]
fn models_report_specification_version_v4() {
    assert_eq!(
        TestModel::new(|_| Ok(valid())).specification_version(),
        "v4"
    );
}

/// `validate-decision.ts:61-93`, run before any model call
/// (`decide.ts:77`).
#[tokio::test]
async fn invalid_input_is_rejected_before_the_model_is_called() {
    let model = TestModel::new(|_| Ok(valid()));
    let cases = [
        (
            json!({}),
            "Invalid argument for parameter questions: must be a nonempty question map",
        ),
        (
            json!({"team": {"type": "choice", "instructions": "x", "criteria": {}}}),
            "Invalid argument for parameter questions.team: choice criteria must be a nonempty option map",
        ),
        (
            json!({"level": {"type": "score", "instructions": "x", "criteria": ["only"]}}),
            "Invalid argument for parameter questions.level: score criteria must contain at least two ordered levels",
        ),
    ];
    for (questions, expected) in cases {
        let options = DecideOptions::new("text", serde_json::from_value(questions).unwrap());
        let error = decide(&model, options).await.unwrap_err();
        assert!(matches!(error, AiMuxError::InvalidArgument { .. }));
        assert_eq!(message(error), expected);
    }
    assert!(
        serde_json::from_value::<DecisionQuestion>(
            json!({"type": "boolean", "instructions": "x", "criteria": {"maybe": "?"}})
        )
        .is_err(),
        "validate-decision.ts:94-105: boolean criteria only describe true and false"
    );
    assert_eq!(model.calls(), 0);
}

/// `decide.ts:79-88` and
/// `@ai-sdk/provider@4.0.22 src/errors/decision-unsupported-question-type-error.ts:18-31`.
#[tokio::test]
async fn unsupported_question_types_fail_before_the_model_is_called() {
    let mut model = TestModel::new(|_| Ok(valid()));
    model.supported = &[DecisionQuestionType::Choice, DecisionQuestionType::Score];
    let error = decide(&model, options()).await.unwrap_err();
    assert!(matches!(error, AiMuxError::UnsupportedFunctionality(_)));
    assert_eq!(
        message(error),
        "Question \"boolean\" has type \"boolean\", which is not supported by provider \"test-provider\" and model \"test-model\"."
    );
    assert_eq!(model.calls(), 0);
}

/// `decide.ts:138-147`: the model receives the state, questions, headers
/// and provider options as given.
#[tokio::test]
async fn the_model_receives_the_call_options() {
    let model = TestModel::new(|_| Ok(valid()));
    let mut options = DecideOptions::new(
        DecisionInput::Array(vec![json!("a"), json!({"b": 1})]),
        questions(),
    );
    options.headers = Some([("x-test".to_string(), "1".to_string())].into());
    options.provider_options = Some(
        [(
            "test".to_string(),
            json!({"temperature": 0}).as_object().unwrap().clone(),
        )]
        .into(),
    );
    decide(&model, options.clone()).await.unwrap();
    let seen = model.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].state, options.state);
    assert_eq!(
        serde_json::to_value(&seen[0].questions).unwrap(),
        serde_json::to_value(&options.questions).unwrap()
    );
    assert_eq!(seen[0].headers, options.headers);
    assert_eq!(seen[0].provider_options, options.provider_options);
}

/// `decide.ts:91-94,138-147`: transient failures are retried, twice by
/// default.
#[tokio::test(start_paused = true)]
async fn transient_failures_are_retried_twice_by_default() {
    let model = TestModel::new(|_| Err(retryable()));
    let error = decide(&model, options()).await.unwrap_err();
    assert!(matches!(error, AiMuxError::Retry(_)));
    assert_eq!(model.calls(), 3);

    let model = TestModel::new(|call| {
        if call == 0 {
            Err(retryable())
        } else {
            Ok(valid())
        }
    });
    let mut options = options();
    options.max_retries = Some(1);
    decide(&model, options).await.unwrap();
    assert_eq!(model.calls(), 2);
}

/// `decide.ts:149-154`: answers are validated after the retries, so an
/// invalid answer is not retried.
#[tokio::test]
async fn answers_are_validated_once_after_the_retries() {
    let model = TestModel::new(|_| {
        Ok(result(answers(
            json!({"choice": {"type": "choice", "choice": "yes"}}),
        )))
    });
    let error = decide(&model, options()).await.unwrap_err();
    assert!(matches!(error, AiMuxError::InvalidResponseData { .. }));
    assert_eq!(
        message(error),
        "Decision must return exactly one answer for every question."
    );
    assert_eq!(model.calls(), 1);
}

/// `decide.ts:166-186`: usage totals only known counts; the response
/// timestamp defaults to now and its model id to the model's.
#[tokio::test]
async fn the_result_completes_usage_and_response() {
    let model = TestModel::new(|call| {
        let mut result = valid();
        result.warnings = vec![aimux_core::types::Warning::Other {
            message: "note".into(),
        }];
        result.rounding = Some(DecisionRounding {
            probability_decimals: Some(2),
            score_decimals: None,
        });
        result.provider_metadata = Some(
            [(
                "test".to_string(),
                json!({"a": 1}).as_object().unwrap().clone(),
            )]
            .into(),
        );
        if call == 0 {
            result.usage = Some(DecisionUsage {
                input_tokens: Some(10),
                output_tokens: Some(2),
            });
        } else {
            result.usage = Some(DecisionUsage {
                input_tokens: Some(10),
                output_tokens: None,
            });
            result.response = Some(ResponseInfo {
                id: Some("resp-1".into()),
                timestamp: Some("2026-01-01T00:00:00Z".parse().unwrap()),
                model_id: Some("served".into()),
                ..Default::default()
            });
        }
        Ok(result)
    });
    let first = decide(&model, options()).await.unwrap();
    assert_eq!(
        first.usage,
        DecideUsage {
            input_tokens: Some(10),
            output_tokens: Some(2),
            total_tokens: Some(12),
        }
    );
    assert!(first.response.timestamp.is_some());
    assert_eq!(first.response.model_id.as_deref(), Some("test-model"));
    assert_eq!(first.warnings.len(), 1);
    assert_eq!(first.rounding.unwrap().probability_decimals, Some(2));
    assert_eq!(first.provider_metadata.unwrap()["test"]["a"], 1);
    assert_eq!(
        first.answers.keys().collect::<Vec<_>>(),
        ["choice", "score", "boolean"]
    );

    let second = decide(&model, options()).await.unwrap();
    assert_eq!(second.usage.total_tokens, None);
    assert_eq!(second.response.id.as_deref(), Some("resp-1"));
    assert_eq!(
        second
            .response
            .timestamp
            .map(|time| time.to_string())
            .as_deref(),
        Some("2026-01-01T00:00:00.000Z")
    );
    assert_eq!(second.response.model_id.as_deref(), Some("served"));
    let value = serde_json::to_value(&second).unwrap();
    assert_eq!(
        value["answers"]["boolean"],
        json!({"type": "boolean", "probability": 0.25})
    );
}

fn check(
    questions: &DecisionQuestions,
    answers_json: serde_json::Value,
    rounding: Option<DecisionRounding>,
) -> Result<(), String> {
    validate_decision_answers(questions, &answers(answers_json), rounding).map_err(message)
}

/// `validate-decision.ts:197-295`, message by message.
#[test]
fn invalid_answers_report_the_upstream_messages() {
    let questions = questions();
    let base = json!({
        "choice": {"type": "choice", "choice": "yes", "probabilities": {"yes": 0.75, "no": 0.25}},
        "score": {"type": "score", "score": 0.6, "probabilities": {"0": 0.4, "1": 0.6}},
        "boolean": {"type": "boolean", "probability": 0.25}
    });
    check(&questions, base.clone(), None).unwrap();
    let cases: [(&str, serde_json::Value, &str); 10] = [
        (
            "extra",
            json!({"type": "boolean", "probability": 0.5}),
            "Decision must return exactly one answer for every question.",
        ),
        (
            "boolean",
            json!({"type": "choice", "choice": "yes"}),
            "Question \"boolean\" returned an answer with the wrong type.",
        ),
        (
            "choice",
            json!({"type": "choice", "choice": "maybe"}),
            "Question \"choice\" selected an unknown option.",
        ),
        (
            "choice",
            json!({"type": "choice", "choice": "yes", "probabilities": {"yes": 1.0}}),
            "Question \"choice\" must have a complete distribution of finite probabilities in [0, 1].",
        ),
        (
            "choice",
            json!({"type": "choice", "choice": "yes", "probabilities": {"yes": 1.5, "no": -0.5}}),
            "Question \"choice\" must have a complete distribution of finite probabilities in [0, 1].",
        ),
        (
            "choice",
            json!({"type": "choice", "choice": "yes", "probabilities": {"yes": 0.75, "no": 0.3}}),
            "Question \"choice\" probabilities must sum to 1 within the declared rounding precision.",
        ),
        (
            "choice",
            json!({"type": "choice", "choice": "no", "probabilities": {"yes": 0.75, "no": 0.25}}),
            "Question \"choice\" did not select a highest-probability option.",
        ),
        (
            "score",
            json!({"type": "score", "score": 1.5}),
            "Question \"score\" score must be in [0, 1].",
        ),
        (
            "score",
            json!({"type": "score", "score": 0.5, "probabilities": {"0": 0.4, "1": 0.6}}),
            "Question \"score\" score must equal the probability-weighted mean within the declared rounding precision.",
        ),
        (
            "boolean",
            json!({"type": "boolean", "probability": 1.25}),
            "Question \"boolean\" must return P(true) as a finite probability in [0, 1].",
        ),
    ];
    for (id, answer, expected) in cases {
        let mut answers = base.clone();
        answers[id] = answer;
        assert_eq!(check(&questions, answers, None).unwrap_err(), expected);
    }
    let mut missing = base.clone();
    missing.as_object_mut().unwrap().remove("score");
    assert_eq!(
        check(&questions, missing, None).unwrap_err(),
        "Decision must return exactly one answer for every question."
    );
    // validate-decision.ts:185-194
    for decimals in [16, 99] {
        assert_eq!(
            check(
                &questions,
                base.clone(),
                Some(DecisionRounding {
                    probability_decimals: Some(decimals),
                    score_decimals: None,
                })
            )
            .unwrap_err(),
            "Decision rounding decimals must be integers between 0 and 15."
        );
    }
}

/// A Score question with `levels` ordered levels.
fn score_questions(levels: usize) -> DecisionQuestions {
    let criteria: Vec<Option<DecisionInput>> = (0..levels)
        .map(|index| Some(format!("level-{index}").into()))
        .collect();
    IndexMap::from([(
        "score".to_string(),
        DecisionQuestion::Score {
            instructions: "score".into(),
            criteria,
        },
    )])
}

fn score_answer(score: f64, probabilities: &[f64]) -> serde_json::Value {
    let probabilities: serde_json::Map<String, serde_json::Value> = probabilities
        .iter()
        .enumerate()
        .map(|(index, p)| (index.to_string(), json!(p)))
        .collect();
    json!({"score": {"type": "score", "score": score, "probabilities": probabilities}})
}

/// `validate-decision.ts:9,168,272-278`: an absolute tolerance of 1e-6,
/// plus half a unit in the last declared place per rounded value.
#[test]
fn scores_must_agree_with_distributions_allowing_rounding() {
    let rounding = Some(DecisionRounding {
        probability_decimals: Some(2),
        score_decimals: Some(2),
    });
    for (probabilities, score, valid) in [
        (vec![1.0, 0.0], 1.0, false),
        (vec![0.0, 1.0], 0.0, false),
        (vec![1.0, 0.0], 0.0, true),
        (vec![0.3, 0.7], 0.8, false),
        (vec![0.33, 0.33, 0.33], 1.0, true),
        (vec![0.0, 0.34, 0.67], 1.67, true),
        (vec![1.0, 0.0], 0.011, false),
    ] {
        let questions = score_questions(probabilities.len());
        assert_eq!(
            check(&questions, score_answer(score, &probabilities), rounding).is_ok(),
            valid,
            "score={score}, probabilities={probabilities:?}"
        );
    }
}

/// `validate-decision.ts:185-196`: the probability and score precisions
/// are declared separately.
#[test]
fn precision_is_explicit_and_independent_for_probabilities_and_scores() {
    let questions = score_questions(2);
    let rounding = |probability_decimals, score_decimals| {
        Some(DecisionRounding {
            probability_decimals,
            score_decimals,
        })
    };
    let answer = score_answer(0.76, &[0.25, 0.75]);
    assert!(check(&questions, answer.clone(), None).is_err());
    assert!(check(&questions, answer.clone(), rounding(None, Some(1))).is_ok());
    assert!(check(&questions, answer, rounding(None, Some(2))).is_err());
    let answer = score_answer(0.754, &[0.25, 0.75]);
    assert!(check(&questions, answer.clone(), rounding(Some(2), None)).is_ok());
    assert!(check(&questions, answer.clone(), rounding(Some(4), None)).is_err());
    assert!(check(&questions, answer, rounding(Some(16), None)).is_err());
}

/// `validate-decision.ts:8-9`: provider output is never renormalized;
/// float32 softmax noise stays within the absolute tolerance.
#[test]
fn native_float32_softmax_noise_is_not_decimal_rounding() {
    let questions = score_questions(3);
    let probabilities = [f64::from(0.1_f32), f64::from(0.2_f32), f64::from(0.7_f32)];
    check(
        &questions,
        score_answer(f64::from(1.6_f32), &probabilities),
        None,
    )
    .unwrap();
    assert!(check(&questions, score_answer(1.601, &probabilities), None).is_err());
}

#[tokio::test]
async fn already_aborted_request_does_not_start_provider() {
    let model = TestModel::pending();
    let mut options = options();
    let signal = AbortSignal::new();
    signal.abort();
    options.abort_signal = Some(signal);
    assert!(matches!(
        decide(&model, options).await,
        Err(AiMuxError::Aborted(_))
    ));
    assert_eq!(model.calls(), 0);
}

#[tokio::test]
async fn abort_cancels_in_flight_decision() {
    let model = TestModel::pending();
    let signal = AbortSignal::new();
    let mut options = options();
    options.abort_signal = Some(signal.clone());
    let abort = async {
        tokio::task::yield_now().await;
        signal.abort();
    };
    let (result, ()) = tokio::join!(decide(&model, options), abort);
    assert!(matches!(result, Err(AiMuxError::Aborted(_))));
    assert_eq!(model.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn total_timeout_bounds_pending_attempt() {
    let model = TestModel::pending();
    let mut options = options();
    options.timeout = Some(aimux_core::options::TimeoutConfiguration {
        total_ms: Some(10),
        ..Default::default()
    });
    assert!(matches!(
        decide(&model, options).await,
        Err(AiMuxError::Timeout(_))
    ));
    assert_eq!(model.calls(), 1);
}
