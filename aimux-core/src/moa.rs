//! `MoaModel` — Mixture-of-Agents single-fanout aggregation (RFC-0022).
//!
//! Reference models run in parallel (non-streaming) → their text outputs are
//! spliced into the aggregator prompt → the aggregator runs and its output is
//! returned. This all happens inside a single `do_generate` / `do_stream`,
//! with **no agent loop**. `MoaModel` implements [`LanguageModel`], so it drops
//! into `generate_text` / `stream_text` and every binding unchanged.
//!
//! See [`crate::composite`] for the shared skeleton (`ChildModel`, `add_usage`,
//! `build_aggregator_prompt`).

use async_trait::async_trait;
use futures::{StreamExt, future::join_all};
use serde::Deserialize;

use crate::composite::{ChildModel, add_usage, build_aggregator_prompt, extract_text};
use crate::error::AiMuxError;
use crate::language_model::LanguageModel;
use crate::options::CallOptions;
use crate::result::{GenerateResult, StreamResult};
use crate::retry;
use crate::stream_part::StreamPart;
use crate::types::{Usage, Warning};

/// Reference-failure policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoaFailMode {
    /// Drop a failed reference + emit a `Warning::Other`; keep going (default).
    #[default]
    BestEffort,
    /// Fail the whole call as soon as any reference errors.
    FailFast,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MoaConfig {
    /// `provider()` value (default `"moa"`).
    #[serde(default = "default_moa_provider")]
    pub provider_name: String,
    /// `model_id()` value (default `"moa"`).
    #[serde(default = "default_moa_provider")]
    pub model_id: String,
    /// Optional aggregator system instruction prepended to the reference user
    /// message. `None` uses a built-in default.
    #[serde(default)]
    pub aggregator_instructions: Option<String>,
    /// Strip `tools` / `tool_choice` from reference calls so references stay
    /// cheap (Hermes: references don't carry tool schemas). Default `true`.
    #[serde(default = "default_strip_reference_tools")]
    pub strip_reference_tools: bool,
    /// Reference-failure policy.
    #[serde(default)]
    pub fail_mode: MoaFailMode,
}

fn default_moa_provider() -> String {
    "moa".into()
}

fn default_strip_reference_tools() -> bool {
    true
}

// Manual `Default`: `strip_reference_tools` must default to `true` (a derived
// Default would give `false`, contradicting the documented behavior).
impl Default for MoaConfig {
    fn default() -> Self {
        Self {
            provider_name: "moa".into(),
            model_id: "moa".into(),
            aggregator_instructions: None,
            strip_reference_tools: true,
            fail_mode: MoaFailMode::BestEffort,
        }
    }
}

/// Mixture-of-Agents single-fanout aggregation model.
///
/// References fan out in parallel (non-streaming) → outputs are spliced into
/// the aggregator prompt → the aggregator produces the final result. One
/// `generate_text` / `stream_text` call, no agent loop.
pub struct MoaModel {
    references: Vec<ChildModel>,
    aggregator: ChildModel,
    config: MoaConfig,
}

impl MoaModel {
    pub fn new(references: Vec<ChildModel>, aggregator: ChildModel, config: MoaConfig) -> Self {
        Self {
            references,
            aggregator,
            config,
        }
    }

    /// Build reference call options from the user's options. When
    /// `strip_reference_tools` is set, `tools` is cleared and `tool_choice` is
    /// cleared so references don't carry tool schemas.
    fn reference_options(&self, options: &CallOptions) -> CallOptions {
        let mut o = options.clone();
        if self.config.strip_reference_tools {
            o.tools = None;
            o.tool_choice = None;
        }
        o
    }

