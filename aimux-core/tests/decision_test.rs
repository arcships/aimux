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
            answers: BTreeMap::from([
                (
                    "choice".into(),
                    DecisionAnswer::Choice {
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
async fn optional_distributions_support_structured_generation() {
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
