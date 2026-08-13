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
use crate::options::{CallOptions, ToolChoice};
use crate::result::{GenerateResult, StreamResult};
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
    /// reset to `Auto` so references don't carry tool schemas.
    fn reference_options(&self, options: &CallOptions) -> CallOptions {
        let mut o = options.clone();
        if self.config.strip_reference_tools {
            o.tools = None;
            o.tool_choice = ToolChoice::Auto;
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
        let results = join_all(
            self.references
                .iter()
                .map(|m| async { m.do_generate(&ref_opts).await }),
        )
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
                        .model_id
                        .clone()
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
            return Err(AiMuxError::Other(
                "moa: all reference models failed".into(),
            ));
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
        let mut agg = self.aggregator.do_generate(&agg_opts).await?;

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
        let agg = self.aggregator.do_stream(&agg_opts).await?;
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
                    // Stream errors are terminal — relay and stop (the user's
                    // original request was NOT the aggregator body MoA
                    // synthesized, so relaying more would be noise).
                    Err(e) => { yield Err(e); break; }
                }
            }
        };

        // RFC-0022 §3.4: return None for request_body/response_headers. The
        // aggregator's request body is a synthesized prompt (references
        // spliced in), not the user's original — exposing it would mislead
        // cache probing (RFC-0015 fingerprinting).
        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: None,
            response_headers: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::ContentPart;
    use crate::language_model_message::LanguageModelPromptMessage;
    use crate::message::Role;
    use crate::result::GenerateContent;
    use crate::stream_part::StreamPart;
    use crate::types::{
        FinishReason, FinishReasonUnified, TokenUsage,
    };
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::Arc;

    /// A mock child that returns fixed text with fixed usage. `fail` forces an
    /// error. Used as both reference and aggregator.
    struct MockChild {
        name: &'static str,
        text: String,
        fail: bool,
        usage: Usage,
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
            if self.fail {
                return Err(AiMuxError::Other(format!("{} failed", self.name)));
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
                response: crate::types::ResponseMetadata {
                    model_id: Some(self.name.into()),
                    ..Default::default()
                },
                request_body: None,
                response_headers: None,
            })
        }
        async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
            if self.fail {
                return Err(AiMuxError::Other(format!("{} failed", self.name)));
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
                request_body: None,
                response_headers: None,
            })
        }
    }

    fn mk(name: &'static str, text: &str, fail: bool, total: u32) -> ChildModel {
        Arc::new(MockChild {
            name,
            text: text.into(),
            fail,
            usage: Usage {
                input_tokens: TokenUsage {
                    total: Some(total),
                    ..Default::default()
                },
                output_tokens: TokenUsage {
                    total: Some(total),
                    ..Default::default()
                },
                raw: None,
            },
        })
    }

    fn opts_with_prompt() -> CallOptions {
        CallOptions {
            prompt: vec![LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("What is Rust?")],
                provider_options: None,
            }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn generate_fans_out_and_aggregates() {
        let refs = vec![mk("ref-a", "A says hi", false, 10), mk("ref-b", "B says hi", false, 20)];
        let agg = mk("aggregator", "aggregated answer", false, 5);
        let moa = MoaModel::new(refs, agg, MoaConfig::default());

        let r = moa.do_generate(&opts_with_prompt()).await.unwrap();
        // Aggregator text wins as the content.
        assert_eq!(extract_text(&r.content), "aggregated answer");
        // usage = references(10 + 20) + aggregator(5) = 35 on each side.
        assert_eq!(r.usage.input_tokens.total, Some(35));
        assert_eq!(r.usage.output_tokens.total, Some(35));
    }

    #[tokio::test]
    async fn best_effort_drops_failed_reference_and_warns() {
        let refs = vec![mk("ref-a", "A ok", false, 10), mk("ref-b", "B", true, 0)];
        let agg = mk("aggregator", "final", false, 1);
        let moa = MoaModel::new(refs, agg, MoaConfig::default());

        let r = moa.do_generate(&opts_with_prompt()).await.unwrap();
        assert_eq!(extract_text(&r.content), "final");
        // Failed reference still contributed its (zero) usage; only ref-a (10) + agg (1).
        assert_eq!(r.usage.input_tokens.total, Some(11));
        assert!(r.warnings.iter().any(|w| matches!(
            w,
            Warning::Other { message } if message.contains("reference 1 failed")
        )));
    }

    #[tokio::test]
    async fn fail_fast_propagates_first_reference_error() {
        let refs = vec![mk("ref-a", "A", true, 0), mk("ref-b", "B ok", false, 10)];
        let agg = mk("aggregator", "final", false, 1);
        let moa = MoaModel::new(
            refs,
            agg,
            MoaConfig {
                fail_mode: MoaFailMode::FailFast,
                ..Default::default()
            },
        );
        let err = moa.do_generate(&opts_with_prompt()).await.unwrap_err();
        assert!(err.to_string().contains("ref-a failed"));
    }

    #[tokio::test]
    async fn all_references_fail_is_error() {
        let refs = vec![mk("ref-a", "A", true, 0), mk("ref-b", "B", true, 0)];
        let agg = mk("aggregator", "final", false, 1);
        let moa = MoaModel::new(refs, agg, MoaConfig::default());
        let err = moa.do_generate(&opts_with_prompt()).await.unwrap_err();
        assert!(err.to_string().contains("all reference models failed"));
    }

    #[tokio::test]
    async fn no_references_degrades_to_aggregator() {
        // With 0 references, aggregator runs the original prompt directly.
        let agg = mk("aggregator", "solo answer", false, 3);
        let moa = MoaModel::new(vec![], agg, MoaConfig::default());
        let r = moa.do_generate(&opts_with_prompt()).await.unwrap();
        assert_eq!(extract_text(&r.content), "solo answer");
        assert_eq!(r.usage.input_tokens.total, Some(3));
    }

    #[tokio::test]
    async fn stream_runs_references_then_streams_aggregator() {
        let refs = vec![mk("ref-a", "A", false, 7), mk("ref-b", "B", false, 8)];
        let agg = mk("aggregator", "streamed", false, 2);
        let moa = MoaModel::new(refs, agg, MoaConfig::default());

        let result = moa.do_stream(&opts_with_prompt()).await.unwrap();
        let mut s = result.stream;
        let mut saw_start = false;
        let mut text = String::new();
        let mut finish_usage: Option<Usage> = None;
        while let Some(part) = s.next().await {
            match part.unwrap() {
                StreamPart::StreamStart { .. } => saw_start = true,
                StreamPart::TextDelta { delta, .. } => text.push_str(&delta),
                StreamPart::Finish { usage, .. } => finish_usage = Some(usage),
                _ => {}
            }
        }
        assert!(saw_start);
        assert_eq!(text, "streamed");
        // references(7 + 8) + aggregator(2) = 17 on each side.
        assert_eq!(finish_usage.unwrap().input_tokens.total, Some(17));
    }

    #[tokio::test]
    async fn stream_best_effort_emits_drop_warning_in_start() {
        let refs = vec![mk("ref-a", "A", false, 5), mk("ref-b", "B", true, 0)];
        let agg = mk("aggregator", "out", false, 1);
        let moa = MoaModel::new(refs, agg, MoaConfig::default());

        let result = moa.do_stream(&opts_with_prompt()).await.unwrap();
        let mut s = result.stream;
        // First item is our StreamStart, which should carry the drop warning.
        match s.next().await.unwrap().unwrap() {
            StreamPart::StreamStart { warnings } => {
                assert!(warnings
                    .iter()
                    .any(|w| matches!(w, Warning::Other { message } if message.contains("reference 1 failed"))));
            }
            other => panic!("expected StreamStart, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn strip_reference_tools_default_clears_tools() {
        // Verify the helper clears tools/tool_choice when configured (default).
        let agg = mk("agg", "x", false, 0);
        let moa = MoaModel::new(vec![], agg, MoaConfig::default());
        let opts = CallOptions {
            prompt: vec![],
            tools: Some(vec![]),
            tool_choice: ToolChoice::Required,
            ..Default::default()
        };
        let ref_opts = moa.reference_options(&opts);
        assert!(ref_opts.tools.is_none());
        assert_eq!(ref_opts.tool_choice, ToolChoice::Auto);
        // The original options are untouched (clone, not mutate).
        assert_eq!(opts.tool_choice, ToolChoice::Required);
    }

    #[tokio::test]
    async fn no_references_does_not_inject_empty_heading() {
        // S4 regression guard: with 0 references the aggregator prompt is the
        // original prompt verbatim — no "# Reference model responses" heading.
        let prompt = vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("hello")],
            provider_options: None,
        }];
        let built = build_aggregator_prompt(&prompt, None, &[]);
        // Still just the single original message; nothing appended.
        assert_eq!(built.len(), 1);
        // Heading must NOT appear.
        assert!(!built[0].content.iter().any(|p| matches!(
            p,
            ContentPart::Text { text, .. } if text.contains("Reference model responses")
        )));
    }

    /// A mock aggregator whose stream yields TextDelta then a transport Err,
    /// then (misbehaving) more parts. Guards that MoA relays the Err and stops.
    struct MidStreamErrorAggregator;

    #[async_trait]
    impl LanguageModel for MidStreamErrorAggregator {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "agg-err"
        }
        async fn do_generate(
            &self,
            _options: &CallOptions,
        ) -> Result<GenerateResult, AiMuxError> {
            unimplemented!()
        }
        async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
            let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
                Ok(StreamPart::StreamStart { warnings: vec![] }),
                Ok(StreamPart::TextDelta {
                    id: "t1".into(),
                    delta: "partial".into(),
                    provider_metadata: None,
                }),
                // A transport error mid-stream.
                Err(AiMuxError::Other("aggregator transport blew up".into())),
                // A misbehaving aggregator that keeps going after the Err.
                Ok(StreamPart::TextDelta {
                    id: "t2".into(),
                    delta: "should-not-see".into(),
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

    #[tokio::test]
    async fn stream_relays_aggregator_transport_error_and_stops() {
        // N3 regression guard: a mid-stream transport Err from the aggregator
        // is relayed and the stream terminates — no subsequent parts forwarded.
        let refs = vec![mk("ref-a", "A", false, 5)];
        let agg = Arc::new(MidStreamErrorAggregator);
        let moa = MoaModel::new(refs, agg, MoaConfig::default());
        let result = moa.do_stream(&opts_with_prompt()).await.unwrap();
        let parts: Vec<_> = result.stream.collect().await;
        let mut saw_error = false;
        let mut text = String::new();
        for part in parts {
            match part {
                Ok(StreamPart::StreamStart { .. }) => {}
                Ok(StreamPart::TextDelta { delta, .. }) => text.push_str(&delta),
                Err(e) => {
                    saw_error = true;
                    assert!(e.to_string().contains("blew up"), "got: {e}");
                }
                _ => {}
            }
        }
        assert!(saw_error, "expected the aggregator transport error to be relayed");
        assert!(
            !text.contains("should-not-see"),
            "parts after the terminal Err must not be forwarded; got: {text}"
        );
    }
}
