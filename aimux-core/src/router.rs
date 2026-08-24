//! `RouterModel` — a composite model that routes each call to one child and
//! optionally falls back (RFC-0021).
//!
//! `RouterModel` implements [`LanguageModel`], so `generate_text` /
//! `stream_text` and every binding use it unchanged. Routing decisions live in
//! the pluggable [`Router`] trait (pure decision: prompt + child list → index);
//! execution + fallback live here. Built-in strategies: [`RuleRouter`],
//! [`WeightedRouter`].

use async_trait::async_trait;

use crate::composite::ChildModel;
use crate::error::AiMuxError;
use crate::language_model::LanguageModel;
use crate::language_model_message::LanguageModelPrompt;
use crate::options::CallOptions;
use crate::result::{GenerateResult, StreamResult};
use crate::retry;

// ─────────────────────────────────────────────────────────────────────────────
// Router trait
// ─────────────────────────────────────────────────────────────────────────────

/// Routing strategy: a pure decision over the prompt + child list. It does NOT
/// execute the call — [`RouterModel`] owns execution and fallback.
///
/// Built-in implementations: [`RuleRouter`], [`WeightedRouter`]. Users can
/// implement this to inject learned classifiers (e.g. RouteLLM via `ort`) —
/// learned routing is intentionally out of core (see RFC-0021 §6.1).
pub trait Router: Send + Sync {
    /// Choose a child-model index. `Err` means "no child can serve this prompt".
    ///
    /// # Errors
    ///
    /// `Err` means no child model can serve this prompt (all children filtered
    /// out).
    fn route(
        &self,
        prompt: &LanguageModelPrompt,
        models: &[ChildModel],
    ) -> Result<usize, AiMuxError>;
}

/// Static-priority router: always pick child 0 (the primary); fallback walks the
/// rest in array order. Equivalent to "primary + backups".
pub struct RuleRouter;

impl Router for RuleRouter {
    fn route(
        &self,
        _prompt: &LanguageModelPrompt,
        models: &[ChildModel],
    ) -> Result<usize, AiMuxError> {
        if models.is_empty() {
            return Err(AiMuxError::Other("router: no models configured".into()));
        }
        Ok(0)
    }
}

/// Weighted router: pick the child with the highest weight. On ties the
/// **earliest** index wins (so all-equal weights behave like `RuleRouter` —
/// always child 0). Missing trailing weights default to `0.0`. NaN at index > 0
/// loses to any finite weight (NaN at index 0 wins only because nothing
/// compares greater than NaN — avoid NaN weights). To route by cost
/// (lowest-cost first), pass reciprocals or negative weights.
pub struct WeightedRouter {
    weights: Vec<f64>,
}

impl WeightedRouter {
    /// Weights are positional — `weights[i]` applies to `models[i]`. Missing
    /// trailing weights default to `0.0`.
    #[must_use]
    pub fn new(weights: Vec<f64>) -> Self {
        Self { weights }
    }
}

impl Router for WeightedRouter {
    fn route(
        &self,
        _prompt: &LanguageModelPrompt,
        models: &[ChildModel],
    ) -> Result<usize, AiMuxError> {
        if models.is_empty() {
            return Err(AiMuxError::Other("router: no models configured".into()));
        }
        // Pick the child with the highest weight. On ties, prefer the earliest
        // index (matches `RuleRouter`'s "child 0 first" expectation when all
        // weights are equal). NaN weights lose to any finite weight.
        let mut best_idx = 0;
        let mut best_weight = *self.weights.first().unwrap_or(&0.0);
        for i in 1..models.len() {
            let w = *self.weights.get(i).unwrap_or(&0.0);
            if w > best_weight {
                best_idx = i;
                best_weight = w;
            }
        }
        Ok(best_idx)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RouterModel
// ─────────────────────────────────────────────────────────────────────────────

/// When a routed call fails, should we try the next child?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FallbackPolicy {
    /// On error, try the remaining children in array order (default).
    #[default]
    OnError,
    /// The chosen child's failure is final.
    None,
}

#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// `provider()` value (default `"router"`).
    pub provider_name: String,
    /// `model_id()` value (default `"router"`).
    pub model_id: String,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            provider_name: "router".into(),
            model_id: "router".into(),
        }
    }
}

/// A composite model that routes each call to one child and (optionally) falls
/// back across the rest on error.
///
/// Streaming retries setup on the routed child, but does not fall back or retry
/// after setup: once stream parts are visible to the user, either would risk
/// duplicating tokens. See RFC-0021 §3.3.
pub struct RouterModel {
    models: Vec<ChildModel>,
    router: Box<dyn Router>,
    fallback: FallbackPolicy,
    config: RouterConfig,
}

