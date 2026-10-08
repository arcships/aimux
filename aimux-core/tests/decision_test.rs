use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::decision_model::*;
use aimux_core::{AbortSignal, AiMuxError};
use async_trait::async_trait;
use serde_json::json;

struct TestModel {
    calls: AtomicUsize,
    pending: bool,
    distributions: bool,
}

#[async_trait]
impl DecisionModel for TestModel {
    fn provider(&self) -> &str {
        "test"
    }
    fn model_id(&self) -> &str {
        "test"
    }
    fn capabilities(&self) -> DecisionCapabilities {
        DecisionCapabilities {
            supports_images: false,
            max_images: None,
            supports_typed_choices: false,
            rounding: DecisionRounding::default(),
            probability_source: DecisionProbabilitySource::ModelEstimate,
            supports_boolean: true,
            supports_choice: true,
            supports_score: true,
            returns_distributions: self.distributions,
            max_questions: None,
            min_choices: None,
            max_choices: None,
            max_score_levels: None,
        }
    }
    async fn do_decide(&self, _: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.pending {
            std::future::pending::<()>().await;
        }
        Ok(DecisionResult {
            rounding: DecisionRounding::default(),
            answers: BTreeMap::from([
                (
                    "choice".into(),
                    DecisionAnswer::Choice {
                        value: None,
                        selected: "yes".into(),
                        probabilities: None,
                        confidence: None,
                    },
                ),
                (
                    "score".into(),
                    DecisionAnswer::Score {
                        expected_value: 0.6,
                        levels: vec!["low".into(), "high".into()],
                        probabilities: None,
                        confidence: None,
                    },
                ),
            ]),
            provider: "test".into(),
            model: "test".into(),
            model_version: None,
            probability_source: DecisionProbabilitySource::ModelEstimate,
            usage: None,
            latency_ms: None,
            provider_metadata: None,
            response: None,
        })
    }
}

fn options() -> DecisionCallOptions {
    serde_json::from_value(json!({"state": "text", "questions": [
        {"id":"choice", "type":"choice", "instructions":"choose", "options":[{"label":"yes"}]},
        {"id":"score", "type":"score", "instructions":"score", "levels":["low", "high"]}
    ]}))
    .unwrap()
}

#[tokio::test]
async fn optional_distributions_follow_declared_provider_capabilities() {
    let model = TestModel {
        calls: AtomicUsize::new(0),
        pending: false,
        distributions: false,
    };
    assert_eq!(decide(&model, options()).await.unwrap().answers.len(), 2);
    let native = TestModel {
        distributions: true,
        ..model
    };
    assert!(matches!(
        decide(&native, options()).await,
        Err(AiMuxError::InvalidResponseData(_))
    ));
}

