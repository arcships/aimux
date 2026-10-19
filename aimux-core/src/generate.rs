//! User-facing API: `generate_text` and `stream_text`.
//!
//! These are the primary entry points for users. They:
//! 1. Convert user prompt (`ModelPrompt`) to `LanguageModelPrompt`
//!    (provider-facing), first downloading the file URLs the model does not
//!    fetch itself.
//! 2. Build `CallOptions` from user-facing options.
//! 3. Call `LanguageModel::do_generate` / `do_stream`.
//! 4. Record the model call as a [`StepResult`] and aggregate the steps into
//!    a [`GenerateTextResult`], as the AI SDK's `generateText` and
//!    `streamText` do (ai@7.0.127 `generate-text/generate-text.ts`,
//!    `generate-text/stream-text.ts`). Aimux runs one step per call.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Instant;

use futures::{Stream, StreamExt};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use tracing::Instrument;
use ts_rs::TS;

use crate::download::{DownloadFunction, create_default_download_function};
use crate::error::AiMuxError;
use crate::language_model::LanguageModel;
use crate::language_model_message::{
    LanguageModelPrompt, convert_with_downloaded_assets, download_assets,
};
use crate::message::{ModelMessage, ModelPrompt};
use crate::operation::OperationSettings;
use crate::options::{CallOptions, ToolChoice, prepare_tool_choice};
use crate::output::Output;
use crate::parse_tool_call::{ToolCallRepair, parse_tool_call};
use crate::result::{
    ContentPart, GenerateContent, GenerateResult, GeneratedFile, GeneratedFileOutput,
    LanguageModelFile, ReasoningPart, Source, StreamResult, ToolApprovalRequestOutput,
    TypedToolError, TypedToolResult,
};
use crate::shared::{RequestInfo, SharedHeaders, SharedProviderOptions};
use crate::step_result::{
    StepModel, StepResponse, StepResponseMetadata, StepResult, StepResultPerformance,
};
use crate::stream_part::{StreamPart, StreamPartPayload, TextStreamPart};
use crate::tool::{Tool, ToolCall, ToolResult};
use crate::types::{
    FinishReasonUnified, LanguageModelUsage, ProviderMetadata, ReasoningEffort, Warning,
    add_language_model_usage, as_language_model_usage,
};
use crate::{AbortSignal, retry, timeout};

fn response_metadata_defaults(
    mut metadata: crate::types::ResponseMetadata,
    model_id: &str,
) -> crate::types::ResponseMetadata {
    metadata.id.get_or_insert_with(|| {
        use rand::Rng;
        rand::thread_rng()
            .sample_iter(rand::distributions::Alphanumeric)
            .take(16)
            .map(char::from)
            .collect()
    });
    metadata
        .timestamp
        .get_or_insert_with(crate::timestamp::Timestamp::now);
    metadata.model_id.get_or_insert_with(|| model_id.to_owned());
    metadata
}

/// Milliseconds between two instants, the unit of the AI SDK's step timing
/// (`util/now.ts` reads `performance.now()`).
fn ms_between(start: Instant, end: Instant) -> f64 {
    end.saturating_duration_since(start).as_secs_f64() * 1000.0
}

/// Matches the AI SDK's `isOutputChunk`: only chunks containing model output
/// start or reset the first/chunk output timers.
fn is_output_chunk<C, A, R>(part: &StreamPart<C, A, R>) -> bool {
    match part {
        StreamPart::TextDelta { delta, .. }
        | StreamPart::ReasoningDelta { delta, .. }
        | StreamPart::ToolInputDelta { delta, .. } => !delta.is_empty(),
        StreamPart::ToolCall(_) | StreamPart::File(_) | StreamPart::ReasoningFile(_) => true,
        _ => false,
    }
}