    /// Fan out references in parallel (non-streaming), accumulate usage, and
    /// collect `(model_id, text)` for the successful ones. Failures are handled
    /// per `fail_mode`. Returns the reference texts, accumulated usage, and any
    /// drop warnings. Errors if all references fail (and references were
    /// configured).
    async fn run_references_nonstream(
        &self,
        options: &CallOptions,
    ) -> Result<(Vec<(String, String)>, Usage, Vec<Warning>), AiMuxError> {
        if self.references.is_empty() {
            return Ok((Vec::new(), Usage::default(), Vec::new()));
        }
        let ref_opts = self.reference_options(options);
        let results = join_all(self.references.iter().enumerate().map(|(i, m)| {
            let child_opts =
                ref_opts.for_step(format!("moa.ref[{i}]:{}/{}", m.provider(), m.model_id()));
            async move {
                let retries = retry::prepare_retries(
                    child_opts.max_retries,
                    m.retry_config(),
                    child_opts.abort_signal.clone(),
                );
                retries
                    .retry(|| {
                        if let Some(context) = &child_opts.recording_context {
                            let _ = context.start_attempt();
                        }
                        m.do_generate(&child_opts)
                    })
                    .await
            }
        }))
        .await;

        let mut usage = Usage::default();
        let mut warnings = Vec::new();
        let mut texts: Vec<(String, String)> = Vec::new();
        for (i, r) in results.into_iter().enumerate() {
            match r {
                Ok(res) => {
                    usage = add_usage(usage, &res.usage);
                    let mid = res
                        .response
                        .as_ref()
                        .and_then(|response| response.model_id.clone())
                        .unwrap_or_else(|| format!("ref-{i}"));
                    texts.push((mid, extract_text(&res.content)));
                }
                Err(e) => {
                    if self.config.fail_mode == MoaFailMode::FailFast {
                        return Err(e);
                    }
                    warnings.push(Warning::Other {
                        message: format!("moa reference {i} failed: {e}"),
                    });
                }
            }
        }
        if texts.is_empty() {
            return Err(AiMuxError::Other("moa: all reference models failed".into()));
        }
        Ok((texts, usage, warnings))
    }
}

#[async_trait]
impl LanguageModel for MoaModel {
    fn provider(&self) -> &str {
        &self.config.provider_name
    }

    fn model_id(&self) -> &str {
        &self.config.model_id
    }

