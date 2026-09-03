//! User-facing API: `generate_text` and `stream_text`.
//!
//! These are the primary entry points for users. They:
//! 1. Convert user prompt (`ModelPrompt`) to `LanguageModelPrompt` (provider-facing).
//! 2. Build `CallOptions` from user-facing options.
//! 3. Call `LanguageModel::do_generate` / `do_stream`.
//! 4. Extract / wrap the result for the user.

use std::collections::HashMap;
use std::pin::Pin;
use std::time::Instant;

use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::Instrument;
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::language_model::LanguageModel;
use crate::language_model_message::convert_to_language_model_prompt;
use crate::message::{ModelMessage, ModelPrompt};
use crate::options::{CallOptions, ResponseFormat, ToolChoice};
use crate::parse_tool_call::{RawToolCall, ToolCallRepair, parse_tool_call};
use crate::result::{
    FilePart, GenerateContent, GenerateResult, ReasoningPart, SourcePart, StreamResult,
    StreamTextResultAggregated,
};
use crate::stream_part::StreamPart;
use crate::tool::Tool;
use crate::types::{FinishReason, ReasoningEffort, Usage, Warning};

// ─────────────────────────────────────────────────────────────────────────────
// User-facing options
// ─────────────────────────────────────────────────────────────────────────────

/// User-facing options for `generate_text` and `stream_text`.
///
/// Unlike `CallOptions` (provider-facing), this does not include `prompt`
/// (passed separately) and defaults are more ergonomic.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GenerateTextOptions {
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub stop_sequences: Option<Vec<String>>,
    pub top_p: Option<f64>,
    pub top_k: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub response_format: Option<ResponseFormat>,
    #[ts(type = "number | null")]
    pub seed: Option<u64>,
    pub tools: Option<Vec<Tool>>,
    pub tool_choice: Option<ToolChoice>,
    pub headers: Option<HashMap<String, String>>,
    pub provider_options: Option<HashMap<String, Value>>,
    /// Top-level reasoning effort.
    pub reasoning: Option<ReasoningEffort>,
    /// System instructions prepended to the prompt.
    pub instructions: Option<String>,
    /// Per-call request body overrides (deep-merged). See RFC-0017.
    pub body_overrides: Option<Value>,
    /// Per-call retry count override. `None` = provider default, `Some(0)` = disable.
    pub max_retries: Option<u32>,
    /// Per-call timeout configuration (total / first-chunk / chunk idle).
    pub timeout: Option<crate::options::TimeoutConfiguration>,
    /// Session identifier, for grouping consecutive calls into a session
    /// (observability, see RFC-0024). Orthogonal to RFC-0019 session-affinity
    /// headers. When `None` and the optional session inferer is enabled, one
    /// may be inferred from prompt-prefix continuation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Abort signal for cancelling the call.
    ///
    /// Runtime handle — never crosses the JSON boundary. Rust callers set it
    /// directly; the Node binding bridges a JS `AbortSignal` natively.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<crate::shared::AbortSignal>,
    /// Emit raw provider stream chunks as `StreamPart::Raw` (debugging aid).
    pub include_raw_chunks: Option<bool>,
    /// Optional one-shot repair callback for unknown, malformed, or
    /// schema-invalid tool calls.
    #[serde(skip)]
    #[ts(skip)]
    pub repair_tool_call: Option<ToolCallRepair>,
}