#[tokio::test]
async fn already_aborted_request_does_not_start_provider() {
    let model = TestModel {
        calls: AtomicUsize::new(0),
        pending: true,
        distributions: false,
    };
    let mut options = options();
    let signal = AbortSignal::new();
    signal.abort();
    options.abort_signal = Some(signal);
    assert!(matches!(
        decide(&model, options).await,
        Err(AiMuxError::Aborted(_))
    ));
    assert_eq!(model.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn abort_cancels_in_flight_decision() {
    let model = TestModel {
        calls: AtomicUsize::new(0),
        pending: true,
        distributions: false,
    };
    let signal = AbortSignal::new();
    let mut options = options();
    options.abort_signal = Some(signal.clone());
    let abort = async {
        tokio::task::yield_now().await;
        signal.abort();
    };
    let (result, ()) = tokio::join!(decide(&model, options), abort);
    assert!(matches!(result, Err(AiMuxError::Aborted(_))));
    assert_eq!(model.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn total_timeout_bounds_pending_attempt() {
    let model = TestModel {
        calls: AtomicUsize::new(0),
        pending: true,
        distributions: false,
    };
    let mut options = options();
    options.timeout = Some(aimux_core::options::TimeoutConfiguration {
        total_ms: Some(10),
        ..Default::default()
    });
    assert!(matches!(
        decide(&model, options).await,
        Err(AiMuxError::Timeout(_))
    ));
    assert_eq!(model.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn scores_must_agree_with_distributions_allowing_rounding() {
    let model = TestModel {
        calls: AtomicUsize::new(0),
        pending: false,
        distributions: false,
    };
    for (probabilities, score, valid) in [
        (vec![1.0, 0.0], 1.0, false),
        (vec![0.0, 1.0], 0.0, false),
        (vec![1.0, 0.0], 0.0, true),
        (vec![0.3, 0.7], 0.8, false),
        (vec![0.33, 0.33, 0.33], 1.0, true),
        (vec![0.0, 0.34, 0.67], 1.67, true),
        (vec![1.0, 0.0], 0.011, false),
    ] {
        let mut request = options();
        let levels: Vec<DecisionDescription> = (0..probabilities.len())
            .map(|index| format!("level-{index}").into())
            .collect();
        if let DecisionQuestion::Score {
            levels: request_levels,
            ..
        } = &mut request.questions[1]
        {
            *request_levels = levels.clone();
        }
        let mut result = model.do_decide(&request).await.unwrap();
        result.rounding = DecisionRounding {
            probability_decimals: Some(2),
            score_decimals: Some(2),
        };
        result.answers.insert(
            "score".into(),
            DecisionAnswer::Score {
                expected_value: score,
                levels,
                probabilities: Some(probabilities.clone()),
                confidence: None,
            },
        );
        assert_eq!(
            result.validate(&request).is_ok(),
            valid,
            "score={score}, probabilities={probabilities:?}"
        );
    }
}

#[test]
fn probability_sources_parse_only_known_contract_values() {
    for source in ["native", "logit_scoring", "model_estimate"] {
        let parsed: DecisionProbabilitySource = source.parse().unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), json!(source));
    }
    assert!(matches!(
        "unknown".parse::<DecisionProbabilitySource>(),
        Err(AiMuxError::InvalidArgument(_))
    ));
}

#[tokio::test]
async fn precision_is_explicit_and_independent_for_probabilities_and_scores() {
    let model = TestModel {
        calls: AtomicUsize::new(0),
        pending: false,
        distributions: false,
    };
    let request = options();
    let mut result = model.do_decide(&request).await.unwrap();
    result.answers.insert(
        "score".into(),
        DecisionAnswer::Score {
            expected_value: 0.76,
            levels: vec!["low".into(), "high".into()],
            probabilities: Some(vec![0.25, 0.75]),
            confidence: None,
        },
    );
    assert!(result.validate(&request).is_err());
    result.rounding.score_decimals = Some(1);
    assert!(result.validate(&request).is_ok());
    result.rounding.score_decimals = Some(2);
    assert!(result.validate(&request).is_err());
    result.rounding = DecisionRounding {
        probability_decimals: Some(2),
        score_decimals: None,
    };
    if let DecisionAnswer::Score { expected_value, .. } = result.answers.get_mut("score").unwrap() {
        *expected_value = 0.754;
    }
    assert!(result.validate(&request).is_ok());
    result.rounding.probability_decimals = Some(4);
    assert!(result.validate(&request).is_err());
    result.rounding.probability_decimals = Some(16);
    assert!(result.validate(&request).is_err());
}

#[test]
fn structured_descriptions_reject_scalar_coercion_and_bad_levels() {
    for invalid in [json!(null), json!(true), json!(42)] {
        let mut request = serde_json::to_value(options()).unwrap();
        request["questions"][0]["instructions"] = invalid;
        assert!(serde_json::from_value::<DecisionCallOptions>(request).is_err());
    }
    for levels in [
        json!([{}, "high"]),
        json!([[], "high"]),
        json!([{"label":"low"}, {"label":"low"}]),
    ] {
        let mut request = serde_json::to_value(options()).unwrap();
        request["questions"][1]["levels"] = levels;
        let request: DecisionCallOptions = serde_json::from_value(request).unwrap();
        assert!(request.validate().is_err());
    }
}

#[tokio::test]
async fn native_float32_softmax_noise_is_not_decimal_rounding() {
    let model = TestModel {
        calls: AtomicUsize::new(0),
        pending: false,
        distributions: false,
    };
    let mut request = options();
    let levels: Vec<DecisionDescription> = vec!["low".into(), "middle".into(), "high".into()];
    if let DecisionQuestion::Score {
        levels: original, ..
    } = &mut request.questions[1]
    {
        *original = levels.clone();
    }
    let mut result = model.do_decide(&request).await.unwrap();
    let probabilities = vec![f64::from(0.1_f32), f64::from(0.2_f32), f64::from(0.7_f32)];
    result.answers.insert(
        "score".into(),
        DecisionAnswer::Score {
            expected_value: f64::from(1.6_f32),
            levels,
            probabilities: Some(probabilities.clone()),
            confidence: None,
        },
    );
    result.validate(&request).unwrap();
    assert_eq!(result.rounding, DecisionRounding::default());
    if let DecisionAnswer::Score {
        probabilities: Some(actual),
        ..
    } = &result.answers["score"]
    {
        assert_eq!(*actual, probabilities);
    }
    if let DecisionAnswer::Score { expected_value, .. } = result.answers.get_mut("score").unwrap() {
        *expected_value = 1.601;
    }
    assert!(result.validate(&request).is_err());
}