    fn retry_config(&self) -> retry::RetryConfig {
        // Retrying the composite would rerun the entire reference fanout;
        // references and aggregator are retried independently instead.
        retry::RetryConfig {
            max_retries: 0,
            ..retry::RetryConfig::default()
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        // 1. Fan out references (non-streaming).
        let (texts, ref_usage, warnings) = self.run_references_nonstream(options).await?;

        // 2. Build the aggregator prompt + options.
        let agg_prompt = build_aggregator_prompt(
            &options.prompt,
            self.config.aggregator_instructions.as_deref(),
            &texts,
        );
        let mut agg_opts = options.clone();
        agg_opts.prompt = agg_prompt;

        // 3. Run the aggregator.
        let retries = retry::prepare_retries(
            agg_opts.max_retries,
            self.aggregator.retry_config(),
            agg_opts.abort_signal.clone(),
        );
        let agg_opts = agg_opts.for_step(format!(
            "moa.aggregator:{}/{}",
            self.aggregator.provider(),
            self.aggregator.model_id()
        ));
        let mut agg = retries
            .retry(|| {
                if let Some(context) = &agg_opts.recording_context {
                    let _ = context.start_attempt();
                }
                self.aggregator.do_generate(&agg_opts)
            })
            .await?;

        // 4. Fold reference usage + drop warnings into the aggregator result.
        agg.usage = add_usage(agg.usage, &ref_usage);
        agg.warnings.extend(warnings);
        Ok(agg)
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        // 1. Fan out references (non-streaming, blocking until done — MoA
        //    inherent latency). Errors surface as `Err` from `do_stream`.
        let (texts, ref_usage, drop_warnings) = self.run_references_nonstream(options).await?;

        // 2. Aggregator prompt + options.
        let agg_prompt = build_aggregator_prompt(
            &options.prompt,
            self.config.aggregator_instructions.as_deref(),
            &texts,
        );
        let mut agg_opts = options.clone();
        agg_opts.prompt = agg_prompt;

        // 3. Aggregator streams; we emit our own StreamStart and add reference
        //    usage onto its Finish. We swallow the aggregator's StreamStart
        //    (we've already emitted ours).
        let retries = retry::prepare_retries(
            agg_opts.max_retries,
            self.aggregator.retry_config(),
            agg_opts.abort_signal.clone(),
        );
        let agg_opts = agg_opts.for_step(format!(
            "moa.aggregator:{}/{}",
            self.aggregator.provider(),
            self.aggregator.model_id()
        ));
        let agg = retries
            .retry(|| {
                if let Some(context) = &agg_opts.recording_context {
                    let _ = context.start_attempt();
                }
                self.aggregator.do_stream(&agg_opts)
            })
            .await?;
        let mut agg_stream = agg.stream;

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: drop_warnings });
            while let Some(part) = agg_stream.next().await {
                match part {
                    Ok(StreamPart::StreamStart { .. }) => { /* swallow */ }
                    Ok(StreamPart::Finish { finish_reason, usage, provider_metadata }) => {
                        yield Ok(StreamPart::Finish {
                            finish_reason,
                            usage: add_usage(usage, &ref_usage),
                            provider_metadata,
                        });
                    }
                    Ok(other) => yield Ok(other),
                    Err(e) => {
                        let recoverable = e.is_recoverable_stream_error();
                        yield Err(e);
                        if !recoverable {
                            // A transport/Core failure is terminal. Retrying or
                            // replaying the synthesized aggregator request after
                            // output has escaped would duplicate visible data.
                            break;
                        }
                    }
                }
            }
        };

        // RFC-0022 §3.4: return None for request/response. The
        // aggregator's request body is a synthesized prompt (references
        // spliced in), not the user's original — exposing it would mislead
        // cache probing (RFC-0015 fingerprinting).
        Ok(StreamResult {
            stream: Box::pin(stream),
            request: None,
            response: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language_model_message::LanguageModelMessage;
    use crate::result::GenerateContent;
    use crate::stream_part::StreamPart;
    use crate::types::{FinishReason, FinishReasonUnified};
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A mock child that returns fixed text with fixed usage. `fail` forces an
    /// error. Used as both reference and aggregator.
    struct MockChild {
        name: &'static str,
        text: String,
        fail: bool,
        usage: Usage,
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
                return Err(AiMuxError::Other(format!("{} failed", self.name)));
            }
            if attempt <= self.retry_failures {
                return Err(retryable_error(self.name));
            }
            Ok(GenerateResult {
                content: vec![GenerateContent::Text {
                    text: self.text.clone(),
                    provider_metadata: None,
                }],
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: self.usage.clone(),
                warnings: vec![],
                provider_metadata: None,
                response: Some(
                    crate::types::ResponseMetadata {
                        model_id: Some(self.name.into()),
                        ..Default::default()
                    }
                    .into(),
                ),
                request: None,
            })
        }
        async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail {
                return Err(AiMuxError::Other(format!("{} failed", self.name)));
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
                    usage: self.usage.clone(),
                    provider_metadata: None,
                }),
            ];
            Ok(StreamResult {
                stream: Box::pin(stream::iter(parts)),
                request: None,
                response: None,
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
                usage: Usage::default(),
                retry_failures,
                calls: calls.clone(),
            }),
            calls,
        )
    }

    #[tokio::test]
    async fn retries_references_and_aggregator_independently_without_replaying_fanout() {
        let (ref_a, ref_a_calls) = retry_child("ref-a", "A", 1);
        let (ref_b, ref_b_calls) = retry_child("ref-b", "B", 0);
        let (aggregator, aggregator_calls) = retry_child("aggregator", "final", 1);
        let moa = MoaModel::new(vec![ref_a, ref_b], aggregator, MoaConfig::default());
        assert_eq!(moa.retry_config().max_retries, 0);

        let result = crate::generate::generate_text(
            &moa,
            "hello",
            crate::generate::GenerateTextOptions {
                max_retries: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(result.text, "final");
        assert_eq!(ref_a_calls.load(Ordering::SeqCst), 2);
        assert_eq!(ref_b_calls.load(Ordering::SeqCst), 1);
        assert_eq!(aggregator_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retries_aggregator_stream_setup_without_replaying_references() {
        let (reference, reference_calls) = retry_child("reference", "A", 0);
        let (aggregator, aggregator_stream_calls) = retry_child("aggregator", "final", 1);
        let moa = MoaModel::new(vec![reference], aggregator, MoaConfig::default());

        let _result = crate::generate::stream_text(
            &moa,
            "hello",
            crate::generate::GenerateTextOptions {
                max_retries: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(reference_calls.load(Ordering::SeqCst), 1);
        assert_eq!(aggregator_stream_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn no_references_does_not_inject_empty_heading() {
        // S4 regression guard: with 0 references the aggregator prompt is the
        // original prompt verbatim — no "# Reference model responses" heading.
        let prompt = vec![LanguageModelMessage::user_text("hello")];
        let built = build_aggregator_prompt(&prompt, None, &[]);
        // Still just the single original message; nothing appended.
        assert_eq!(built.len(), 1);
        assert_eq!(built, prompt);
    }
}