/// The text-generation file part for a provider file: its URL is downloaded,
/// the file is `{ base64, mediaType }`, and the part keeps the provider
/// metadata (`convert-language-model-content.ts:55-70`,
/// `stream-language-model-call.ts:569-590`).
async fn resolve_file(
    file: LanguageModelFile,
    abort_signal: Option<&AbortSignal>,
) -> Result<GeneratedFileOutput, AiMuxError> {
    Ok(GeneratedFileOutput {
        file: GeneratedFile {
            data: crate::download::resolve_generated_file_data(&file.data, abort_signal).await?,
            media_type: file.media_type,
            provider_metadata: None,
        },
        provider_metadata: file.provider_metadata,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// User-facing options
// ─────────────────────────────────────────────────────────────────────────────

/// User-facing options for `generate_text` and `stream_text`.
///
/// Unlike `CallOptions` (provider-facing), this does not include `prompt`
/// (passed separately) and defaults are more ergonomic.
///
/// Unknown keys are rejected (RFC-0042 §7.2): a host still writing aimux
/// 0.5's snake_case keys (`max_retries`, `timeout.total_ms`) gets an error
/// naming the key instead of the default it would silently fall back to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct GenerateTextOptions {
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<f64>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    /// The structured output to generate, the AI SDK's `output`
    /// (`generate-text/output.ts`). It sets the model call's
    /// `response_format` and parses the final text into
    /// [`GenerateTextResult::output`]. `None` sends no response format and
    /// takes the text as the output.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
    /// Top-level reasoning effort.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningEffort>,
    /// System instructions prepended to the prompt.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Maximum number of retries, the AI SDK's `maxRetries`; `Some(0)` disables
    /// retrying. `None` uses 2, the AI SDK default.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// Per-call timeout configuration (total / first-chunk / chunk idle).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<crate::options::TimeoutConfiguration>,
    /// Session identifier, for grouping consecutive calls into a session
    /// (observability, see RFC-0024). Orthogonal to RFC-0019 session-affinity
    /// headers. When `None` and the optional session inferer is enabled, one
    /// may be inferred from prompt-prefix continuation.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Abort signal for cancelling the call.
    ///
    /// Runtime handle — never crosses the JSON boundary. Rust callers set it
    /// directly; the Node binding bridges a JS `AbortSignal` natively.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,
    /// Emit raw provider stream chunks as `StreamPart::Raw` (debugging aid).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_raw_chunks: Option<bool>,
    /// Optional one-shot repair callback for unknown, malformed, or
    /// schema-invalid tool calls.
    #[serde(skip)]
    #[ts(skip)]
    pub repair_tool_call: Option<ToolCallRepair>,
    /// The download of the prompt's file URLs, the AI SDK's
    /// `experimental_download` (ai@7.0.127 `generate-text.ts:382-386`,
    /// `stream-text.ts:439`). Before the model call it receives every file
    /// URL of the prompt, each marked with whether the model's
    /// `supported_urls` cover it; the files it returns are sent inline.
    /// `None` downloads the URLs the model does not fetch itself with the
    /// guarded download
    /// ([`create_default_download_function`]).
    ///
    /// Runtime handle; never crosses the JSON boundary.
    #[serde(skip)]
    #[ts(skip)]
    pub download: Option<DownloadFunction>,
}

impl GenerateTextOptions {
    /// The operation settings of these options: the retry count, the timeout
    /// and the session id, which the model call options do not carry.
    #[must_use]
    pub fn operation_settings(&self) -> OperationSettings {
        OperationSettings {
            max_retries: self.max_retries,
            timeout: self.timeout,
            session_id: self.session_id.clone(),
        }
    }

    /// Build the provider-facing `CallOptions` from user-facing options.
    /// The operation settings ([`Self::operation_settings`]) are left out.
    #[must_use]
    pub fn into_call_options(
        self,
        prompt: crate::language_model_message::LanguageModelPrompt,
    ) -> CallOptions {
        // `generateText` / `streamText` always send the prepared choice,
        // `{type:"auto"}` when none is given (ai@7.0.127
        // src/generate-text/generate-text.ts:953-955, stream-text.ts:2382-2384).
        let tool_choice = Some(prepare_tool_choice(self.tool_choice));
        CallOptions {
            prompt,
            max_output_tokens: self.max_output_tokens,
            temperature: self.temperature,
            stop_sequences: self.stop_sequences,
            top_p: self.top_p,
            top_k: self.top_k,
            presence_penalty: self.presence_penalty,
            frequency_penalty: self.frequency_penalty,
            // `responseFormat: await output?.responseFormat`
            // (generate-text.ts:1048, stream-language-model-call.ts:361).
            response_format: self.output.as_ref().map(Output::response_format),
            seed: self.seed,
            tools: self.tools,
            tool_choice,
            headers: self.headers,
            provider_options: self.provider_options,
            reasoning: self.reasoning,
            abort_signal: self.abort_signal,
            include_raw_chunks: self.include_raw_chunks,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// User-facing result types
// ─────────────────────────────────────────────────────────────────────────────

/// Result of `generate_text` and of a consumed `stream_text`: the AI SDK's
/// `GenerateTextResult` (ai@7.0.127 `generate-text/generate-text-result.ts`).
///
/// It stores the steps, the usage summed over them and the parsed output, as
/// `DefaultGenerateTextResult` does (`generate-text.ts:1691-1808`); every
/// other value is derived from the steps by an accessor. The serialized form
/// is a snapshot that writes the derived values next to the stored ones, so
/// a binding reads `text`, `tool_calls`, `usage` and the rest directly. Only
/// `steps`, `total_usage` and `output` are read back.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "GenerateTextResultFacts")]
pub struct GenerateTextResult {
    steps: Vec<StepResult>,
    total_usage: LanguageModelUsage,
    output: OutputState,
}

/// The output of a result.
#[derive(Debug, Clone)]
pub(crate) enum OutputState {
    /// Parsed by the output specification.
    Parsed(Value),
    /// Not parsed: `generate_text` parses only a final step that finished
    /// with `stop`, or with text and not with tool calls
    /// (`generate-text.ts:1599-1612`).
    NotParsed,
    /// Parsing a streamed final step failed. Only reading the output fails,
    /// as only `StreamTextResult.output` rejects upstream
    /// (`stream-text.ts:3419-3439`).
    Failed(AiMuxError),
}

/// The text output, used when no output is configured (`output ?? text()`).
static TEXT_OUTPUT: Output = Output::Text {};

impl GenerateTextResult {
    /// Aggregate the steps of a call with the usage summed over them
    /// (`generate-text.ts:1526-1544`).
    pub(crate) fn from_steps(steps: Vec<StepResult>, output: OutputState) -> Self {
        let total_usage = steps
            .iter()
            .fold(LanguageModelUsage::default(), |total, step| {
                add_language_model_usage(&total, &step.usage)
            });
        Self::with_total_usage(steps, total_usage, output)
    }

    fn with_total_usage(
        steps: Vec<StepResult>,
        total_usage: LanguageModelUsage,
        output: OutputState,
    ) -> Self {
        assert!(!steps.is_empty(), "a text result has at least one step");
        Self {
            steps,
            total_usage,
            output,
        }
    }

    /// Every step of the call, in order. Aimux runs one step per call.
    #[must_use]
    pub fn steps(&self) -> &[StepResult] {
        &self.steps
    }

    /// The last step.
    #[must_use]
    pub fn final_step(&self) -> &StepResult {
        self.steps
            .last()
            .expect("a text result has at least one step")
    }

    /// The content of every step.
    #[must_use]
    pub fn content(&self) -> Vec<&ContentPart> {
        self.steps.iter().flat_map(|step| &step.content).collect()
    }

    /// The text of the last step.
    #[must_use]
    pub fn text(&self) -> String {
        self.final_step().text()
    }

    /// The reasoning of the last step in output form.
    #[deprecated(note = "use `final_step().reasoning()`, as the AI SDK deprecates \
                         `GenerateTextResult.reasoning` for `finalStep.reasoning`")]
    #[must_use]
    pub fn reasoning(&self) -> Vec<ReasoningPart> {
        self.final_step().reasoning_outputs()
    }

    /// The reasoning text of the last step, `None` when it is empty.
    #[deprecated(
        note = "use `final_step().reasoning_text()`, as the AI SDK deprecates \
                         `GenerateTextResult.reasoningText` for `finalStep.reasoningText`"
    )]
    #[must_use]
    pub fn reasoning_text(&self) -> Option<String> {
        self.final_step().reasoning_text()
    }

    /// The files of every step.
    #[must_use]
    pub fn files(&self) -> Vec<&GeneratedFile> {
        self.steps.iter().flat_map(StepResult::files).collect()
    }

    /// The sources of every step.
    #[must_use]
    pub fn sources(&self) -> Vec<&Source> {
        self.steps.iter().flat_map(StepResult::sources).collect()
    }

    /// The tool calls of every step.
    #[must_use]
    pub fn tool_calls(&self) -> Vec<&ToolCall> {
        self.steps.iter().flat_map(StepResult::tool_calls).collect()
    }

    /// The tool calls of tools known in advance, from every step.
    #[must_use]
    pub fn static_tool_calls(&self) -> Vec<&ToolCall> {
        self.steps
            .iter()
            .flat_map(StepResult::static_tool_calls)
            .collect()
    }

    /// The tool calls of dynamic tools, from every step.
    #[must_use]
    pub fn dynamic_tool_calls(&self) -> Vec<&ToolCall> {
        self.steps
            .iter()
            .flat_map(StepResult::dynamic_tool_calls)
            .collect()
    }

    /// The tool results of every step.
    #[must_use]
    pub fn tool_results(&self) -> Vec<&TypedToolResult> {
        self.steps
            .iter()
            .flat_map(StepResult::tool_results)
            .collect()
    }

    /// The tool results of tools known in advance, from every step.
    #[must_use]
    pub fn static_tool_results(&self) -> Vec<&TypedToolResult> {
        self.steps
            .iter()
            .flat_map(StepResult::static_tool_results)
            .collect()
    }

    /// The tool results of dynamic tools, from every step.
    #[must_use]
    pub fn dynamic_tool_results(&self) -> Vec<&TypedToolResult> {
        self.steps
            .iter()
            .flat_map(StepResult::dynamic_tool_results)
            .collect()
    }

    /// Why the last step finished.
    #[must_use]
    pub fn finish_reason(&self) -> FinishReasonUnified {
        self.final_step().finish_reason
    }

    /// The provider's own finish reason of the last step.
    #[must_use]
    pub fn raw_finish_reason(&self) -> Option<&str> {
        self.final_step().raw_finish_reason.as_deref()
    }

    /// The usage summed over every step.
    #[must_use]
    pub fn usage(&self) -> &LanguageModelUsage {
        &self.total_usage
    }

    /// The usage summed over every step.
    #[deprecated(note = "use `usage()`, as the AI SDK deprecates \
                         `GenerateTextResult.totalUsage` for `usage`")]
    #[must_use]
    pub fn total_usage(&self) -> &LanguageModelUsage {
        &self.total_usage
    }

    /// The warnings of every step.
    #[must_use]
    pub fn warnings(&self) -> Vec<&Warning> {
        self.steps.iter().flat_map(|step| &step.warnings).collect()
    }

    /// The request of the last step.
    #[deprecated(note = "use `final_step().request`, as the AI SDK deprecates \
                         `GenerateTextResult.request` for `finalStep.request`")]
    #[must_use]
    pub fn request(&self) -> &RequestInfo {
        &self.final_step().request
    }

    /// The response of the last step.
    #[deprecated(note = "use `final_step().response`, as the AI SDK deprecates \
                         `GenerateTextResult.response` for `finalStep.response`")]
    #[must_use]
    pub fn response(&self) -> &StepResponse {
        &self.final_step().response
    }

    /// The response messages of every step, ready to append to the prompt of
    /// the next call.
    #[must_use]
    pub fn response_messages(&self) -> Vec<&ModelMessage> {
        self.steps
            .iter()
            .flat_map(|step| &step.response.messages)
            .collect()
    }

    /// The provider metadata of the last step.
    #[deprecated(
        note = "use `final_step().provider_metadata`, as the AI SDK deprecates \
                         `GenerateTextResult.providerMetadata` for `finalStep.providerMetadata`"
    )]
    #[must_use]
    pub fn provider_metadata(&self) -> Option<&ProviderMetadata> {
        self.final_step().provider_metadata.as_ref()
    }

    /// The output parsed by [`GenerateTextOptions::output`] (the text when
    /// no output is configured).
    ///
    /// # Errors
    ///
    /// `NoOutputGenerated` ("No output generated.", the AI SDK's
    /// `NoOutputGeneratedError`) when the output was not parsed; for a
    /// streamed result, the parse error when parsing failed.
    pub fn output(&self) -> Result<&Value, AiMuxError> {
        match &self.output {
            OutputState::Parsed(value) => Ok(value),
            OutputState::NotParsed => Err(AiMuxError::no_output_generated("No output generated.")),
            OutputState::Failed(error) => Err(error.clone()),
        }
    }

    pub(crate) fn steps_mut(&mut self) -> &mut [StepResult] {
        &mut self.steps
    }

    fn snapshot(&self) -> GenerateTextResultSnapshot<'_> {
        let final_step = self.final_step();
        GenerateTextResultSnapshot {
            content: self.content(),
            text: final_step.text(),
            reasoning: final_step.reasoning_outputs(),
            reasoning_text: final_step.reasoning_text(),
            files: self.files(),
            sources: self.sources(),
            tool_calls: self.tool_calls(),
            tool_results: self.tool_results(),
            finish_reason: final_step.finish_reason,
            raw_finish_reason: final_step.raw_finish_reason.as_deref(),
            usage: &self.total_usage,
            total_usage: &self.total_usage,
            warnings: self.warnings(),
            request: &final_step.request,
            response: &final_step.response,
            response_messages: self.response_messages(),
            provider_metadata: final_step.provider_metadata.as_ref(),
            steps: &self.steps,
            output: match &self.output {
                OutputState::Parsed(value) => Some(value),
                OutputState::NotParsed | OutputState::Failed(_) => None,
            },
        }
    }
}

/// The serialized [`GenerateTextResult`]: the stored values and the values
/// the AI SDK's result derives from them (`generate-text.ts:1714-1808`).
/// `finalStep` and the static/dynamic subsets are left out: they are the
/// last entry of `steps` and a filter on `dynamic`.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "GenerateTextResult")]
#[cfg_attr(feature = "ts-export", ts(export))]
struct GenerateTextResultSnapshot<'a> {
    content: Vec<&'a ContentPart>,
    text: String,
    /// Deprecated upstream: the last step's reasoning.
    reasoning: Vec<ReasoningPart>,
    /// Deprecated upstream: the last step's reasoning text.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    reasoning_text: Option<String>,
    files: Vec<&'a GeneratedFile>,
    /// Each with its `type` tag (`generate-text-result.ts:73`).
    #[serde(with = "crate::wire::typed_vec")]
    #[ts(as = "Vec<crate::result::TypedSource>")]
    sources: Vec<&'a Source>,
    /// Each with its `type` tag (`generate-text-result.ts:78`).
    #[serde(with = "crate::wire::typed_vec")]
    #[ts(as = "Vec<crate::tool::TypedToolCall>")]
    tool_calls: Vec<&'a ToolCall>,
    /// Each with its `type` tag (`generate-text-result.ts:93`).
    #[serde(with = "crate::wire::typed_vec")]
    #[ts(as = "Vec<crate::result::TaggedToolResult>")]
    tool_results: Vec<&'a TypedToolResult>,
    finish_reason: FinishReasonUnified,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    raw_finish_reason: Option<&'a str>,
    usage: &'a LanguageModelUsage,
    /// Deprecated upstream: equal to `usage`.
    total_usage: &'a LanguageModelUsage,
    warnings: Vec<&'a Warning>,
    /// Deprecated upstream: the last step's request.
    request: &'a RequestInfo,
    /// Deprecated upstream: the last step's response.
    response: &'a StepResponse,
    response_messages: Vec<&'a ModelMessage>,
    /// Deprecated upstream: the last step's provider metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    provider_metadata: Option<&'a ProviderMetadata>,
    steps: &'a [StepResult],
    /// Absent when the output was not parsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    output: Option<&'a Value>,
}

impl Serialize for GenerateTextResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.snapshot().serialize(serializer)
    }
}

/// The values a serialized [`GenerateTextResult`] is read back from.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateTextResultFacts {
    steps: Vec<StepResult>,
    total_usage: LanguageModelUsage,
    #[serde(default, deserialize_with = "present_value")]
    output: Option<Value>,
}

/// A present `output` is `Some`, JSON `null` included.
fn present_value<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

impl TryFrom<GenerateTextResultFacts> for GenerateTextResult {
    type Error = String;

    fn try_from(facts: GenerateTextResultFacts) -> Result<Self, Self::Error> {
        if facts.steps.is_empty() {
            return Err("a text result has at least one step".into());
        }
        Ok(Self::with_total_usage(
            facts.steps,
            facts.total_usage,
            facts
                .output
                .map_or(OutputState::NotParsed, OutputState::Parsed),
        ))
    }
}