impl GenerateTextOptions {
    /// Build the provider-facing `CallOptions` from user-facing options.
    #[must_use]
    pub fn into_call_options(
        self,
        prompt: crate::language_model_message::LanguageModelPrompt,
    ) -> CallOptions {
        CallOptions {
            prompt,
            max_output_tokens: self.max_output_tokens,
            temperature: self.temperature,
            stop_sequences: self.stop_sequences,
            top_p: self.top_p,
            top_k: self.top_k,
            presence_penalty: self.presence_penalty,
            frequency_penalty: self.frequency_penalty,
            response_format: self.response_format,
            seed: self.seed,
            tools: self.tools,
            tool_choice: self.tool_choice.unwrap_or_default(),
            headers: self.headers,
            provider_options: self.provider_options,
            reasoning: self.reasoning,
            body_overrides: self.body_overrides,
            max_retries: self.max_retries,
            timeout: self.timeout,
            session_id: self.session_id,
            abort_signal: self.abort_signal,
            call_id: None,
            recording_context: None,
            include_raw_chunks: self.include_raw_chunks,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// User-facing result types
// ─────────────────────────────────────────────────────────────────────────────

/// Result of `generate_text` (user-facing).
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GenerateTextResult {
    /// The generated text (concatenated from all text content parts).
    pub text: String,
    /// Tool calls requested by the model.
    pub tool_calls: Vec<crate::tool::ToolCall>,
    /// Why generation stopped.
    pub finish_reason: FinishReason,
    /// Token usage.
    pub usage: Usage,
    /// Warnings from the provider.
    pub warnings: Vec<Warning>,
    /// Raw provider result (for advanced use).
    pub raw: GenerateResult,
    // ── M7: top-level aggregation (extracted from `raw.content`) ──
    /// Reasoning / thinking segments from the model.
    #[serde(default)]
    pub reasoning: Vec<ReasoningPart>,
    /// Concatenated reasoning text (convenience for `reasoning.iter().map(text).join("")`).
    #[serde(default)]
    pub reasoning_text: String,
    /// Sources / citations (search-preview models).
    #[serde(default)]
    pub sources: Vec<SourcePart>,
    /// Files generated by the model.
    #[serde(default)]
    pub files: Vec<FilePart>,
    /// Assistant messages ready to append to the prompt for the next turn
    /// (solves the multi-turn "manually build assistant message" footgun).
    #[serde(default)]
    pub response_messages: Vec<ModelMessage>,
    /// The raw provider-specific finish reason string (e.g. "stop",
    /// "end_turn", "safety"). Useful when `finish_reason.unified` is `Other`.
    #[serde(default)]
    pub raw_finish_reason: Option<String>,
    /// Provider-specific metadata (e.g. Anthropic cache info). Mirrored from
    /// `raw.provider_metadata` for top-level convenience.
    #[serde(default)]
    pub provider_metadata: Option<Value>,
    /// Response metadata (id, timestamp, model_id). Mirrored from
    /// `raw.response` for top-level convenience.
    #[serde(default)]
    pub response: crate::types::ResponseMetadata,
    /// Total token usage across all steps. In single-step mode (aimux's
    /// default), `total_usage` equals `usage`. Provided for AI SDK parity.
    #[serde(default)]
    pub total_usage: Usage,
}

/// Result of `generate_object` (user-facing, M12). The parsed JSON object plus
/// convenience fields from the underlying `generate_text` call.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GenerateObjectResult {
    /// The parsed JSON object returned by the model.
    pub object: Value,
    /// Why generation stopped.
    pub finish_reason: FinishReason,
    /// Raw provider-specific finish reason string.
    #[serde(default)]
    pub raw_finish_reason: Option<String>,
    /// Token usage.
    pub usage: Usage,
    /// Warnings from the provider.
    pub warnings: Vec<Warning>,
    /// Concatenated reasoning text (if the model produced reasoning/thinking).
    #[serde(default)]
    pub reasoning: Option<String>,
    /// Provider-specific metadata (e.g. Anthropic cache info).
    #[serde(default)]
    pub provider_metadata: Option<Value>,
    /// Response metadata (id, timestamp, model_id).
    #[serde(default)]
    pub response: crate::types::ResponseMetadata,
    /// The full `generate_text` result (for advanced use).
    pub raw: GenerateTextResult,
}

/// Result of `stream_text` (user-facing).
pub struct StreamTextResult {
    /// The stream of `StreamPart` items.
    pub stream: Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>>,
    /// The request body that was sent (for debugging / cache probing, RFC-0015).
    pub request_body: Option<serde_json::Value>,
    /// Response headers.
    pub response_headers: Option<HashMap<String, String>>,
}

impl std::fmt::Debug for StreamTextResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamTextResult")
            .field("stream", &"<stream>")
            .field("request_body", &self.request_body)
            .field("response_headers", &self.response_headers)
            .finish()
    }
}

