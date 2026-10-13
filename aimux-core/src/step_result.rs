//! One model call of a text generation: the AI SDK's `StepResult`
//! (ai@7.0.127 `generate-text/step-result.ts`).
//!
//! A step stores facts only: the step's content, finish reason, usage,
//! performance, warnings, request and response. Text, reasoning, files,
//! sources, tool calls and tool results are derived from the content by the
//! accessor methods, as the AI SDK's `DefaultStepResult` getters derive them
//! (`step-result.ts:354-417`).
//!
//! Aimux runs one step per call (alignment doc §0.4 Q1): `generate_text` and
//! `stream_text` record exactly one step. `toolsContext` and `runtimeContext`
//! are host-side values with no wire form and are not carried.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::content::AssistantContentPart;
use crate::message::ModelMessage;
use crate::result::{ContentPart, GeneratedFile, ReasoningPart, Source, TypedToolResult};
use crate::shared::{RequestInfo, SharedHeaders};
use crate::timestamp::Timestamp;
use crate::tool::ToolCall;
use crate::types::{FinishReasonUnified, LanguageModelUsage, ProviderMetadata, Warning};

/// The model that ran a step (`StepResult.model`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepModel {
    pub provider: String,
    pub model_id: String,
}

/// Distribution of the gaps between output chunks of a streamed step
/// (`OutputChunkTimingStats`, `step-result.ts:39-57`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputChunkTimingStats {
    pub min: f64,
    pub p10: f64,
    pub median: f64,
    pub avg: f64,
    pub p90: f64,
    pub max: f64,
}

impl OutputChunkTimingStats {
    /// The AI SDK's `calculateOutputChunkTimingStats`
    /// (`generate-text/stream-language-model-call.ts:866-889`): nearest-rank
    /// percentiles over the sorted gaps. `None` when there are no gaps.
    #[must_use]
    pub fn from_gaps(gaps_ms: &[f64]) -> Option<Self> {
        if gaps_ms.is_empty() {
            return None;
        }
        let mut sorted = gaps_ms.to_vec();
        sorted.sort_by(f64::total_cmp);
        let nearest_rank = |percentile: f64| {
            // `Math.ceil(percentile * length) - 1`
            let rank = (percentile * sorted.len() as f64).ceil() as usize;
            sorted[rank.saturating_sub(1)]
        };
        Some(Self {
            min: sorted[0],
            p10: nearest_rank(0.1),
            median: nearest_rank(0.5),
            avg: gaps_ms.iter().sum::<f64>() / gaps_ms.len() as f64,
            p90: nearest_rank(0.9),
            max: sorted[sorted.len() - 1],
        })
    }
}

/// Timing of a step (`StepResultPerformance`, `step-result.ts:62-133`).
/// Durations are milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepResultPerformance {
    /// Output tokens per second over the whole response time.
    pub effective_output_tokens_per_second: f64,
    /// Output tokens per second after the first output chunk (streaming only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens_per_second: Option<f64>,
    /// Input tokens per second until the first output chunk (streaming only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_per_second: Option<f64>,
    /// Input plus output tokens per second over the whole response time.
    pub effective_total_tokens_per_second: f64,
    /// The whole step, including retries and tool execution.
    pub step_time_ms: f64,
    /// From the model call to its finish, including retries.
    pub response_time_ms: f64,
    /// Execution time of each host-executed tool, by tool call id. Aimux does
    /// not execute tools, so this is empty.
    #[serde(default)]
    pub tool_execution_ms: BTreeMap<String, f64>,
    /// From the model call to its first output chunk (streaming only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_output_ms: Option<f64>,
    /// Gaps between output chunks (streaming only, two or more chunks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_between_output_chunks_ms: Option<OutputChunkTimingStats>,
}

/// The AI SDK's `calculateTokensPerSecond`
/// (`generate-text/calculate-tokens-per-second.ts`): a non-finite rate is 0.
#[must_use]
pub fn tokens_per_second(tokens: Option<u32>, duration_ms: f64) -> f64 {
    let rate = 1000.0 * f64::from(tokens.unwrap_or(0)) / duration_ms;
    if rate.is_finite() { rate } else { 0.0 }
}

/// The AI SDK's `sumTokenCounts` (`generate-text/sum-token-counts.ts`).
fn sum_token_counts(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
    }
}