impl RouterModel {
    #[must_use]
    pub fn new(
        models: Vec<ChildModel>,
        router: Box<dyn Router>,
        fallback: FallbackPolicy,
        config: RouterConfig,
    ) -> Self {
        Self {
            models,
            router,
            fallback,
            config,
        }
    }

    /// Try every child except `exclude` in array order; return the first `Ok`,
    /// else the last error. `primary_err` seeds the error returned when there
    /// are no fallback candidates (e.g. a single-child router) so the real
    /// failure is never lost to a generic "all models failed".
    async fn fallback_generate(
        &self,
        exclude: usize,
        options: &CallOptions,
        primary_err: AiMuxError,
    ) -> Result<GenerateResult, AiMuxError> {
        let mut last_err = Some(primary_err);
        for (i, m) in self.models.iter().enumerate() {
            if i == exclude {
                continue;
            }
            let retries = retry::prepare_retries(
                options.max_retries,
                m.retry_config(),
                options.abort_signal.clone(),
            );
            let child_options = options.for_step(self.step_label(i));
            match retries
                .retry(|| {
                    if let Some(context) = &child_options.recording_context {
                        let _ = context.start_attempt();
                    }
                    m.do_generate(&child_options)
                })
                .await
            {
                Ok(r) => return Ok(r),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.expect("seeded primary_err makes last_err always Some"))
    }

    fn step_label(&self, idx: usize) -> String {
        let m = &self.models[idx];
        format!("router[{idx}]:{}/{}", m.provider(), m.model_id())
    }

    /// Validate a `Router`-returned index before indexing `self.models`. A
    /// buggy/hostile `Router` (user-implementable trait) must not panic the
    /// process — surface it as `InvalidArgument` instead.
    fn check_index(&self, idx: usize, from: &str) -> Result<usize, AiMuxError> {
        if idx < self.models.len() {
            Ok(idx)
        } else {
            Err(AiMuxError::InvalidArgument(format!(
                "{from}: router returned out-of-bounds index {idx} (models: {})",
                self.models.len()
            )))
        }
    }
}

#[async_trait]
impl LanguageModel for RouterModel {
    fn provider(&self) -> &str {
        &self.config.provider_name
    }

    fn model_id(&self) -> &str {
        &self.config.model_id
    }

    fn retry_config(&self) -> retry::RetryConfig {
        // Retrying the composite would rerun routing and previously attempted
        // children; each child is retried at its own execution boundary.
        retry::RetryConfig {
            max_retries: 0,
            ..retry::RetryConfig::default()
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let raw = self.router.route(&options.prompt, &self.models)?;
        let idx = self.check_index(raw, "router")?;
        let model = &self.models[idx];
        let retries = retry::prepare_retries(
            options.max_retries,
            model.retry_config(),
            options.abort_signal.clone(),
        );
        let child_options = options.for_step(self.step_label(idx));
        match retries
            .retry(|| {
                if let Some(context) = &child_options.recording_context {
                    let _ = context.start_attempt();
                }
                model.do_generate(&child_options)
            })
            .await
        {
            Ok(r) => Ok(r),
            Err(e) => {
                if self.fallback == FallbackPolicy::OnError {
                    self.fallback_generate(idx, options, e).await
                } else {
                    Err(e)
                }
            }
        }
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let raw = self.router.route(&options.prompt, &self.models)?;
        let idx = self.check_index(raw, "router")?;
        let model = &self.models[idx];
        let retries = retry::prepare_retries(
            options.max_retries,
            model.retry_config(),
            options.abort_signal.clone(),
        );
        let child_options = options.for_step(self.step_label(idx));
        // Only setup can be retried. The returned stream is passed through, so
        // failures after setup never invoke another child or duplicate output.
        retries
            .retry(|| {
                if let Some(context) = &child_options.recording_context {
                    let _ = context.start_attempt();
                }
                model.do_stream(&child_options)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{
        HttpExchange, OutcomeRecord, ProviderRecord, Recorder, RecordingContext, ResponseRecord,
    };
    use crate::result::GenerateResult;
    use crate::stream_part::StreamPart;
    use crate::types::{FinishReason, FinishReasonUnified, Usage};
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A mock child that either succeeds with `text` or always fails.
    struct MockChild {
        name: &'static str,
        text: String,
        fail: bool,
        retry_failures: usize,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LanguageModel for MockChild {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            self.name
        }
        async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail {
                return Err(AiMuxError::Other(format!("{} always fails", self.name)));
            }
            if attempt <= self.retry_failures {
                return Err(retryable_error(self.name));
            }
            Ok(GenerateResult {
                content: vec![crate::result::GenerateContent::Text {
                    text: self.text.clone(),
                    provider_metadata: None,
                }],
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: Usage::default(),
                warnings: vec![],
                provider_metadata: None,
                response: Default::default(),
                request_body: None,
                response_headers: None,
            })
        }
        async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail {
                return Err(AiMuxError::Other(format!("{} always fails", self.name)));
            }
            if attempt <= self.retry_failures {
                return Err(retryable_error(self.name));
            }
            let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
                Ok(StreamPart::StreamStart { warnings: vec![] }),
                Ok(StreamPart::TextDelta {
                    id: "t1".into(),
                    delta: self.text.clone(),
                    provider_metadata: None,
                }),
                Ok(StreamPart::Finish {
                    finish_reason: FinishReason {
                        unified: FinishReasonUnified::Stop,
                        raw: None,
                    },
                    usage: Usage::default(),
                    provider_metadata: None,
                }),
            ];
            Ok(StreamResult {
                stream: Box::pin(stream::iter(parts)),
                request_body: None,
                response_headers: None,
            })
        }
    }

    fn retryable_error(name: &str) -> AiMuxError {
        AiMuxError::ApiCall(Box::new(crate::ApiCallError {
            status_code: Some(503),
            response_headers: Some(std::collections::HashMap::from([(
                "retry-after-ms".into(),
                "0".into(),
            )])),
            is_retryable: true,
            ..crate::ApiCallError::new(
                format!("{name} retryable failure"),
                "https://example.test",
                serde_json::json!({}),
            )
        }))
    }

    fn retry_child(
        name: &'static str,
        text: &str,
        retry_failures: usize,
    ) -> (ChildModel, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(MockChild {
                name,
                text: text.into(),
                fail: false,
                retry_failures,
                calls: calls.clone(),
            }),
            calls,
        )
    }