impl StreamTextResult {
    /// Consume the stream and collect all text deltas into a single `String`.
    ///
    /// # Errors
    ///
    /// Returns the stream's error part or transport error, propagated from the
    /// underlying model stream.
    pub async fn text(self) -> Result<String, AiMuxError> {
        use futures::StreamExt;
        let mut result = String::new();
        let mut stream = self.stream;
        while let Some(part) = stream.next().await {
            match part? {
                StreamPart::TextDelta { delta, .. } => result.push_str(&delta),
                StreamPart::Finish { .. } => break,
                StreamPart::Error { error } => return Err(error),
                _ => {}
            }
        }
        Ok(result)
    }

    /// Consume the full stream and return an aggregated result (M11).
    ///
    /// Unlike [`text`](Self::text) (which returns only the concatenated text),
    /// this collects reasoning, tool calls, sources, files, usage, finish
    /// reason, and builds `response_messages` for the next turn.
    ///
    /// # Errors
    ///
    /// Returns the stream's error part or transport error, propagated from the
    /// underlying model stream.
    pub async fn consume(self) -> Result<StreamTextResultAggregated, AiMuxError> {
        use futures::StreamExt;

        let mut text = String::new();
        let mut tool_calls: Vec<crate::tool::ToolCall> = Vec::new();
        let mut sources: Vec<SourcePart> = Vec::new();
        let mut files: Vec<FilePart> = Vec::new();
        let mut warnings: Vec<Warning> = Vec::new();
        let mut finish_reason = FinishReason {
            unified: crate::types::FinishReasonUnified::Stop,
            raw: None,
        };
        let mut raw_finish_reason: Option<String> = None;
        let mut usage = Usage::default();
        let mut finish_provider_metadata: Option<Value> = None;
        let mut response: Option<crate::types::ResponseMetadata> = None;
        let mut rm = crate::response_messages::ResponseMessageBuilder::new();

        let mut stream = self.stream;
        while let Some(part) = stream.next().await {
            match part? {
                StreamPart::TextStart {
                    provider_metadata, ..
                } => rm.text_start(provider_metadata),
                StreamPart::TextDelta {
                    delta,
                    provider_metadata,
                    ..
                } => {
                    text.push_str(&delta);
                    rm.text_delta(&delta, provider_metadata);
                }
                StreamPart::TextEnd {
                    provider_metadata, ..
                } => rm.text_end(provider_metadata),
                StreamPart::ReasoningStart {
                    provider_metadata, ..
                } => rm.reasoning_start(provider_metadata),
                StreamPart::ReasoningDelta {
                    delta,
                    provider_metadata,
                    ..
                } => rm.reasoning_delta(&delta, provider_metadata),
                StreamPart::ReasoningEnd {
                    provider_metadata, ..
                } => rm.reasoning_end(provider_metadata),
                StreamPart::ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    provider_executed,
                    dynamic,
                    thought_signature,
                    provider_metadata,
                    invalid,
                    error,
                    ..
                } => {
                    let call = crate::tool::ToolCall {
                        tool_call_id,
                        tool_name,
                        input,
                        provider_executed,
                        dynamic,
                        thought_signature,
                        provider_metadata,
                        invalid,
                        error,
                    };
                    rm.tool_call(&call);
                    tool_calls.push(call);
                }
                StreamPart::ToolResult {
                    tool_call_id,
                    tool_name,
                    result,
                    is_error,
                    preliminary,
                    dynamic,
                    provider_metadata,
                } => rm.tool_result(
                    tool_call_id,
                    tool_name,
                    result,
                    is_error,
                    preliminary,
                    dynamic,
                    provider_metadata,
                ),
                StreamPart::Source {
                    id,
                    source_type,
                    url,
                    title,
                    ..
                } => {
                    sources.push(SourcePart {
                        id,
                        source_type,
                        url,
                        title,
                    });
                }
                StreamPart::File {
                    data, media_type, ..
                } => {
                    files.push(FilePart { data, media_type });
                }
                StreamPart::StreamStart { warnings: w, .. } => {
                    warnings = w;
                }
                StreamPart::Finish {
                    finish_reason: fr,
                    usage: u,
                    provider_metadata: pm,
                } => {
                    raw_finish_reason = fr.raw.clone();
                    finish_reason = fr;
                    usage = u.clone();
                    finish_provider_metadata = pm;
                    break;
                }
                StreamPart::Error { error } => return Err(error),
                StreamPart::ResponseMetadata {
                    id,
                    timestamp,
                    model_id,
                } => {
                    response = Some(crate::types::ResponseMetadata {
                        id,
                        timestamp,
                        model_id,
                    });
                }
                _ => {}
            }
        }