impl StepResultPerformance {
    /// Performance of a step whose model call took `response_time_ms` and
    /// whose whole step took `step_time_ms`. `first_output_ms` and
    /// `chunk_gaps_ms` come from a streamed call
    /// (`generate-text/stream-language-model-call.ts:593-624`); a
    /// non-streamed call has neither (`generate-text/generate-text.ts:1382-1401`).
    #[must_use]
    pub fn new(
        usage: &LanguageModelUsage,
        response_time_ms: f64,
        step_time_ms: f64,
        first_output_ms: Option<f64>,
        chunk_gaps_ms: &[f64],
    ) -> Self {
        Self {
            effective_output_tokens_per_second: tokens_per_second(
                usage.output_tokens,
                response_time_ms,
            ),
            output_tokens_per_second: first_output_ms
                .map(|first| tokens_per_second(usage.output_tokens, response_time_ms - first)),
            input_tokens_per_second: first_output_ms
                .map(|first| tokens_per_second(usage.input_tokens, first)),
            effective_total_tokens_per_second: tokens_per_second(
                sum_token_counts(usage.input_tokens, usage.output_tokens),
                response_time_ms,
            ),
            step_time_ms,
            response_time_ms,
            tool_execution_ms: BTreeMap::new(),
            time_to_first_output_ms: first_output_ms,
            time_between_output_chunks_ms: OutputChunkTimingStats::from_gaps(chunk_gaps_ms),
        }
    }
}

/// The response of a step without its messages and body: the AI SDK's
/// `Omit<LanguageModelResponseMetadata, 'messages' | 'body'>`, carried by the
/// `finish-step` stream part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepResponseMetadata {
    /// Response id; the call layer generates one when the provider sent none.
    pub id: String,
    /// Start of the response; the call layer uses the current time when the
    /// provider sent none.
    pub timestamp: Timestamp,
    /// The model that produced the response.
    pub model_id: String,
    #[serde(default)]
    pub headers: Option<SharedHeaders>,
}

/// The response of a step (`LanguageModelResponseMetadata`,
/// `types/language-model-response-metadata.ts`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepResponse {
    pub id: String,
    pub timestamp: Timestamp,
    pub model_id: String,
    #[serde(default)]
    pub headers: Option<SharedHeaders>,
    /// The raw response body (non-streaming calls).
    #[serde(default)]
    pub body: Option<Value>,
    /// The assistant and tool messages of this step, ready to append to the
    /// prompt of the next call.
    #[serde(default)]
    pub messages: Vec<ModelMessage>,
}

impl StepResponse {
    /// This response without messages and body.
    #[must_use]
    pub fn metadata(&self) -> StepResponseMetadata {
        StepResponseMetadata {
            id: self.id.clone(),
            timestamp: self.timestamp,
            model_id: self.model_id.clone(),
            headers: self.headers.clone(),
        }
    }
}

/// One model call of a text generation (`StepResult`, `step-result.ts:138-278`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepResult {
    /// Identifies the generation call the step belongs to.
    pub call_id: String,
    /// Zero-based step index.
    pub step_number: u32,
    /// The model that ran the step.
    pub model: StepModel,
    /// The content generated in this step, in order.
    pub content: Vec<ContentPart>,
    /// Why the step finished.
    pub finish_reason: FinishReasonUnified,
    /// The provider's own finish reason.
    #[serde(default)]
    pub raw_finish_reason: Option<String>,
    /// Token usage of this step.
    pub usage: LanguageModelUsage,
    /// Timing of this step.
    pub performance: StepResultPerformance,
    /// Warnings of the model call.
    #[serde(default)]
    pub warnings: Vec<Warning>,
    /// The request of the model call.
    #[serde(default)]
    pub request: RequestInfo,
    /// The response of the model call, with the step's response messages.
    pub response: StepResponse,
    /// Provider metadata of the model call.
    #[serde(default)]
    pub provider_metadata: Option<ProviderMetadata>,
}

impl StepResult {
    /// The text of the step: every text part, joined.
    #[must_use]
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The reasoning of the step in input form, ready for a prompt: the AI
    /// SDK's `convertFromReasoningOutputs` (`generate-text/reasoning-output.ts:57-80`)
    /// turns `providerMetadata` into `providerOptions`.
    #[must_use]
    pub fn reasoning(&self) -> Vec<AssistantContentPart> {
        self.reasoning_outputs()
            .into_iter()
            .map(|part| match part {
                ReasoningPart::Text(reasoning) => AssistantContentPart::Reasoning {
                    text: reasoning.text,
                    provider_options: reasoning.provider_metadata,
                },
                // `data: part.file.base64` (`reasoning-output.ts:71-78`).
                ReasoningPart::File(file) => AssistantContentPart::ReasoningFile {
                    data: crate::content::ReasoningFilePartData::String(
                        file.file.base64().into_owned(),
                    ),
                    media_type: file.file.media_type,
                    provider_options: file.provider_metadata,
                },
            })
            .collect()
    }

