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