        let crate::response_messages::ResponseMessages {
            messages: response_messages,
            reasoning,
        } = rm.finish();

        let reasoning_text = reasoning
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("");

        Ok(StreamTextResultAggregated {
            text,
            reasoning,
            reasoning_text,
            tool_calls,
            sources,
            files,
            finish_reason,
            raw_finish_reason,
            total_usage: usage.clone(),
            usage,
            warnings,
            provider_metadata: finish_provider_metadata,
            response,
            response_messages,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// User-facing functions
// ─────────────────────────────────────────────────────────────────────────────

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
/// println!("{}", result.text);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the error produced by the model's `do_generate` call (provider API,
/// network, or abort).
pub async fn generate_text(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<GenerateTextResult, AiMuxError> {
    // 1. Convert user prompt to provider-facing prompt.
    let repair_tool_call = options.repair_tool_call.clone();
    let tools = options.tools.clone();
    let operation_instructions = options.instructions.clone();
    let (messages, instructions) = split_prompt(prompt.into(), operation_instructions.as_deref());
    let lm_prompt = convert_to_language_model_prompt(&messages, instructions);

    // 2. Build CallOptions.
    let mut call_options = options.into_call_options(lm_prompt);

    // 2a. RFC-0023: 关闭时零成本(M2 评审)——仅在录制开启时生成 call_id
    //     并绑定 recorder 快照;传输封闭由层 A 收尾声明(P1 无层 B,barrier
    //     前提;P2 移交层 B)。
    let context = crate::recording::recorder().map(|recorder| {
        let call_id = crate::recording::new_call_id();
        call_options.call_id = Some(call_id.clone());
        let ctx = crate::recording::RecordingContext {
            call_id: call_id.clone(),
            recorder,
        };
        call_options.recording_context = Some(ctx.clone());
        ctx.recorder.record_input(
            &ctx.call_id,
            &call_options,
            model.provider(),
            model.model_id(),
        );
        ctx.recorder
            .record_provider(&ctx.call_id, &model.config_snapshot());
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
        crate::session::resolve_session_id(
            call_options.session_id.as_deref(),
            &call_options.prompt,
        ),
        crate::session::session_store(),
    ) {
        let call = store.append(&session_id, call_options.call_id.as_deref(), source);
        // RFC-0024 P3: 录制带上会话归组信息(仅录制开启时有效;
        // recorder/call_id 同时存在才可能被写入)。
        if let (Some(recorder), Some(call_id)) = (&recorder, &call_id) {
            recorder.record_session(call_id, &session_id, call.step);
        }
    }

    // 3. Call provider, inside the "generate" span (RFC-0014 §4.1 span tree).
    //    The http layer's `http_request` span nests under this one.
    let started = Instant::now();
    let span = tracing::info_span!(
        target: "aimux_core::generate",
        "generate",
        provider = %model.provider(),
        model = %model.model_id(),
        modality = "text",
    );
    let result = match do_generate_with_logging(model, &call_options, span, started).await {
        Ok(r) => r,
        Err(e) => {
            if let (Some(rec), Some(call_id)) = (&recorder, &call_id) {
                rec.record_outcome(call_id, &crate::recording::OutcomeRecord::from_error(&e));
            }
            return Err(e);
        }
    };
    // 4. Extract text, tool calls, reasoning, sources, files from content.
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut sources = Vec::new();
    let mut files = Vec::new();
    // Build the assistant response message content parts in parallel.
    let mut rm = crate::response_messages::ResponseMessageBuilder::new();
    for content in &result.content {
        match content {
            GenerateContent::Text {
                text: t,
                provider_metadata,
            } => {
                text.push_str(t);
                rm.text(t, provider_metadata.as_ref());
            }
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                provider_executed,
                dynamic,
                thought_signature,
                provider_metadata,
                ..
            } => {
                let parsed = parse_tool_call(
                    RawToolCall {
                        tool_call_id: tool_call_id.clone(),
                        tool_name: tool_name.clone(),
                        input: input.clone(),
                        provider_executed: *provider_executed,
                        dynamic: *dynamic,
                        thought_signature: thought_signature.clone(),
                        provider_metadata: provider_metadata.clone(),
                    },
                    tools.as_deref(),
                    repair_tool_call.as_ref(),
                    &messages,
                    operation_instructions.as_deref(),
                )
                .await;
                rm.tool_call(&parsed);
                tool_calls.push(parsed);
            }
            GenerateContent::Reasoning {
                text: rtext,
                provider_metadata,
            } => {
                rm.reasoning(rtext, provider_metadata.as_ref());
            }
            GenerateContent::Source {
                id,
                source_type,
                url,
                title,
                ..
            } => {
                sources.push(SourcePart {
                    id: id.clone(),
                    source_type: source_type.clone(),
                    url: url.clone(),
                    title: title.clone(),
                });
            }
            GenerateContent::File {
                data, media_type, ..
            } => {
                files.push(FilePart {
                    data: data.clone(),
                    media_type: media_type.clone(),
                });
            }
            // Provider-executed results stay in the assistant message so the
            // provider can replay its own server-tool transcript next turn.
            GenerateContent::ToolResult {
                tool_call_id,
                tool_name,
                result,
                is_error,
                preliminary,
                dynamic,
                provider_metadata,
            } => {
                rm.tool_result(
                    tool_call_id.clone(),
                    tool_name.clone(),
                    result.clone(),
                    *is_error,
                    *preliminary,
                    *dynamic,
                    provider_metadata.clone(),
                );
            }
        }
    }

    let crate::response_messages::ResponseMessages {
        messages: response_messages,
        reasoning,
    } = rm.finish();
    let reasoning_text = reasoning
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("");

    // Parsing and optional repair are part of the user operation. Record a
    // successful outcome only after that phase has completed within the same
    // deadline/cancellation scope as the provider call.
    if let (Some(rec), Some(call_id)) = (&recorder, &call_id) {
        rec.record_outcome(
            call_id,
            &crate::recording::OutcomeRecord::from_generate_result(&result),
        );
    }

    // Extract fields before moving `result` into `raw`.
    let raw_finish_reason = result.finish_reason.raw.clone();
    let provider_metadata = result.provider_metadata.clone();
    let response = result.response.clone();
    let usage = result.usage.clone();

    Ok(GenerateTextResult {
        text,
        tool_calls,
        finish_reason: result.finish_reason.clone(),
        usage: usage.clone(),
        warnings: result.warnings.clone(),
        raw: result,
        reasoning,
        reasoning_text,
        sources,
        files,
        response_messages,
        raw_finish_reason,
        provider_metadata,
        response,
        total_usage: usage,
    })
}

/// Generate a structured JSON object (M12). Uses `generate_text` with
/// `response_format: Json`, then parses the model output into a JSON value.
///
/// The schema is passed via `options.response_format = ResponseFormat::Json {
/// schema: Some(schema), .. }`. JSON repair (`fix_json`) is applied before
/// parsing to handle truncated or slightly malformed model output. No schema
/// validation is performed (weak validation — relies on the model following
/// the schema; use retries for robustness).
///
/// # Example
/// ```
/// # use aimux_core::prelude::*;
/// # use aimux_core::options::ResponseFormat;
/// # async fn example(model: &dyn aimux_core::LanguageModel) -> Result<(), aimux_core::error::AiMuxError> {
/// let result = generate_object(
///     model,
///     "Extract: John is 25 years old.",
///     GenerateTextOptions {
///         response_format: Some(ResponseFormat::Json {
///             schema: Some(serde_json::json!({
///                 "type": "object",
///                 "properties": { "name": { "type": "string" }, "age": { "type": "number" } },
///                 "required": ["name", "age"]
///             })),
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
/// Propagates `generate_text` failures; additionally returns `JsonParse` when
/// the (repaired) model output is not valid JSON.
pub async fn generate_object(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<GenerateObjectResult, AiMuxError> {
    let text_result = generate_text(model, prompt, options).await?;
    let repaired = crate::json_repair::fix_json(&text_result.text);
    let object: Value = serde_json::from_str(&repaired).map_err(|e| {
        AiMuxError::JsonParse(format!(
            "generateObject: model output is not valid JSON after repair: {e}"
        ))
    })?;
    let raw_finish_reason = text_result.raw_finish_reason.clone();
    let finish_reason = text_result.finish_reason.clone();
    let usage = text_result.usage.clone();
    let warnings = text_result.warnings.clone();
    let reasoning = if text_result.reasoning_text.is_empty() {
        None
    } else {
        Some(text_result.reasoning_text.clone())
    };
    let provider_metadata = text_result.raw.provider_metadata.clone();
    let response = text_result.raw.response.clone();
    Ok(GenerateObjectResult {
        object,
        finish_reason,
        raw_finish_reason,
        usage,
        warnings,
        reasoning,
        provider_metadata,
        response,
        raw: text_result,
    })
}

/// Extract the reasoning signature from provider metadata, checking all known
/// provider keys (Anthropic, Bedrock).
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
///     if let StreamPart::TextDelta { delta, .. } = part? {
///         print!("{}", delta);
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the error produced by the model's `do_stream` call (provider API,
/// network, or abort); errors raised mid-stream surface as `StreamPart::Error`
/// items instead.
pub async fn stream_text(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<StreamTextResult, AiMuxError> {
    // 1. Convert user prompt to provider-facing prompt.
    let repair_tool_call = options.repair_tool_call.clone();
    let tools = options.tools.clone();
    let operation_instructions = options.instructions.clone();
    let (messages, instructions) = split_prompt(prompt.into(), operation_instructions.as_deref());
    let lm_prompt = convert_to_language_model_prompt(&messages, instructions);

    // 2. Build CallOptions.
    let mut call_options = options.into_call_options(lm_prompt);

    // 2a. RFC-0023: 关闭时零成本(M2 评审)——仅在录制开启时生成 call_id
    //     并绑定 recorder 快照;传输封闭由层 A 收尾声明(P1 无层 B,barrier
    //     前提;P2 移交层 B)。
    let context = crate::recording::recorder().map(|recorder| {
        let call_id = crate::recording::new_call_id();
        call_options.call_id = Some(call_id.clone());
        let ctx = crate::recording::RecordingContext {
            call_id: call_id.clone(),
            recorder,
        };
        call_options.recording_context = Some(ctx.clone());
        ctx.recorder.record_input(
            &ctx.call_id,
            &call_options,
            model.provider(),
            model.model_id(),
        );
        ctx.recorder
            .record_provider(&ctx.call_id, &model.config_snapshot());
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
        crate::session::resolve_session_id(
            call_options.session_id.as_deref(),
            &call_options.prompt,
        ),
        crate::session::session_store(),
    ) {
        let call = store.append(&session_id, call_options.call_id.as_deref(), source);
        // RFC-0024 P3: 录制带上会话归组信息(仅录制开启时有效;
        // recorder/call_id 同时存在才可能被写入)。
        if let (Some(recorder), Some(call_id)) = (&recorder, &call_id) {
            recorder.record_session(call_id, &session_id, call.step);
        }
    }

    // 3. Call provider, inside the "generate" span (RFC-0014 §4.1 span tree).
    let span = tracing::info_span!(
        target: "aimux_core::generate",
        "generate",
        provider = %model.provider(),
        model = %model.model_id(),
        modality = "text",
    );
    let result: StreamResult = match model.do_stream(&call_options).instrument(span).await {
        Ok(r) => r,
        Err(e) => {
            if let (Some(rec), Some(call_id)) = (&recorder, &call_id) {
                rec.record_outcome(call_id, &crate::recording::OutcomeRecord::from_error(&e));
            }
            return Err(e);
        }
    };
    // 解构避免部分 move(result 各字段去向不同)。
    let StreamResult {
        stream,
        request_body,
        response_headers,
    } = result;
    let mut stream = stream;
    let stream: Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>> =
        Box::pin(async_stream::stream! {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(StreamPart::ToolCall {
                        tool_call_id,
                        tool_name,
                        input,
                        provider_executed,
                        dynamic,
                        thought_signature,
                        provider_metadata,
                        ..
                    }) => {
                        let parsed = parse_tool_call(
                            RawToolCall {
                                tool_call_id,
                                tool_name,
                                input: crate::parse_tool_call::raw_tool_input(&input),
                                provider_executed,
                                dynamic,
                                thought_signature,
                                provider_metadata,
                            },
                            tools.as_deref(),
                            repair_tool_call.as_ref(),
                            &messages,
                            operation_instructions.as_deref(),
                        ).await;
                        yield Ok(StreamPart::ToolCall {
                            tool_call_id: parsed.tool_call_id,
                            tool_name: parsed.tool_name,
                            input: parsed.input,
                            provider_executed: parsed.provider_executed,
                            dynamic: parsed.dynamic,
                            thought_signature: parsed.thought_signature,
                            invalid: parsed.invalid,
                            error: parsed.error,
                            provider_metadata: parsed.provider_metadata,
                        });
                    }
                    item => yield item,
                }
            }
        });
    // 录制开启时才包装(终结时写 outcome + 传输封闭);关闭时零成本透传。
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
    })
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
fn split_prompt(
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
    ChatCompletion, ChatCompletionFunction, ChatCompletionStream, ChatCompletionToolCall,
    OpenAiStreamOptions, parsed_tool_call_arguments, to_chat_completion, to_chat_completion_stream,
    to_chat_completion_stream_with_deferred_tool_calls,
};

/// Generate text and return the result as an OpenAI Chat Completion.
///
/// This is the OpenAI-compatible equivalent of [`generate_text`]. Internally
/// it calls `generate_text`, then converts the [`GenerateResult`] into a
/// [`ChatCompletion`] via [`to_chat_completion`].
/// Tool calls reflect Core parsing and any configured repair; provider
/// response metadata, usage, and non-tool content are preserved from `raw`.
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
    let mut completion = to_chat_completion(&result.raw, model.model_id());
    if let Some(choice) = completion.choices.first_mut() {
        choice.message.tool_calls = if result.tool_calls.is_empty() {
            None
        } else {
            Some(
                result
                    .tool_calls
                    .iter()
                    .map(|tool_call| ChatCompletionToolCall {
                        id: tool_call.tool_call_id.clone(),
                        tool_type: "function".to_string(),
                        function: ChatCompletionFunction {
                            name: tool_call.tool_name.clone(),
                            arguments: parsed_tool_call_arguments(
                                &tool_call.input,
                                tool_call.invalid,
                                tool_call.error.as_ref(),
                            ),
                        },
                    })
                    .collect(),
            )
        };
    }
    Ok(completion)
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
    // `parse_tool_call` only invokes repair when a tool set was supplied.
    let defer_tool_calls = options.repair_tool_call.is_some() && options.tools.is_some();
    let result = stream_text(model, prompt, options).await?;
    if defer_tool_calls {
        Ok(to_chat_completion_stream_with_deferred_tool_calls(
            result.stream,
            model.model_id(),
            stream_options,
        ))
    } else {
        Ok(to_chat_completion_stream(
            result.stream,
            model.model_id(),
            stream_options,
        ))
    }
}