    /// The reasoning of the step in output form (reasoning and
    /// reasoning-file content parts).
    #[must_use]
    pub fn reasoning_outputs(&self) -> Vec<ReasoningPart> {
        self.content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Reasoning(reasoning) => Some(ReasoningPart::Text(reasoning.clone())),
                ContentPart::ReasoningFile(file) => Some(ReasoningPart::File(file.clone())),
                _ => None,
            })
            .collect()
    }

    /// The reasoning text, or `None` when it is empty (the AI SDK's
    /// `asReasoningText`, `generate-text/reasoning.ts`).
    #[must_use]
    pub fn reasoning_text(&self) -> Option<String> {
        let text: String = self
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Reasoning(reasoning) => Some(reasoning.text.as_str()),
                _ => None,
            })
            .collect();
        (!text.is_empty()).then_some(text)
    }

    /// Files generated in this step.
    #[must_use]
    pub fn files(&self) -> Vec<&GeneratedFile> {
        self.content
            .iter()
            .filter_map(|part| match part {
                ContentPart::File { file, .. } => Some(file),
                _ => None,
            })
            .collect()
    }

    /// Sources cited in this step.
    #[must_use]
    pub fn sources(&self) -> Vec<&Source> {
        self.content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Source(source) => Some(source),
                _ => None,
            })
            .collect()
    }

    /// Tool calls of this step.
    #[must_use]
    pub fn tool_calls(&self) -> Vec<&ToolCall> {
        self.content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolCall(call) => Some(call),
                _ => None,
            })
            .collect()
    }

    /// Tool calls of tools known in advance (`dynamic` is not `true`).
    #[must_use]
    pub fn static_tool_calls(&self) -> Vec<&ToolCall> {
        let mut calls = self.tool_calls();
        calls.retain(|call| call.dynamic != Some(true));
        calls
    }

    /// Tool calls of dynamic tools (`dynamic: true`).
    #[must_use]
    pub fn dynamic_tool_calls(&self) -> Vec<&ToolCall> {
        let mut calls = self.tool_calls();
        calls.retain(|call| call.dynamic == Some(true));
        calls
    }

    /// Tool results of this step.
    #[must_use]
    pub fn tool_results(&self) -> Vec<&TypedToolResult> {
        self.content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolResult(result) => Some(result),
                _ => None,
            })
            .collect()
    }

    /// Tool results of tools known in advance (`dynamic` is not `true`).
    #[must_use]
    pub fn static_tool_results(&self) -> Vec<&TypedToolResult> {
        let mut results = self.tool_results();
        results.retain(|result| result.dynamic != Some(true));
        results
    }

    /// Tool results of dynamic tools (`dynamic: true`).
    #[must_use]
    pub fn dynamic_tool_results(&self) -> Vec<&TypedToolResult> {
        let mut results = self.tool_results();
        results.retain(|result| result.dynamic == Some(true));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stream-language-model-call.ts:866-889: nearest-rank percentiles
    /// (`sorted[ceil(p * n) - 1]`) and the arithmetic mean.
    #[test]
    fn chunk_timing_stats_use_nearest_rank_percentiles() {
        assert_eq!(OutputChunkTimingStats::from_gaps(&[]), None);
        let stats = OutputChunkTimingStats::from_gaps(&[30.0, 10.0, 20.0, 40.0]).unwrap();
        assert_eq!(
            stats,
            OutputChunkTimingStats {
                min: 10.0,
                p10: 10.0,
                median: 20.0,
                avg: 25.0,
                p90: 40.0,
                max: 40.0,
            }
        );
    }

    /// calculate-tokens-per-second.ts: `1000 * (tokens ?? 0) / durationMs`,
    /// 0 when not finite.
    #[test]
    fn tokens_per_second_is_zero_when_not_finite() {
        assert_eq!(tokens_per_second(Some(50), 500.0), 100.0);
        assert_eq!(tokens_per_second(Some(5), 0.0), 0.0);
        assert_eq!(tokens_per_second(None, 0.0), 0.0);
        assert_eq!(tokens_per_second(None, 10.0), 0.0);
    }

    /// generate-text.ts:1382-1401: a generated (non-streamed) step has no
    /// first-output timing, so the streaming-only rates are absent.
    #[test]
    fn generated_step_performance_has_no_streaming_rates() {
        let usage = LanguageModelUsage {
            input_tokens: Some(10),
            output_tokens: Some(20),
            ..LanguageModelUsage::default()
        };
        let performance = StepResultPerformance::new(&usage, 1000.0, 1200.0, None, &[]);
        assert_eq!(performance.effective_output_tokens_per_second, 20.0);
        assert_eq!(performance.effective_total_tokens_per_second, 30.0);
        assert_eq!(performance.output_tokens_per_second, None);
        assert_eq!(performance.input_tokens_per_second, None);
        assert_eq!(performance.step_time_ms, 1200.0);
        assert_eq!(performance.time_between_output_chunks_ms, None);

        // stream-language-model-call.ts:597-616
        let streamed = StepResultPerformance::new(&usage, 1000.0, 1000.0, Some(200.0), &[5.0]);
        assert_eq!(streamed.output_tokens_per_second, Some(25.0));
        assert_eq!(streamed.input_tokens_per_second, Some(50.0));
    }
}