/// Parse the output of a generated final step: only a step that finished with
/// `stop`, or with text and not with tool calls, is parsed, and a parse
/// failure fails the call (`generate-text.ts:1599-1612`).
fn generate_output(step: &StepResult, output: Option<&Output>) -> Result<OutputState, AiMuxError> {
    let text = step.text();
    if step.finish_reason == FinishReasonUnified::Stop
        || (step.finish_reason != FinishReasonUnified::ToolCalls && !text.is_empty())
    {
        let value = output
            .unwrap_or(&TEXT_OUTPUT)
            .parse_complete_output(&text)?;
        Ok(OutputState::Parsed(value))
    } else {
        Ok(OutputState::NotParsed)
    }
}

/// Parse the output of a streamed final step: always parsed, and a failure
/// only fails reading the output (`stream-text.ts:3419-3435`).
fn stream_output(step: &StepResult, output: Option<&Output>) -> OutputState {
    match output
        .unwrap_or(&TEXT_OUTPUT)
        .parse_complete_output(&step.text())
    {
        Ok(value) => OutputState::Parsed(value),
        Err(error) => OutputState::Failed(error),
    }
}

/// Result of the deprecated [`generate_object`]: the AI SDK's
/// `GenerateObjectResult` (`generate-object/generate-object-result.ts`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct GenerateObjectResult {
    /// The generated object, as parsed by the output.
    pub object: Value,
    /// The reasoning text, `None` when there is none.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Why the model call finished.
    pub finish_reason: FinishReasonUnified,
    /// Token usage of the model call.
    pub usage: LanguageModelUsage,
    /// Warnings of the model call.
    #[serde(default)]
    pub warnings: Vec<Warning>,
    /// The request of the model call.
    #[serde(default)]
    pub request: RequestInfo,
    /// The response of the model call, without messages.
    pub response: GenerateObjectResponse,
    /// Provider metadata of the model call.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<ProviderMetadata>,
}

/// `GenerateObjectResult.response`: the AI SDK's
/// `Omit<LanguageModelResponseMetadata, 'messages'>`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct GenerateObjectResponse {
    pub id: String,
    pub timestamp: crate::timestamp::Timestamp,
    pub model_id: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

/// Result of `stream_text` (user-facing).
pub struct StreamTextResult {
    /// The stream of `TextStreamPart` items.
    pub stream: Pin<Box<dyn Stream<Item = Result<TextStreamPart, AiMuxError>> + Send>>,
    /// The request body that was sent (for debugging / cache probing, RFC-0015).
    pub request_body: Option<serde_json::Value>,
    /// Response headers.
    pub response_headers: Option<HashMap<String, String>>,
    call_id: String,
    model: StepModel,
    output: Option<Output>,
}

impl std::fmt::Debug for StreamTextResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamTextResult")
            .field("stream", &"<stream>")
            .field("request_body", &self.request_body)
            .field("response_headers", &self.response_headers)
            .field("call_id", &self.call_id)
            .field("model", &self.model)
            .field("output", &self.output)
            .finish()
    }
}

impl StreamTextResult {
    /// Consume the stream and return the text of the final step.
    ///
    /// # Errors
    ///
    /// The errors of [`consume`](Self::consume).
    pub async fn text(self) -> Result<String, AiMuxError> {
        self.consume().await.map(|result| result.text())
    }