    struct CountingRuleRouter(Arc<AtomicUsize>);

    impl Router for CountingRuleRouter {
        fn route(
            &self,
            prompt: &LanguageModelPrompt,
            models: &[ChildModel],
        ) -> Result<usize, AiMuxError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            RuleRouter.route(prompt, models)
        }
    }

    fn child(name: &'static str, text: &str, fail: bool) -> ChildModel {
        Arc::new(MockChild {
            name,
            text: text.into(),
            fail,
            retry_failures: 0,
            calls: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn opts() -> CallOptions {
        CallOptions {
            prompt: vec![],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn routes_to_first_with_rule_router() {
        let router = RouterModel::new(
            vec![child("a", "from-a", false), child("b", "from-b", false)],
            Box::new(RuleRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let r = router.do_generate(&opts()).await.unwrap();
        let text = crate::composite::extract_text(&r.content);
        assert_eq!(text, "from-a");
    }

    #[tokio::test]
    async fn weighted_router_picks_max() {
        let router = RouterModel::new(
            vec![child("a", "from-a", false), child("b", "from-b", false)],
            Box::new(WeightedRouter::new(vec![1.0, 5.0])),
            FallbackPolicy::None,
            RouterConfig::default(),
        );
        let r = router.do_generate(&opts()).await.unwrap();
        assert_eq!(crate::composite::extract_text(&r.content), "from-b");
    }

    #[tokio::test]
    async fn fallback_on_error_walks_remaining() {
        // Primary fails, second fails, third succeeds.
        let router = RouterModel::new(
            vec![
                child("a", "from-a", true),
                child("b", "from-b", true),
                child("c", "from-c", false),
            ],
            Box::new(RuleRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let r = router.do_generate(&opts()).await.unwrap();
        assert_eq!(crate::composite::extract_text(&r.content), "from-c");
    }

    #[tokio::test]
    async fn fallback_none_propagates_error() {
        let router = RouterModel::new(
            vec![child("a", "from-a", true), child("b", "from-b", false)],
            Box::new(RuleRouter),
            FallbackPolicy::None,
            RouterConfig::default(),
        );
        let err = router.do_generate(&opts()).await.unwrap_err();
        assert!(err.to_string().contains("always fails"));
    }

    #[tokio::test]
    async fn all_fail_returns_last_error() {
        let router = RouterModel::new(
            vec![child("a", "from-a", true), child("b", "from-b", true)],
            Box::new(RuleRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let err = router.do_generate(&opts()).await.unwrap_err();
        assert!(err.to_string().contains("always fails"));
    }

    #[tokio::test]
    async fn stream_routes_without_fallback() {
        // Primary is selected by RuleRouter; even though it would fail, stream
        // does not fall back. Use a healthy primary to confirm the happy path.
        let router = RouterModel::new(
            vec![child("a", "from-a", false), child("b", "from-b", false)],
            Box::new(RuleRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let result = router.do_stream(&opts()).await.unwrap();
        use futures::StreamExt;
        let mut s = result.stream;
        let mut text = String::new();
        while let Some(part) = s.next().await {
            if let Ok(StreamPart::TextDelta { delta, .. }) = part {
                text.push_str(&delta);
            }
        }
        assert_eq!(text, "from-a");
    }

    #[tokio::test]
    async fn single_child_failure_preserves_real_error() {
        // B1 regression guard: a single failing child must surface its real
        // error, not a generic "all models failed".
        let router = RouterModel::new(
            vec![child("solo", "solo-out", true)],
            Box::new(RuleRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let err = router.do_generate(&opts()).await.unwrap_err();
        assert!(
            err.to_string().contains("solo always fails"),
            "expected the real child error, got: {err}"
        );
    }

    #[tokio::test]
    async fn single_child_success_works() {
        let router = RouterModel::new(
            vec![child("solo", "solo-out", false)],
            Box::new(RuleRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let r = router.do_generate(&opts()).await.unwrap();
        assert_eq!(crate::composite::extract_text(&r.content), "solo-out");
    }

    #[tokio::test]
    async fn out_of_bounds_router_index_is_invalid_argument() {
        // A buggy/hostile Router returning an OOB index must not panic; it
        // surfaces as InvalidArgument.
        struct OobRouter;
        impl Router for OobRouter {
            fn route(
                &self,
                _prompt: &LanguageModelPrompt,
                _models: &[ChildModel],
            ) -> Result<usize, AiMuxError> {
                Ok(99)
            }
        }
        let router = RouterModel::new(
            vec![child("a", "from-a", false)],
            Box::new(OobRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let err = router.do_generate(&opts()).await.unwrap_err();
        match err {
            AiMuxError::InvalidArgument(msg) => {
                assert!(msg.contains("out-of-bounds"), "got: {msg}");
            }
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn weighted_router_ties_pick_earliest() {
        // S2 regression guard: equal weights resolve to the lowest index.
        let router = RouterModel::new(
            vec![child("a", "from-a", false), child("b", "from-b", false)],
            Box::new(WeightedRouter::new(vec![1.0, 1.0])),
            FallbackPolicy::None,
            RouterConfig::default(),
        );
        let r = router.do_generate(&opts()).await.unwrap();
        assert_eq!(crate::composite::extract_text(&r.content), "from-a");
    }

    #[tokio::test]
    async fn empty_models_is_error() {
        let router = RouterModel::new(
            vec![],
            Box::new(RuleRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let err = router.do_generate(&opts()).await.unwrap_err();
        assert!(err.to_string().contains("no models"));
    }

    #[tokio::test]
    async fn stream_does_not_fallback_when_routed_child_fails() {
        // G3: the key streaming invariant — even with FallbackPolicy::OnError,
        // a failing primary is NOT retried via the backup during do_stream.
        // (StreamStart is already emitted by the time a retry would matter.)
        let router = RouterModel::new(
            vec![
                child("primary", "primary-out", true),
                child("backup", "backup-out", false),
            ],
            Box::new(RuleRouter),
            FallbackPolicy::OnError, // would fall back in do_generate, must NOT here
            RouterConfig::default(),
        );
        let err = router.do_stream(&opts()).await.unwrap_err();
        assert!(
            err.to_string().contains("primary always fails"),
            "do_stream must surface the primary error, not fall back; got: {err}"
        );
    }

    #[tokio::test]
    async fn usable_as_dyn_language_model_via_generate_text() {
        // G7: the RFCs' headline claim — composite models drop into the
        // generate_text free function unchanged via &dyn LanguageModel.
        let router = RouterModel::new(
            vec![child("a", "works through trait object", false)],
            Box::new(RuleRouter),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        let r = crate::generate::generate_text(
            &router,
            "hello",
            crate::generate::GenerateTextOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(r.text, "works through trait object");
    }

    #[tokio::test]
    async fn retries_each_child_before_falling_back_without_rerouting() {
        let (primary, primary_calls) = retry_child("primary", "primary", usize::MAX);
        let (backup, backup_calls) = retry_child("backup", "backup", 1);
        let route_calls = Arc::new(AtomicUsize::new(0));
        let router = RouterModel::new(
            vec![primary, backup],
            Box::new(CountingRuleRouter(route_calls.clone())),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );
        assert_eq!(router.retry_config().max_retries, 0);

        let result = crate::generate::generate_text(
            &router,
            "hello",
            crate::generate::GenerateTextOptions {
                max_retries: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(result.text, "backup");
        assert_eq!(route_calls.load(Ordering::SeqCst), 1);
        assert_eq!(primary_calls.load(Ordering::SeqCst), 3);
        assert_eq!(backup_calls.load(Ordering::SeqCst), 2);
    }

    /// Children must receive a step-labeled child recording context so their
    /// exchanges and retry attempts are recorded as nested steps.
    #[tokio::test]
    async fn children_receive_step_labeled_recording_contexts() {
        struct StepProbe {
            seen: std::sync::Mutex<Vec<Option<String>>>,
        }

        #[async_trait]
        impl LanguageModel for StepProbe {
            fn provider(&self) -> &str {
                "mock"
            }
            fn model_id(&self) -> &str {
                "probe"
            }
            async fn do_generate(
                &self,
                options: &CallOptions,
            ) -> Result<GenerateResult, AiMuxError> {
                self.seen.lock().unwrap().push(
                    options
                        .recording_context
                        .as_ref()
                        .and_then(RecordingContext::step),
                );
                Err(AiMuxError::Other("probe fails".into()))
            }
            async fn do_stream(&self, _: &CallOptions) -> Result<StreamResult, AiMuxError> {
                unreachable!()
            }
        }

        struct NoopRecorder;
        impl Recorder for NoopRecorder {
            fn record_input(&self, _: &str, _: &CallOptions, _: &str, _: &str) {}
            fn record_provider(&self, _: &str, _: &ProviderRecord) {}
            fn record_exchange(&self, _: &str, _: &HttpExchange) {}
            fn record_exchange_update(
                &self,
                _: &str,
                _: u32,
                _: u32,
                _: &ResponseRecord,
                _: Option<String>,
            ) {
            }
            fn record_outcome(&self, _: &str, _: &OutcomeRecord) {}
            fn flush(&self) {}
        }

        let probe = Arc::new(StepProbe {
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let router = RouterModel::new(
            vec![probe.clone()],
            Box::new(CountingRuleRouter(Arc::new(AtomicUsize::new(0)))),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );

        let options = CallOptions {
            recording_context: Some(RecordingContext::new("call-1", Arc::new(NoopRecorder))),
            ..CallOptions::default()
        };
        let _ = router.do_generate(&options).await;

        let seen = probe.seen.lock().unwrap();
        assert!(!seen.is_empty());
        assert_eq!(seen[0].as_deref(), Some("router[0]:mock/probe"));
    }

    /// Even when every child fails and the caller asked for per-call retries,
    /// the composite operation must run exactly once: the exhausted child's
    /// `Retry` error passes through the outer retry loop instead of replaying
    /// routing and fallback.
    #[tokio::test]
    async fn all_children_failing_does_not_replay_the_composite() {
        let (primary, primary_calls) = retry_child("primary", "primary", usize::MAX);
        let (backup, backup_calls) = retry_child("backup", "backup", usize::MAX);
        let route_calls = Arc::new(AtomicUsize::new(0));
        let router = RouterModel::new(
            vec![primary, backup],
            Box::new(CountingRuleRouter(route_calls.clone())),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );

        let error = crate::generate::generate_text(
            &router,
            "hello",
            crate::generate::GenerateTextOptions {
                max_retries: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, crate::AiMuxError::Retry(_)), "{error:?}");
        assert_eq!(route_calls.load(Ordering::SeqCst), 1);
        assert_eq!(primary_calls.load(Ordering::SeqCst), 3);
        assert_eq!(backup_calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retries_stream_setup_on_selected_child_without_fallback_or_rerouting() {
        let (primary, primary_stream_calls) = retry_child("primary", "primary", 1);
        let (backup, backup_stream_calls) = retry_child("backup", "backup", 0);
        let route_calls = Arc::new(AtomicUsize::new(0));
        let router = RouterModel::new(
            vec![primary, backup],
            Box::new(CountingRuleRouter(route_calls.clone())),
            FallbackPolicy::OnError,
            RouterConfig::default(),
        );

        let _result = crate::generate::stream_text(
            &router,
            "hello",
            crate::generate::GenerateTextOptions {
                max_retries: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(route_calls.load(Ordering::SeqCst), 1);
        assert_eq!(primary_stream_calls.load(Ordering::SeqCst), 2);
        assert_eq!(backup_stream_calls.load(Ordering::SeqCst), 0);
    }
}
