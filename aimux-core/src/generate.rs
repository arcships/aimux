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

use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::Instrument;
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::language_model::LanguageModel;
use crate::language_model_message::convert_to_language_model_prompt;
use crate::message::{ModelMessage, ModelPrompt};
use crate::options::{CallOptions, ResponseFormat, ToolChoice};
use crate::result::{GenerateContent, GenerateResult, StreamResult};
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
}

impl GenerateTextOptions {
    /// Build the provider-facing `CallOptions` from user-facing options.
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
pub async fn generate_text(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<GenerateTextResult, AiMuxError> {
    // 1. Convert user prompt to provider-facing prompt.
    let (messages, instructions) = split_prompt(prompt.into(), options.instructions.as_deref());
    let lm_prompt = convert_to_language_model_prompt(&messages, instructions);

    // 2. Build CallOptions.
    let call_options = options.into_call_options(lm_prompt);

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
        store.append(&session_id, source);
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
    let result = do_generate_with_logging(model, &call_options, span, started).await?;

    // 4. Extract text and tool calls from content.
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for content in &result.content {
        match content {
            GenerateContent::Text { text: t, .. } => text.push_str(t),
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                thought_signature,
                ..
            } => {
                tool_calls.push(crate::tool::ToolCall {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    input: input.clone(),
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: thought_signature.clone(),
                });
            }
            // Sources / citations are not text or tool calls; skip them here.
            GenerateContent::Source { .. } => {}
            // Reasoning / thinking is not text or tool calls; skip it here.
            GenerateContent::Reasoning { .. } => {}
            // Files generated by the model; skip here (not text or tool calls).
            GenerateContent::File { .. } => {}
            // Provider-executed tool results; skip here.
            GenerateContent::ToolResult { .. } => {}
        }
    }

    Ok(GenerateTextResult {
        text,
        tool_calls,
        finish_reason: result.finish_reason.clone(),
        usage: result.usage.clone(),
        warnings: result.warnings.clone(),
        raw: result,
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
///     if let StreamPart::TextDelta { delta, .. } = part? {
///         print!("{}", delta);
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub async fn stream_text(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<StreamTextResult, AiMuxError> {
    // 1. Convert user prompt to provider-facing prompt.
    let (messages, instructions) = split_prompt(prompt.into(), options.instructions.as_deref());
    let lm_prompt = convert_to_language_model_prompt(&messages, instructions);

    // 2. Build CallOptions.
    let call_options = options.into_call_options(lm_prompt);

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
        store.append(&session_id, source);
    }

    // 3. Call provider, inside the "generate" span (RFC-0014 §4.1 span tree).
    let span = tracing::info_span!(
        target: "aimux_core::generate",
        "generate",
        provider = %model.provider(),
        model = %model.model_id(),
        modality = "text",
    );
    let result: StreamResult = model.do_stream(&call_options).instrument(span).await?;

    // 4. Return stream to user (request_body/response_headers kept for
    //    debugging / cache probing, RFC-0015).
    Ok(StreamTextResult {
        stream: result.stream,
        request_body: result.request_body,
        response_headers: result.response_headers,
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