    /// Consume the stream and return the result, as the AI SDK's
    /// `StreamTextResult` promises resolve once the stream has finished.
    ///
    /// Each step is recorded from the parts between its `StartStep` and
    /// `FinishStep` (the event processor of `stream-text.ts:1480-1700`); the
    /// total usage comes from `Finish`.
    ///
    /// # Errors
    ///
    /// The first error part or recoverable stream error, once the stream has
    /// ended; a transport error, timeout or abort error at once. A stream that
    /// recorded no step fails with the abort error when it was aborted and
    /// with `NoOutputGenerated` ("No output generated. Check the stream for
    /// errors.") otherwise (`stream-text.ts:1724-1736`).
    pub async fn consume(self) -> Result<GenerateTextResult, AiMuxError> {
        let Self {
            stream,
            call_id,
            model,
            output,
            ..
        } = self;
        let mut steps: Vec<StepResult> = Vec::new();
        let mut rm = crate::response_messages::ResponseMessageBuilder::new();
        let mut request = RequestInfo::default();
        let mut warnings: Vec<Warning> = Vec::new();
        let mut total_usage: Option<LanguageModelUsage> = None;
        let mut aborted: Option<AiMuxError> = None;
        let mut first_error: Option<AiMuxError> = None;
        let mut stream = stream;
        while let Some(part) = stream.next().await {
            let part = match part {
                Ok(part) => part,
                // A recoverable frame error is data on this stream (core keeps
                // the stream alive past it); fold it into the same first-error
                // slot as an error part and keep consuming so the trailing
                // Finish still lands (usage, recording).
                Err(error) if error.is_recoverable_stream_error() => {
                    first_error.get_or_insert(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            match part {
                TextStreamPart::StartStep {
                    request: step_request,
                    warnings: step_warnings,
                } => {
                    rm = crate::response_messages::ResponseMessageBuilder::new();
                    request = step_request;
                    warnings = step_warnings;
                }
                TextStreamPart::TextStart {
                    id,
                    provider_metadata,
                } => rm.text_start(id, provider_metadata),
                TextStreamPart::TextDelta {
                    id,
                    text,
                    provider_metadata,
                } => rm.text_delta(id, &text, provider_metadata),
                TextStreamPart::TextEnd {
                    id,
                    provider_metadata,
                } => rm.text_end(id, provider_metadata),
                TextStreamPart::ReasoningStart {
                    id,
                    provider_metadata,
                } => rm.reasoning_start(id, provider_metadata),
                TextStreamPart::ReasoningDelta {
                    id,
                    text,
                    provider_metadata,
                } => rm.reasoning_delta(id, &text, provider_metadata),
                TextStreamPart::ReasoningEnd {
                    id,
                    provider_metadata,
                } => rm.reasoning_end(id, provider_metadata),
                TextStreamPart::ToolCall(call) => rm.tool_call(&call),
                // Tool results (not preliminary ones), tool errors and
                // approval responses enter the step content as they arrive
                // (stream-text.ts:1529-1538, :1648-1650).
                TextStreamPart::ToolResult(result) => rm.typed_tool_result(result),
                TextStreamPart::ToolError(error) => rm.tool_error(error),
                TextStreamPart::ToolApprovalResponse(response) => rm.approval_response(response),
                TextStreamPart::Source(source) => rm.source(source),
                TextStreamPart::File(output) => rm.file(output, false),
                TextStreamPart::ReasoningFile(output) => rm.file(output, true),
                TextStreamPart::Custom {
                    kind,
                    provider_metadata,
                } => rm.custom(kind, provider_metadata),
                TextStreamPart::ToolApprovalRequest(approval) => rm.approval(&approval),
                TextStreamPart::FinishStep {
                    response,
                    usage,
                    performance,
                    finish_reason,
                    raw_finish_reason,
                    provider_metadata,
                } => {
                    let assembled = std::mem::take(&mut rm).finish();
                    steps.push(StepResult {
                        call_id: call_id.clone(),
                        step_number: u32::try_from(steps.len()).unwrap_or(u32::MAX),
                        model: model.clone(),
                        content: assembled.content,
                        finish_reason,
                        raw_finish_reason,
                        usage,
                        performance,
                        warnings: std::mem::take(&mut warnings),
                        request: std::mem::take(&mut request),
                        response: StepResponse {
                            id: response.id,
                            timestamp: response.timestamp,
                            model_id: response.model_id,
                            headers: response.headers,
                            body: None,
                            messages: assembled.messages,
                        },
                        provider_metadata,
                    });
                }
                TextStreamPart::Finish {
                    total_usage: usage, ..
                } => {
                    total_usage = Some(usage);
                    break;
                }
                // stream-text.ts:1725-1757: an abort after a recorded step
                // resolves the result (null total usage without `finish`);
                // only a stream that recorded no step fails with the abort
                // (:1729-1740). stream_text itself yields FinishStep and
                // Finish back to back, so this order only comes from a
                // caller-supplied stream.
                TextStreamPart::Abort { reason } => {
                    aborted = Some(AiMuxError::Aborted(
                        reason.unwrap_or_else(|| "request aborted".into()),
                    ));
                    break;
                }
                // Keep consuming after an error part: a trailing Finish still
                // carries usage for the recording layer. The error is
                // returned once the stream reaches its end.
                TextStreamPart::Error { error } => {
                    first_error.get_or_insert(error);
                }
                TextStreamPart::ToolInputStart { .. }
                | TextStreamPart::ToolInputDelta { .. }
                | TextStreamPart::ToolInputEnd { .. }
                | TextStreamPart::ToolOutputDenied(_)
                | TextStreamPart::Start {}
                | TextStreamPart::Raw { .. } => {}
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }
        if steps.is_empty() {
            return Err(aborted.unwrap_or_else(|| {
                AiMuxError::no_output_generated("No output generated. Check the stream for errors.")
            }));
        }
        let output = stream_output(steps.last().expect("checked above"), output.as_ref());
        Ok(GenerateTextResult::with_total_usage(
            steps,
            total_usage.unwrap_or_default(),
            output,
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// User-facing functions
// ─────────────────────────────────────────────────────────────────────────────

/// The provider id the prompt conversion gets: `'openai'` for `'openai.chat'`
/// (upstream passes `model.provider.split('.')[0]`, `ai`
/// `generate-text/generate-text.ts:970` and
/// `generate-text/stream-language-model-call.ts:311`).
fn provider_id(model: &dyn LanguageModel) -> Option<&str> {
    model.provider().split('.').next()
}

/// Generate text (non-streaming).
///
/// # Example
///
/// ```no_run
/// use aimux_core::prelude::*;
///
/// # async fn example(model: &dyn LanguageModel) -> Result<(), AiMuxError> {
/// let result = generate_text(
///     model,
///     "What is Rust?",
///     GenerateTextOptions::default(),
/// ).await?;
///
/// println!("{}", result.text());
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the error produced by the model's `do_generate` call (provider API,
/// network, or abort), and the output's parse error when the final step's
/// text does not parse.
pub async fn generate_text(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<GenerateTextResult, AiMuxError> {
    let settings = options.operation_settings();
    settings
        .clone()
        .scope(generate_in_operation(
            model,
            OperationPrompt::User(prompt.into()),
            options,
            settings,
        ))
        .await
}

// Replay already has a provider prompt. Keep it intact instead of converting
// through user content, which cannot represent every provider part.
pub(crate) async fn generate_text_from_language_model_prompt(
    model: &dyn LanguageModel,
    lm_prompt: LanguageModelPrompt,
    messages: Vec<ModelMessage>,
    options: GenerateTextOptions,
) -> Result<GenerateTextResult, AiMuxError> {
    let settings = options.operation_settings();
    settings
        .clone()
        .scope(generate_in_operation(
            model,
            OperationPrompt::Model {
                prompt: lm_prompt,
                messages,
            },
            options,
            settings,
        ))
        .await
}

/// The prompt of a text operation.
enum OperationPrompt {
    /// A user prompt, prepared inside the operation ([`prepare_prompt`]).
    User(ModelPrompt),
    /// A model prompt sent as it is (replay), with the user messages it was
    /// made from.
    Model {
        prompt: LanguageModelPrompt,
        messages: Vec<ModelMessage>,
    },
}

/// Standardize a user prompt, download the file URLs the model does not
/// fetch itself, and convert it for the model: the AI SDK's
/// `convertToLanguageModelPrompt` with the model's `supportedUrls` and the
/// operation's `experimental_download` (ai@7.0.127
/// `prompt/convert-to-language-model-prompt.ts:36-55`,
/// `generate-text/generate-text.ts:963-971`).
///
/// It runs inside the operation, before the model call: the download
/// observes the caller's abort signal and the operation timeout.
async fn prepare_prompt(
    model: &dyn LanguageModel,
    prompt: ModelPrompt,
    instructions: Option<&str>,
    download: Option<DownloadFunction>,
    abort_signal: Option<&AbortSignal>,
    operation_timeout: timeout::OperationTimeout,
) -> Result<(LanguageModelPrompt, Vec<ModelMessage>), AiMuxError> {
    let (messages, instructions) = standardize_prompt(prompt, instructions)?;
    let supported_urls = model.supported_urls().await;
    let download =
        download.unwrap_or_else(|| create_default_download_function(None, abort_signal.cloned()));
    let assets = timeout::run(
        download_assets(&messages, &download, &supported_urls),
        abort_signal,
        operation_timeout,
    )
    .await?;
    let lm_prompt =
        convert_with_downloaded_assets(&messages, instructions, provider_id(model), &assets)?;
    Ok((lm_prompt, messages))
}

// The operation body runs inside its settings' scope, where the model, the
// composite children and the recorder read them.
async fn generate_in_operation(
    model: &dyn LanguageModel,
    prompt: OperationPrompt,
    options: GenerateTextOptions,
    settings: OperationSettings,
) -> Result<GenerateTextResult, AiMuxError> {
    let repair_tool_call = options.repair_tool_call.clone();
    let tools = options.tools.clone();
    let operation_instructions = options.instructions.clone();
    let output = options.output.clone();
    if let Some(output) = &output {
        output.validate()?;
    }
    let operation_timeout = timeout::OperationTimeout::new(settings.timeout.unwrap_or_default())?;
    let abort_signal = options.abort_signal.clone();

    // 1. Convert the user prompt to the provider-facing prompt.
    let (lm_prompt, messages) = match prompt {
        OperationPrompt::User(prompt) => {
            prepare_prompt(
                model,
                prompt,
                operation_instructions.as_deref(),
                options.download.clone(),
                abort_signal.as_ref(),
                operation_timeout,
            )
            .await?
        }
        OperationPrompt::Model { prompt, messages } => (prompt, messages),
    };

    // 2. Build CallOptions.
    let call_options = options.into_call_options(lm_prompt);
    let retries = retry::prepare_retries(settings.max_retries, abort_signal.clone());

    // 2a. RFC-0023: 关闭时零成本(M2 评审)——仅在录制开启时生成 call_id
    //     并绑定 recorder 快照;传输封闭由层 A 收尾声明(P1 无层 B,barrier
    //     前提;P2 移交层 B)。
    let context = crate::recording::recorder().map(|recorder| {
        let ctx =
            crate::recording::RecordingContext::new(crate::recording::new_call_id(), recorder);
        ctx.recorder.record_input(
            &ctx.call_id,
            &call_options,
            model.provider(),
            model.model_id(),
        );
        ctx.recorder.record_provider(
            &ctx.call_id,
            &crate::recording::ProviderRecord::from_model(model.provider(), model.model_id()),
        );
        // 层 A 早发 closed(defense-in-depth):无 HTTP 的调用(mock/local)也
        // 能完成 barrier;真实 HTTP 的骨架 finalized=false 仍会挡写,由层 B
        // 结束再发 closed 完成。双发幂等。
        ctx.recorder.record_transport_closed(&ctx.call_id);
        ctx
    });
    let call_id = context.as_ref().map(|c| c.call_id.clone());
    let recorder = context.as_ref().map(|c| c.recorder.clone());

    // 2b. Session grouping (RFC-0024): explicit session_id first, opt-in
    //     prompt-prefix inference as fallback. Recorded before the call so
    //     failures are still part of the session. No-op unless a
    //     SessionStore is registered (init_session_store); the resolution
    //     and store lookups are cheap (once-locked env check + two RwLock
    //     reads) when no store / inferer is registered.
    if let (Some((session_id, source)), Some(store)) = (
        crate::session::resolve_session_id(settings.session_id.as_deref(), &call_options.prompt),
        crate::session::session_store(),
    ) {
        let call = store.append(&session_id, call_id.as_deref(), source);
        // RFC-0024 P3: 录制带上会话归组信息(仅录制开启时有效;
        // recorder/call_id 同时存在才可能被写入)。
        if let (Some(recorder), Some(call_id)) = (&recorder, &call_id) {
            recorder.record_session(call_id, &session_id, call.step);
        }
    }

    // 3. Call provider, inside the "generate" span (RFC-0014 §4.1 span tree).
    //    The http layer's `http_request` span nests under this one.
    //    `stepStartTimestampMs` (generate-text.ts:1023) is taken before the
    //    retry loop, so the response time includes retries (:1068).
    let started = Instant::now();
    let span = tracing::info_span!(
        target: "aimux_core::generate",
        "generate",
        provider = %model.provider(),
        model = %model.model_id(),
        modality = "text",
    );
    let operation = crate::recording::RecordingContext::scope(
        context.clone(),
        retries.retry(|| {
            if let Some(context) = &context {
                let _ = context.start_attempt();
            }
            do_generate_with_logging(model, &call_options, span.clone(), started)
        }),
    );
    let result = match timeout::run(operation, abort_signal.as_ref(), operation_timeout).await {
        Ok(r) => r,
        Err(e) => {
            if let (Some(rec), Some(call_id)) = (&recorder, &call_id) {
                rec.record_outcome(call_id, &crate::recording::OutcomeRecord::from_error(&e));
            }
            return Err(e);
        }
    };
    let response_time_ms = ms_between(started, Instant::now());

    // 4. Parse tool calls, build the step and parse the output. Parsing,
    //    repair and file resolution are part of the user operation: the
    //    outcome is recorded once this phase has completed within the same
    //    deadline/cancellation scope as the provider call.
    let step = async {
        let mut tool_calls = Vec::new();
        for content in &result.content {
            if let GenerateContent::ToolCall(call) = content {
                tool_calls.push(
                    parse_tool_call(
                        call.clone(),
                        tools.as_deref(),
                        repair_tool_call.as_ref(),
                        &messages,
                        operation_instructions.as_deref(),
                    )
                    .await,
                );
            }
        }
        if let Some(error) = tool_choice_violation(
            call_options.tool_choice.as_ref(),
            tool_calls.iter().map(|call| call.tool_name.as_str()),
            result.finish_reason.unified,
            model,
            || result.content.clone(),
        ) {
            return Err(error);
        }
        // The step time ends after tool handling and before content
        // conversion (generate-text.ts:1382, :1437-1446).
        let step_time_ms = ms_between(started, Instant::now());
        let content = convert_generated_content(
            &result,
            &tool_calls,
            tools.as_deref(),
            abort_signal.as_ref(),
            operation_timeout,
        )
        .await?;
        Ok::<_, AiMuxError>(build_generated_step(
            model,
            call_id.clone(),
            result,
            content,
            response_time_ms,
            step_time_ms,
        ))
    }
    .await;
    let outcome = step.and_then(|step| {
        let output = generate_output(&step, output.as_ref())?;
        Ok(GenerateTextResult::from_steps(vec![step], output))
    });
    if let (Some(rec), Some(call_id)) = (&recorder, &call_id) {
        rec.record_outcome(
            call_id,
            &match &outcome {
                Ok(result) => crate::recording::OutcomeRecord::success(
                    result.finish_reason(),
                    &result.final_step().usage,
                ),
                Err(error) => crate::recording::OutcomeRecord::from_error(error),
            },
        );
    }
    outcome
}

/// The tool error that follows a client tool call that could not be parsed
/// (`stream-language-model-call.ts:709-725`): the call's input, its error
/// message, and `dynamic`.
pub(crate) fn invalid_tool_call_error(call: &ToolCall) -> Option<TypedToolError> {
    (call.invalid == Some(true) && call.provider_executed != Some(true)).then(|| TypedToolError {
        tool_call_id: call.tool_call_id.clone(),
        tool_name: call.tool_name.clone(),
        input: Some(call.input.clone()),
        error: Value::String(
            call.error
                .as_ref()
                .map_or_else(|| "unknown error".to_owned(), ToString::to_string),
        ),
        provider_executed: None,
        provider_metadata: None,
        dynamic: Some(true),
    })
}

/// A provider's tool result as a stream part
/// (`stream-language-model-call.ts:760-792`): a tool error when the provider
/// marks it as one, otherwise a tool result, with the input of the call it
/// answers (`None` when the call is not in this stream).
///
/// Upstream drops the provider's `preliminary` flag here; aimux keeps it so
/// that, as on the generated path, a preliminary result does not enter the
/// step content.
fn provider_tool_result_part(result: ToolResult, input: Option<Value>) -> TextStreamPart {
    if result.is_error == Some(true) {
        TextStreamPart::ToolError(TypedToolError {
            tool_call_id: result.tool_call_id,
            tool_name: result.tool_name,
            input,
            error: result.result,
            provider_executed: Some(true),
            provider_metadata: result.provider_metadata,
            dynamic: result.dynamic,
        })
    } else {
        TextStreamPart::ToolResult(TypedToolResult {
            tool_call_id: result.tool_call_id,
            tool_name: result.tool_name,
            input,
            output: result.result,
            provider_executed: Some(true),
            provider_metadata: result.provider_metadata,
            dynamic: result.dynamic,
            preliminary: result.preliminary,
        })
    }
}

/// The content of a generated model call with parsed tool calls and
/// resolved files, and its response messages (the AI SDK's
/// `convertLanguageModelContent` and `toResponseMessages`,
/// `generate-text.ts:1437-1455`).
///
/// A provider tool result whose call is not in this response is a deferred
/// result. It is accepted only for a provider tool of that name that sets
/// `supportsDeferredResults`, and otherwise fails with `Tool call {id} not
/// found.` (`convert-language-model-content.ts:87-104`).
async fn convert_generated_content(
    result: &GenerateResult,
    tool_calls: &[ToolCall],
    tools: Option<&[Tool]>,
    abort_signal: Option<&AbortSignal>,
    operation_timeout: timeout::OperationTimeout,
) -> Result<crate::response_messages::ResponseMessages, AiMuxError> {
    let mut rm = crate::response_messages::ResponseMessageBuilder::new();
    for content in &result.content {
        match content {
            GenerateContent::Text {
                text,
                provider_metadata,
            } => rm.text(text, provider_metadata.as_ref()),
            GenerateContent::ToolCall(call) => {
                let parsed = tool_calls
                    .iter()
                    .find(|parsed| parsed.tool_call_id == call.tool_call_id)
                    .expect("every tool call was parsed above");
                rm.tool_call(parsed);
            }
            GenerateContent::Reasoning(reasoning) => rm.reasoning(reasoning),
            GenerateContent::Source(source) => rm.source(source.clone()),
            GenerateContent::File(file) => {
                let output = timeout::run(
                    resolve_file(file.clone(), abort_signal),
                    abort_signal,
                    operation_timeout,
                )
                .await?;
                rm.file(output, false);
            }
            GenerateContent::ReasoningFile(file) => {
                let output = timeout::run(
                    resolve_file(file.clone(), abort_signal),
                    abort_signal,
                    operation_timeout,
                )
                .await?;
                rm.file(output, true);
            }
            GenerateContent::Custom {
                kind,
                provider_metadata,
            } => rm.custom(kind.clone(), provider_metadata.clone()),
            GenerateContent::ToolApprovalRequest(approval) => {
                let call = tool_calls
                    .iter()
                    .find(|call| call.tool_call_id == approval.tool_call_id)
                    .ok_or_else(|| AiMuxError::ToolCallNotFoundForApproval {
                        tool_call_id: approval.tool_call_id.clone(),
                        approval_id: approval.approval_id.clone(),
                    })?;
                rm.approval(&ToolApprovalRequestOutput {
                    approval_id: approval.approval_id.clone(),
                    tool_call: call.clone(),
                    reason: None,
                    is_automatic: None,
                    signature: None,
                    input_schema_input: None,
                });
            }
            GenerateContent::ToolResult(result) => {
                let has_call = tool_calls
                    .iter()
                    .any(|call| call.tool_call_id == result.tool_call_id);
                let deferred_allowed = tools.unwrap_or_default().iter().any(|tool| {
                    matches!(tool, Tool::Provider(tool)
                        if tool.name == result.tool_name
                            && tool.supports_deferred_results == Some(true))
                });
                if !has_call && !deferred_allowed {
                    return Err(AiMuxError::other(format!(
                        "Tool call {} not found.",
                        result.tool_call_id
                    )));
                }
                rm.tool_result(result.clone());
            }
        }
    }
    // A tool call that could not be parsed gets a tool error, after the
    // model's content (`generate-text.ts:1303-1323`,
    // `convert-language-model-content.ts:208-214`).
    for call in tool_calls.iter().filter(|call| call.dynamic == Some(true)) {
        if let Some(error) = invalid_tool_call_error(call) {
            rm.tool_error(error);
        }
    }
    Ok(rm.finish())
}

/// The step of a generated model call (`generate-text.ts:1382-1401`,
/// `:1457-1490`). A generated call has no first-output timing.
fn build_generated_step(
    model: &dyn LanguageModel,
    call_id: Option<String>,
    result: GenerateResult,
    content: crate::response_messages::ResponseMessages,
    response_time_ms: f64,
    step_time_ms: f64,
) -> StepResult {
    let GenerateResult {
        finish_reason,
        usage,
        warnings,
        provider_metadata,
        request,
        response,
        ..
    } = result;
    let response = response.unwrap_or_default();
    let metadata = response_metadata_defaults(
        crate::types::ResponseMetadata {
            id: response.id,
            timestamp: response.timestamp,
            model_id: response.model_id,
        },
        model.model_id(),
    );
    let usage = as_language_model_usage(&usage);
    let performance = StepResultPerformance::new(&usage, response_time_ms, step_time_ms, None, &[]);
    StepResult {
        call_id: call_id.unwrap_or_else(crate::recording::new_call_id),
        step_number: 0,
        model: StepModel {
            provider: model.provider().to_owned(),
            model_id: model.model_id().to_owned(),
        },
        content: content.content,
        finish_reason: finish_reason.unified,
        raw_finish_reason: finish_reason.raw,
        usage,
        performance,
        warnings,
        request: request.unwrap_or_default(),
        response: StepResponse {
            id: metadata.id.unwrap_or_default(),
            timestamp: metadata
                .timestamp
                .unwrap_or_else(crate::timestamp::Timestamp::now),
            model_id: metadata.model_id.unwrap_or_default(),
            headers: response.headers,
            body: response.body,
            messages: content.messages,
        },
        provider_metadata,
    }
}

/// Generate a structured JSON value: a deprecated wrapper over
/// [`generate_text`] with [`GenerateTextOptions::output`], as the AI SDK
/// deprecates `generateObject` for `generateText({ output })`
/// (`generate-object/generate-object.ts:120`).
///
/// `options.output` describes the value; without one the JSON output
/// (`Output::Json`, any JSON value) applies. The object is the result's
/// output, so the model's text must parse and match the schema, with the
/// output's errors.
///
/// # Example
/// ```
/// # #![allow(deprecated)]
/// # use aimux_core::prelude::*;
/// # async fn example(model: &dyn aimux_core::LanguageModel) -> Result<(), aimux_core::error::AiMuxError> {
/// let result = generate_object(
///     model,
///     "Extract: John is 25 years old.",
///     GenerateTextOptions {
///         output: Some(Output::Object {
///             schema: serde_json::json!({
///                 "type": "object",
///                 "properties": { "name": { "type": "string" }, "age": { "type": "number" } },
///                 "required": ["name", "age"]
///             }),
///             name: None,
///             description: None,
///         }),
///         ..Default::default()
///     },
/// ).await?;
/// assert_eq!(result.object["name"], "John");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Propagates `generate_text` failures, including the output's parse
/// errors; `NoOutputGenerated` when the model call finished without a
/// parsed output (for example with tool calls).
#[deprecated(
    note = "use `generate_text` with `GenerateTextOptions::output`, as the AI SDK \
                     deprecates `generateObject` for `generateText({ output })`"
)]
pub async fn generate_object(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    mut options: GenerateTextOptions,
) -> Result<GenerateObjectResult, AiMuxError> {
    options.output.get_or_insert(Output::Json {
        name: None,
        description: None,
    });
    let result = generate_text(model, prompt, options).await?;
    let object = result.output()?.clone();
    let step = result.final_step();
    Ok(GenerateObjectResult {
        object,
        reasoning: step.reasoning_text(),
        finish_reason: step.finish_reason,
        usage: step.usage.clone(),
        warnings: step.warnings.clone(),
        request: step.request.clone(),
        response: GenerateObjectResponse {
            id: step.response.id.clone(),
            timestamp: step.response.timestamp,
            model_id: step.response.model_id.clone(),
            headers: step.response.headers.clone(),
            body: step.response.body.clone(),
        },
        provider_metadata: step.provider_metadata.clone(),
    })
}

/// Stream text from the model.
///
/// # Example
///
/// ```no_run
/// use aimux_core::prelude::*;
/// use futures::StreamExt;
///
/// # async fn example(model: &dyn LanguageModel) -> Result<(), AiMuxError> {
/// let result = stream_text(
///     model,
///     "Write a haiku about Rust.",
///     GenerateTextOptions::default(),
/// ).await?;
///
/// let mut stream = result.stream;
/// while let Some(part) = stream.next().await {
///     if let TextStreamPart::TextDelta { text, .. } = part? {
///         print!("{}", text);
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// The stream is framed as the AI SDK's `fullStream`
/// (`stream-text.ts:1855`, `:2830-2840`, `:3015-3135`): `Start`, then
/// `StartStep` on the step's first chunk, the content parts, `FinishStep` and
/// `Finish`. Aborting ends it with an `Abort` part (`:1860-1882`).
///
/// # Errors
///
/// Returns setup failures from the model's `do_stream` call. Provider error
/// events surface as `Ok(TextStreamPart::Error { .. })`; transport failures
/// and Core timeouts surface as `Err(AiMuxError)` stream items.
pub async fn stream_text(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<StreamTextResult, AiMuxError> {
    let settings = options.operation_settings();
    settings
        .clone()
        .scope(stream_in_operation(model, prompt.into(), options, settings))
        .await
}

// The stream setup runs inside the operation settings' scope, like
// `generate_in_operation`. The pump task that drives the returned stream
// calls no model and does not read them.
async fn stream_in_operation(
    model: &dyn LanguageModel,
    prompt: ModelPrompt,
    options: GenerateTextOptions,
    settings: OperationSettings,
) -> Result<StreamTextResult, AiMuxError> {
    // 1. Convert user prompt to provider-facing prompt.
    let repair_tool_call = options.repair_tool_call.clone();
    let tools = options.tools.clone();
    let operation_instructions = options.instructions.clone();
    let output = options.output.clone();
    if let Some(output) = &output {
        output.validate()?;
    }
    let stream_timeout = settings.timeout.unwrap_or_default();
    let operation_timeout = timeout::OperationTimeout::new(stream_timeout)?;
    let (lm_prompt, messages) = prepare_prompt(
        model,
        prompt,
        operation_instructions.as_deref(),
        options.download.clone(),
        options.abort_signal.as_ref(),
        operation_timeout,
    )
    .await?;

    // 2. Build CallOptions.
    let call_options = options.into_call_options(lm_prompt);
    // Armed before the model call (after the prompt's downloads, which only
    // the total and step deadlines bound): providers await their first SSE
    // event inside do_stream, so the first-chunk budget must already be
    // counting there — a 200-then-silence server is otherwise unbounded when
    // only first_chunk_ms is configured.
    let first_chunk_deadline = stream_timeout
        .first_chunk_ms
        .map(|duration_ms| timeout::TimeoutDeadline::from_now("First chunk", duration_ms))
        .transpose()?;
    let abort_signal = call_options.abort_signal.clone();
    let retries = retry::prepare_retries(settings.max_retries, abort_signal.clone());

    // 2a. RFC-0023: 关闭时零成本(M2 评审)——仅在录制开启时生成 call_id
    //     并绑定 recorder 快照;传输封闭由层 A 收尾声明(P1 无层 B,barrier
    //     前提;P2 移交层 B)。
    let context = crate::recording::recorder().map(|recorder| {
        let ctx =
            crate::recording::RecordingContext::new(crate::recording::new_call_id(), recorder);
        ctx.recorder.record_input(
            &ctx.call_id,
            &call_options,
            model.provider(),
            model.model_id(),
        );
        ctx.recorder.record_provider(
            &ctx.call_id,
            &crate::recording::ProviderRecord::from_model(model.provider(), model.model_id()),
        );
        // 层 A 早发 closed(defense-in-depth):无 HTTP 的调用(mock/local)也
        // 能完成 barrier;真实 HTTP 的骨架 finalized=false 仍会挡写,由层 B
        // 结束再发 closed 完成。双发幂等。
        ctx.recorder.record_transport_closed(&ctx.call_id);
        ctx
    });
    let call_id = context.as_ref().map(|c| c.call_id.clone());
    let recorder = context.as_ref().map(|c| c.recorder.clone());
    let step_call_id = call_id
        .clone()
        .unwrap_or_else(crate::recording::new_call_id);
    let step_model = StepModel {
        provider: model.provider().to_owned(),
        model_id: model.model_id().to_owned(),
    };

    // 2b. Session grouping (RFC-0024): explicit session_id first, opt-in
    //     prompt-prefix inference as fallback. Recorded before the call so
    //     failures are still part of the session. No-op unless a
    //     SessionStore is registered (init_session_store); the resolution
    //     and store lookups are cheap (once-locked env check + two RwLock
    //     reads) when no store / inferer is registered.
    if let (Some((session_id, source)), Some(store)) = (
        crate::session::resolve_session_id(settings.session_id.as_deref(), &call_options.prompt),
        crate::session::session_store(),
    ) {
        let call = store.append(&session_id, call_id.as_deref(), source);
        // RFC-0024 P3: 录制带上会话归组信息(仅录制开启时有效;
        // recorder/call_id 同时存在才可能被写入)。
        if let (Some(recorder), Some(call_id)) = (&recorder, &call_id) {
            recorder.record_session(call_id, &session_id, call.step);
        }
    }

    // 3. Call provider, inside the "generate" span (RFC-0014 §4.1 span tree).
    //    The step starts before the retry loop (`stepStartTimestampMs`,
    //    stream-text.ts:2399); each attempt restarts the model call's clock
    //    (`callStartTimestampMs`, stream-language-model-call.ts:348).
    let span = tracing::info_span!(
        target: "aimux_core::generate",
        "generate",
        provider = %model.provider(),
        model = %model.model_id(),
        modality = "text",
    );
    let step_started = Instant::now();
    let attempt_started = Mutex::new(step_started);
    let operation = crate::recording::RecordingContext::scope(
        context.clone(),
        retries.retry(|| {
            if let Some(context) = &context {
                let _ = context.start_attempt();
            }
            *attempt_started
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
            // Call-level retry boundary: once do_stream returns a stream, retry
            // no longer recreates the provider request; the returned stream is
            // consumed by the stream-step timeout logic below.
            model.do_stream(&call_options).instrument(span.clone())
        }),
    );
    let setup_deadline = timeout::min_deadline(operation_timeout.deadline(), first_chunk_deadline);
    let result: StreamResult =
        match timeout::run_until(operation, abort_signal.as_ref(), setup_deadline).await {
            Ok(r) => r,
            Err(e) => {
                if let (Some(rec), Some(call_id)) = (&recorder, &call_id) {
                    rec.record_outcome(call_id, &crate::recording::OutcomeRecord::from_error(&e));
                }
                return Err(e);
            }
        };
    let call_started = *attempt_started
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // 解构避免部分 move(result 各字段去向不同)。
    let StreamResult {
        stream,
        request,
        response,
    } = result;
    let request_body = request.and_then(|request| request.body);
    let response_headers = response.and_then(|response| response.headers);

    // The consumption phase keeps observing the same deadlines: the
    // first-chunk budget armed at operation start continues until the first
    // output chunk.
    let operation_deadline = operation_timeout.deadline();
    // A pump task drives the provider stream, so the deadlines below measure
    // when provider output *arrives*: a deadline observed only inside
    // `poll_next` keeps elapsing while nobody polls, charging the provider
    // for the consumer's latency. The channel is unbounded so the pump never
    // blocks on the consumer (which would put that latency back into the
    // timers); the AI SDK's `streamText` consumes eagerly for the same
    // reason, and buffering is bounded by the response itself. Each item is
    // stamped with its arrival time, which the step's performance uses.
    //
    // Spawned even with no abort signal or deadlines armed: the pump is also
    // the stream's terminal fuse — `Finish` and non-recoverable errors must
    // end the stream under every configuration, or a provider that keeps the
    // connection open after `Finish` hangs a consumer reading to end-of-stream.
    // The pump runs in its own task, which does not inherit the recording
    // scope; the stream carries it into every poll.
    let mut provider_stream =
        crate::recording::RecordingContext::scope_stream(context.clone(), stream);
    let chunk_ms = stream_timeout.chunk_ms;
    let pump_abort = abort_signal.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let pump = tokio::spawn(async move {
        let mut chunk_deadline = first_chunk_deadline;

        loop {
            let next = tokio::select! {
                biased;
                // Dropping the provider stream cancels the in-flight HTTP
                // exchange; the consumer side reports the abort.
                () = timeout::wait_for_abort(pump_abort.as_ref()) => break,
                () = timeout::wait_for_deadline(operation_deadline) => {
                    let _ = tx.send((Instant::now(), Err(operation_deadline
                        .expect("operation deadline future only resolves for a deadline")
                        .error())));
                    break;
                }
                () = timeout::wait_for_deadline(chunk_deadline) => {
                    let _ = tx.send((Instant::now(), Err(chunk_deadline
                        .expect("chunk deadline future only resolves for a deadline")
                        .error())));
                    break;
                }
                item = provider_stream.next() => item,
            };

            let Some(item) = next else { break };
            let arrived = Instant::now();
            let terminal = is_terminal_item(&item);
            if let Ok(part) = &item
                && is_output_chunk(part)
            {
                chunk_deadline = match chunk_ms
                    .map(|duration_ms| timeout::TimeoutDeadline::from_now("Chunk", duration_ms))
                    .transpose()
                {
                    Ok(deadline) => deadline,
                    Err(error) => {
                        let _ = tx.send((arrived, Err(error)));
                        break;
                    }
                };
            }

            // A closed receiver means the consumer dropped the stream.
            if tx.send((arrived, item)).is_err() || terminal {
                break;
            }
        }
    });
    let pump = AbortOnDrop(pump);

    let file_abort_signal = abort_signal.clone();
    let tool_choice = call_options.tool_choice.clone();
    let step_provider = model.provider().to_owned();
    let step_model_id = model.model_id().to_owned();
    let response_metadata =
        response_metadata_defaults(crate::types::ResponseMetadata::default(), model.model_id());
    let stream_response_headers = response_headers.clone();
    let step_request = RequestInfo {
        body: request_body.clone(),
    };
    let stream: Pin<Box<dyn Stream<Item = Result<TextStreamPart, AiMuxError>> + Send>> = Box::pin(
        async_stream::stream! {
            let _pump = pump;
            let mut step = StreamStep::new(call_started, response_metadata);
            let mut tool_calls: HashMap<String, ToolCall> = HashMap::new();
            let mut content = StreamedContent::default();
            yield Ok(TextStreamPart::Start {});
            loop {
                let next = tokio::select! {
                    biased;
                    () = timeout::wait_for_abort(abort_signal.as_ref()) => {
                        let error = AiMuxError::from_abort_signal(
                            abort_signal.as_ref().expect("abort future only resolves for a signal"),
                        );
                        yield Ok(TextStreamPart::Abort { reason: Some(error.to_string()) });
                        return;
                    }
                    item = rx.recv() => item,
                };
                let Some((arrived, item)) = next else { break };
                let part = match item {
                    Ok(part) => part,
                    Err(error) => {
                        let terminal = !error.is_recoverable_stream_error();
                        yield Err(error);
                        if terminal { return; }
                        continue;
                    }
                };
                // `model-call-start` only carries the warnings; every other
                // chunk first opens the step (stream-text.ts:2822-2840).
                if !matches!(part, StreamPart::StreamStart { .. }) {
                    if !step.started {
                        step.started = true;
                        yield Ok(TextStreamPart::StartStep {
                            request: step_request.clone(),
                            warnings: step.warnings.clone(),
                        });
                    }
                    step.record_chunk(&part, arrived);
                    content.observe(&part);
                }
                match part.split_payload() {
                    StreamPartPayload::ToolCall(raw) => {
                        let parsed = parse_tool_call(
                            raw,
                            tools.as_deref(),
                            repair_tool_call.as_ref(),
                            &messages,
                            operation_instructions.as_deref(),
                        ).await;
                        tool_calls.insert(parsed.tool_call_id.clone(), parsed.clone());
                        let tool_error = invalid_tool_call_error(&parsed);
                        yield Ok(TextStreamPart::ToolCall(parsed));
                        if let Some(error) = tool_error {
                            yield Ok(TextStreamPart::ToolError(error));
                        }
                    }
                    StreamPartPayload::File(file) => {
                        match timeout::run(resolve_file(file, file_abort_signal.as_ref()), file_abort_signal.as_ref(), operation_timeout).await {
                            Ok(output) => yield Ok(TextStreamPart::File(output)),
                            Err(error) => yield Ok(TextStreamPart::Error { error }),
                        }
                    }
                    StreamPartPayload::ToolApprovalRequest(approval) => {
                        if let Some(call) = tool_calls.get(&approval.tool_call_id) {
                            yield Ok(TextStreamPart::ToolApprovalRequest(ToolApprovalRequestOutput {
                                approval_id: approval.approval_id, tool_call: call.clone(),
                                reason: None, is_automatic: None, signature: None, input_schema_input: None,
                            }));
                        } else {
                            yield Ok(TextStreamPart::Error { error: AiMuxError::ToolCallNotFoundForApproval {
                                tool_call_id: approval.tool_call_id, approval_id: approval.approval_id,
                            }});
                        }
                    }
                    StreamPartPayload::ReasoningFile(file) => {
                        match timeout::run(resolve_file(file, file_abort_signal.as_ref()), file_abort_signal.as_ref(), operation_timeout).await {
                            Ok(output) => yield Ok(TextStreamPart::ReasoningFile(output)),
                            Err(error) => yield Ok(TextStreamPart::Error { error }),
                        }
                    }
                    StreamPartPayload::Other(part) => match part {
                        StreamPart::StreamStart { warnings } => step.warnings = warnings,
                        StreamPart::ResponseMetadata(metadata) => step.response_metadata(metadata),
                        // `stream-language-model-call.ts:644-687`: a response that
                        // breaks an enforced tool choice finishes the model call
                        // with an `error` finish reason (the raw reason is kept),
                        // followed by an error part.
                        StreamPart::Finish { mut finish_reason, usage, provider_metadata } => {
                            let violation = tool_choice_violation_error(
                                tool_choice.as_ref(),
                                tool_calls.values().map(|call| call.tool_name.as_str()),
                                finish_reason.unified,
                                &step_provider,
                                &step_model_id,
                                || std::mem::take(&mut content.content),
                            );
                            if violation.is_some() {
                                finish_reason.unified = FinishReasonUnified::Error;
                            }
                            step.finish(finish_reason, &usage, provider_metadata, arrived);
                            if let Some(error) = violation {
                                yield Ok(TextStreamPart::Error { error });
                            }
                        }
                        StreamPart::Error { error } => {
                            step.terminal = true;
                            step.finish_reason = FinishReasonUnified::Error;
                            yield Ok(TextStreamPart::Error { error });
                        }
                        StreamPart::ToolResult(result) => {
                            let input = tool_calls.get(&result.tool_call_id).map(|call| call.input.clone());
                            yield Ok(provider_tool_result_part(result, input));
                        }
                        // An empty text delta without provider metadata is not
                        // forwarded (stream-text.ts:2881-2891); providers emit
                        // one, e.g. OpenAI chat's opening `"content": ""`.
                        StreamPart::TextDelta { delta, provider_metadata: None, .. } if delta.is_empty() => {}
                        part => {
                            if let Some(part) = part
                                .widen::<ToolCall, ToolApprovalRequestOutput, GeneratedFileOutput>()
                                .into_text_stream_part()
                            {
                                yield Ok(part);
                            }
                        }
                    },
                }
            }
            // An incomplete model stream without output is an error, not an
            // empty step; one with partial output keeps it
            // (stream-text.ts:3015-3030).
            if !step.terminal && !step.saw_output {
                yield Ok(TextStreamPart::Error { error: AiMuxError::no_output_generated(
                    "No output generated. The model stream ended without a finish chunk.",
                ) });
                return;
            }
            let (finish_step, finish) = step.into_parts(stream_response_headers, step_started);
            yield Ok(finish_step);
            yield Ok(finish);
        },
    );

    let stream = crate::recording::RecordingOutcomeStream::new(
        stream,
        recorder.clone(),
        call_id.unwrap_or_default(),
    );

    // 4. Return wrapped stream to user (request_body/response_headers kept for
    //    debugging / cache probing, RFC-0015).
    Ok(StreamTextResult {
        stream: Box::pin(stream),
        request_body,
        response_headers,
        call_id: step_call_id,
        model: step_model,
        output,
    })
}

/// The state of the streamed step: what `stream-text.ts:2736-2780` keeps
/// per step, plus the output timing of `stream-language-model-call.ts:441-460`.
struct StreamStep {
    started: bool,
    warnings: Vec<Warning>,
    /// A finish or error chunk arrived.
    terminal: bool,
    saw_output: bool,
    finish_reason: FinishReasonUnified,
    raw_finish_reason: Option<String>,
    usage: LanguageModelUsage,
    provider_metadata: Option<ProviderMetadata>,
    response: crate::types::ResponseMetadata,
    call_started: Instant,
    first_output_ms: Option<f64>,
    previous_output: Option<Instant>,
    chunk_gaps_ms: Vec<f64>,
    /// The model call's performance, known once its finish chunk arrived.
    model_call_performance: Option<StepResultPerformance>,
}

impl StreamStep {
    fn new(call_started: Instant, response: crate::types::ResponseMetadata) -> Self {
        Self {
            started: false,
            warnings: Vec::new(),
            terminal: false,
            saw_output: false,
            finish_reason: FinishReasonUnified::Other,
            raw_finish_reason: None,
            usage: LanguageModelUsage::default(),
            provider_metadata: None,
            response,
            call_started,
            first_output_ms: None,
            previous_output: None,
            chunk_gaps_ms: Vec::new(),
            model_call_performance: None,
        }
    }

    /// Output timing by arrival time (`stream-language-model-call.ts:450-460`).
    fn record_chunk(&mut self, part: &StreamPart, arrived: Instant) {
        if !is_output_chunk(part) {
            return;
        }
        self.saw_output = true;
        if self.first_output_ms.is_none() {
            self.first_output_ms = Some(ms_between(self.call_started, arrived));
        } else if let Some(previous) = self.previous_output {
            self.chunk_gaps_ms.push(ms_between(previous, arrived));
        }
        self.previous_output = Some(arrived);
    }

    /// `model-call-response-metadata` (stream-text.ts:2962-2968).
    fn response_metadata(&mut self, metadata: crate::types::ResponseMetadata) {
        if metadata.id.is_some() {
            self.response.id = metadata.id;
        }
        if metadata.timestamp.is_some() {
            self.response.timestamp = metadata.timestamp;
        }
        if metadata.model_id.is_some() {
            self.response.model_id = metadata.model_id;
        }
    }

    /// The finish chunk: usage and the model call's performance
    /// (`stream-language-model-call.ts:593-624`, stream-text.ts:2971-2983).
    fn finish(
        &mut self,
        finish_reason: crate::types::FinishReason,
        usage: &crate::types::Usage,
        provider_metadata: Option<ProviderMetadata>,
        arrived: Instant,
    ) {
        self.terminal = true;
        self.finish_reason = finish_reason.unified;
        self.raw_finish_reason = finish_reason.raw;
        self.usage = as_language_model_usage(usage);
        self.provider_metadata = provider_metadata;
        self.model_call_performance = Some(StepResultPerformance::new(
            &self.usage,
            ms_between(self.call_started, arrived),
            0.0,
            self.first_output_ms,
            &self.chunk_gaps_ms,
        ));
    }

    /// The `finish-step` and `finish` parts (stream-text.ts:3032-3048,
    /// :3128-3134). Without a finish chunk the model call's performance
    /// stays zero (`createModelCallPerformance`, :2755-2767).
    fn into_parts(
        self,
        headers: Option<SharedHeaders>,
        step_started: Instant,
    ) -> (TextStreamPart, TextStreamPart) {
        let mut performance = self.model_call_performance.unwrap_or_default();
        performance.step_time_ms = ms_between(step_started, Instant::now());
        let total_usage = add_language_model_usage(&LanguageModelUsage::default(), &self.usage);
        (
            TextStreamPart::FinishStep {
                response: StepResponseMetadata {
                    id: self.response.id.unwrap_or_default(),
                    timestamp: self
                        .response
                        .timestamp
                        .unwrap_or_else(crate::timestamp::Timestamp::now),
                    model_id: self.response.model_id.unwrap_or_default(),
                    headers,
                },
                usage: self.usage,
                performance,
                finish_reason: self.finish_reason,
                raw_finish_reason: self.raw_finish_reason.clone(),
                provider_metadata: self.provider_metadata,
            },
            TextStreamPart::Finish {
                finish_reason: self.finish_reason,
                raw_finish_reason: self.raw_finish_reason,
                total_usage,
            },
        )
    }
}

/// The AI SDK's tool choice check after a model call (`generate-text.ts:1155-1176`,
/// `stream-language-model-call.ts:644-662`): a `required` tool choice needs
/// some tool call, a `tool` choice a call to that tool. Every parsed call
/// counts, invalid ones too.
fn tool_choice_violation<'a>(
    tool_choice: Option<&crate::tool::LanguageModelV4ToolChoice>,
    tool_names: impl IntoIterator<Item = &'a str>,
    finish_reason: crate::types::FinishReasonUnified,
    model: &dyn LanguageModel,
    content: impl FnOnce() -> Vec<GenerateContent>,
) -> Option<AiMuxError> {
    tool_choice_violation_error(
        tool_choice,
        tool_names,
        finish_reason,
        model.provider(),
        model.model_id(),
        content,
    )
}

fn tool_choice_violation_error<'a>(
    tool_choice: Option<&crate::tool::LanguageModelV4ToolChoice>,
    tool_names: impl IntoIterator<Item = &'a str>,
    finish_reason: crate::types::FinishReasonUnified,
    provider: &str,
    model_id: &str,
    content: impl FnOnce() -> Vec<GenerateContent>,
) -> Option<AiMuxError> {
    use crate::tool::LanguageModelV4ToolChoice as Choice;
    let tool_choice = tool_choice?;
    let satisfied = match tool_choice {
        Choice::Required => tool_names.into_iter().next().is_some(),
        Choice::Tool { tool_name } => tool_names.into_iter().any(|name| name == tool_name),
        Choice::Auto | Choice::None => return None,
    };
    (!satisfied).then(|| {
        AiMuxError::ToolChoiceViolation(Box::new(crate::error::ToolChoiceViolationError {
            tool_choice: tool_choice.clone(),
            finish_reason,
            provider: provider.to_owned(),
            model_id: model_id.to_owned(),
            content: content(),
        }))
    })
}

/// The provider content of a streamed model call, collected as upstream
/// collects `rawModelCallContent` (`stream-language-model-call.ts:434`): text
/// and reasoning blocks by id (`upsertTextContentPart`, `:892-953`), the
/// other content parts as they arrive (`:583`, `:692`, `:734`, `:761`,
/// `:836-842`).
#[derive(Default)]
struct StreamedContent {
    content: Vec<GenerateContent>,
    text: HashMap<String, usize>,
    reasoning: HashMap<String, usize>,
}

impl StreamedContent {
    fn observe(&mut self, part: &StreamPart) {
        match part {
            StreamPart::TextStart {
                id,
                provider_metadata,
            } => self.upsert_text(id, None, provider_metadata.as_ref()),
            StreamPart::TextDelta {
                id,
                delta,
                provider_metadata,
            } => self.upsert_text(id, Some(delta), provider_metadata.as_ref()),
            StreamPart::TextEnd {
                id,
                provider_metadata,
            } => {
                self.upsert_text(id, None, provider_metadata.as_ref());
                self.text.remove(id);
            }
            StreamPart::ReasoningStart {
                id,
                provider_metadata,
            } => self.upsert_reasoning(id, None, provider_metadata.as_ref()),
            StreamPart::ReasoningDelta {
                id,
                delta,
                provider_metadata,
            } => self.upsert_reasoning(id, Some(delta), provider_metadata.as_ref()),
            StreamPart::ReasoningEnd {
                id,
                provider_metadata,
            } => {
                self.upsert_reasoning(id, None, provider_metadata.as_ref());
                self.reasoning.remove(id);
            }
            StreamPart::File(file) => self.content.push(GenerateContent::File(file.clone())),
            StreamPart::ReasoningFile(file) => self
                .content
                .push(GenerateContent::ReasoningFile(file.clone())),
            StreamPart::ToolCall(call) => {
                self.content.push(GenerateContent::ToolCall(call.clone()))
            }
            StreamPart::ToolApprovalRequest(request) => self
                .content
                .push(GenerateContent::ToolApprovalRequest(request.clone())),
            StreamPart::ToolResult(result) => self
                .content
                .push(GenerateContent::ToolResult(result.clone())),
            StreamPart::Custom {
                kind,
                provider_metadata,
            } => self.content.push(GenerateContent::Custom {
                kind: kind.clone(),
                provider_metadata: provider_metadata.clone(),
            }),
            StreamPart::Source(source) => {
                self.content.push(GenerateContent::Source(source.clone()))
            }
            _ => {}
        }
    }

    fn upsert_text(
        &mut self,
        id: &str,
        delta: Option<&str>,
        provider_metadata: Option<&ProviderMetadata>,
    ) {
        let index = *self.text.entry(id.to_owned()).or_insert_with(|| {
            self.content.push(GenerateContent::Text {
                text: String::new(),
                provider_metadata: provider_metadata.cloned(),
            });
            self.content.len() - 1
        });
        if let GenerateContent::Text {
            text,
            provider_metadata: metadata,
        } = &mut self.content[index]
        {
            text.push_str(delta.unwrap_or_default());
            if provider_metadata.is_some() {
                metadata.clone_from(&provider_metadata.cloned());
            }
        }
    }

    fn upsert_reasoning(
        &mut self,
        id: &str,
        delta: Option<&str>,
        provider_metadata: Option<&ProviderMetadata>,
    ) {
        let index = *self.reasoning.entry(id.to_owned()).or_insert_with(|| {
            self.content
                .push(GenerateContent::Reasoning(crate::result::ReasoningOutput {
                    text: String::new(),
                    provider_metadata: provider_metadata.cloned(),
                }));
            self.content.len() - 1
        });
        if let GenerateContent::Reasoning(reasoning) = &mut self.content[index] {
            reasoning.text.push_str(delta.unwrap_or_default());
            if provider_metadata.is_some() {
                reasoning.provider_metadata = provider_metadata.cloned();
            }
        }
    }
}

#[cfg(test)]
mod pipeline_tests;
#[cfg(test)]
mod tests;

/// A malformed individual frame does not prove that the transport is dead:
/// SSE framing lets a later event still be valid. `Finish` and transport/Core
/// errors are terminal so the stream is not polled past a dead source.
fn is_terminal_item(item: &Result<StreamPart, AiMuxError>) -> bool {
    matches!(item, Ok(StreamPart::Finish { .. }))
        || matches!(item, Err(error) if !error.is_recoverable_stream_error())
}

/// Aborts the stream pump task when the returned stream is dropped, so an
/// abandoned stream leaves no task driving the provider connection.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
/// Run `do_generate` inside the RFC-0014 `generate` span and emit the
/// `generate_end` event. A plain async fn (rather than an inline async block)
/// so the `?` error type is pinned by the declared return type.
async fn do_generate_with_logging(
    model: &dyn LanguageModel,
    call_options: &CallOptions,
    span: tracing::Span,
    started: Instant,
) -> Result<GenerateResult, AiMuxError> {
    let r = async { model.do_generate(call_options).await }
        .instrument(span)
        .await?;
    tracing::info!(
        target: "aimux_core::generate",
        ok = true,
        duration_ms = started.elapsed().as_millis() as u64,
        finish_reason = ?r.finish_reason.unified,
        "generate_end"
    );
    Ok(r)
}

/// Split a `ModelPrompt` into messages + optional instructions.
///
/// If the prompt is a plain string, it becomes a single user message.
/// If the prompt is messages, they are used as-is.
/// Instructions are passed through separately (they get prepended by
/// `convert_to_language_model_prompt`).
/// The AI SDK's `standardizePrompt` (`ai` `prompt/standardize-prompt.ts:34-112`)
/// for the prompt forms aimux takes.
///
/// - A text prompt becomes one user message (`:67-68`).
/// - An empty message list is rejected: `messages must not be empty`
///   (`:78-83`).
/// - `prompt` and `messages` cannot both be given and the messages match
///   the `ModelMessage` schema by construction (`ModelPrompt` is one or the
///   other, and parts are checked by role when they are parsed).
///
/// Not ported yet: upstream rejects system messages in the messages unless
/// `allowSystemInMessages` is set (`:85-96`), and accepts system messages as
/// `instructions` (`:55-65`). The bindings, CLI and web tool still send
/// system messages in the messages, and `instructions` is a string here.
///
/// # Errors
///
/// `InvalidPrompt` when there are no messages.
pub(crate) fn standardize_prompt(
    prompt: ModelPrompt,
    instructions: Option<&str>,
) -> Result<(Vec<ModelMessage>, Option<&str>), AiMuxError> {
    let (messages, instructions) = split_prompt(prompt, instructions);
    if messages.is_empty() {
        return Err(AiMuxError::invalid_prompt("messages must not be empty"));
    }
    Ok((messages, instructions))
}

pub(crate) fn split_prompt(
    prompt: ModelPrompt,
    instructions: Option<&str>,
) -> (Vec<ModelMessage>, Option<&str>) {
    match prompt {
        ModelPrompt::Text(text) => (vec![ModelMessage::user(text)], instructions),
        ModelPrompt::Messages(msgs) => (msgs, instructions),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OpenAI Chat Completions output (RFC-0026)
// ─────────────────────────────────────────────────────────────────────────────

use crate::openai_output::{
    ChatCompletion, ChatCompletionStream, OpenAiStreamOptions, to_chat_completion_stream,
};

/// Generate text and return the result as an OpenAI Chat Completion.
///
/// This is the OpenAI-compatible equivalent of [`generate_text`]. Internally
/// it calls `generate_text`, then converts the result with
/// [`generate_text_result_to_chat_completion`]. Tool calls reflect Core
/// parsing and any configured repair.
///
/// Works with **any** provider (OpenAI, Anthropic, Google, …) — the output is
/// always standard OpenAI Chat Completions JSON.
///
/// # Example
///
/// ```no_run
/// use aimux_core::prelude::*;
///
/// # async fn example(model: &dyn LanguageModel) -> Result<(), AiMuxError> {
/// let completion = generate_text_as_openai(
///     model,
///     "What is Rust?",
///     GenerateTextOptions::default(),
/// ).await?;
///
/// println!("{}", completion.choices[0].message.content.as_deref().unwrap());
/// println!("{}", completion.usage.prompt_tokens);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Propagates any error from the underlying `generate_text` call.
pub async fn generate_text_as_openai(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<ChatCompletion, AiMuxError> {
    let result = generate_text(model, prompt, options).await?;
    Ok(generate_text_result_to_chat_completion(
        &result,
        model.model_id(),
    ))
}

/// Convert a [`GenerateTextResult`] into an OpenAI [`ChatCompletion`].
///
/// The pure half of [`generate_text_as_openai`]: the message is built from
/// the content of the result's steps, so tool calls reflect Core parsing and
/// any repair already applied to the result; the finish reason, id, model and
/// logprobs come from the last step and the usage is the result's total.
/// Hosts that repair a result themselves (RFC-0035) convert it with this
/// afterwards.
///
/// `model_id` is the fallback for `ChatCompletion.model` when the response
/// carries no model id.
#[must_use]
pub fn generate_text_result_to_chat_completion(
    result: &GenerateTextResult,
    model_id: &str,
) -> ChatCompletion {
    crate::openai_output::text_result_to_chat_completion(result, model_id)
}

/// Stream text and return the result as a stream of OpenAI Chat Completion chunks.
///
/// This is the OpenAI-compatible equivalent of [`stream_text`]. Internally
/// it calls `stream_text`, then converts the `StreamPart` stream into a
/// `ChatCompletionChunk` stream via [`to_chat_completion_stream`].
///
/// Works with **any** provider (OpenAI, Anthropic, Google, …) — the output is
/// always standard OpenAI Chat Completions streaming chunks.
///
/// Tool-call argument deltas are the provider's text, forwarded verbatim as it
/// arrives, so this stream does **not** reflect tool-call repair — aligned
/// with the AI SDK, which likewise never withholds input deltas. Use
/// [`generate_text_as_openai`] or the native `stream_text` when repaired calls
/// are required.
///
/// # Example
///
/// ```no_run
/// use aimux_core::prelude::*;
/// use futures::StreamExt;
///
/// # async fn example(model: &dyn LanguageModel) -> Result<(), AiMuxError> {
/// let result = stream_text_as_openai(
///     model,
///     "Write a haiku about Rust.",
///     GenerateTextOptions::default(),
///     OpenAiStreamOptions::default(),
/// ).await?;
///
/// let mut stream = result.stream;
/// while let Some(chunk) = stream.next().await {
///     let chunk = chunk?;
///     if let Some(delta) = chunk.choices.first()
///         && let Some(content) = &delta.delta.content {
///         print!("{}", content);
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Propagates any error from the underlying `stream_text` call.
pub async fn stream_text_as_openai(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
    stream_options: OpenAiStreamOptions,
) -> Result<ChatCompletionStream, AiMuxError> {
    let result = stream_text(model, prompt, options).await?;
    Ok(to_chat_completion_stream(
        result.stream,
        model.model_id(),
        stream_options,
    ))
}
