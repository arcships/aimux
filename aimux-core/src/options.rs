//! `CallOptions` — the provider-facing options passed to `do_generate` / `do_stream`.
//!
//! Aligned with V4 `LanguageModelV4CallOptions`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::AbortSignal;
use crate::language_model_message::LanguageModelPrompt;
pub use crate::tool::{FunctionTool, ProviderTool, Tool, ToolChoice};
use crate::types::ReasoningEffort;

/// How the model should format its response.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ResponseFormat {
    /// Plain text (default).
    Text,
    /// JSON output, optionally constrained to a schema.
    Json {
        schema: Option<Value>,
        name: Option<String>,
        description: Option<String>,
    },
}

/// Per-call timeout configuration.
///
/// Aligned with V4 `LanguageModelV4CallOptions.timeout`
/// (`TimeoutConfiguration`). All values are milliseconds; `None` disables the
/// corresponding limit. A `total` timeout also covers retry backoff and the
/// whole streamed response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimeoutConfiguration {
    /// Overall timeout for the entire call (including retries and, for
    /// streaming, the whole stream), in milliseconds.
    // `number`, not the `bigint` ts-rs infers from u64: the JS bindings pass
    // options through `JSON.stringify`, which throws on BigInt. Milliseconds
    // cannot reach 2^53 (~285k years), so precision is never at stake.
    #[ts(type = "number | null")]
    pub total_ms: Option<u64>,
    /// Timeout for one generation step, including that step's attempts and
    /// retry backoff, in milliseconds. Aimux currently has one step.
    #[ts(type = "number | null")]
    pub step_ms: Option<u64>,
    /// Timeout waiting for the first stream chunk (streaming only).
    ///
    /// Counted from operation start, so it also bounds stream establishment
    /// and any retries before the first semantic output: it is the
    /// user-perceived time-to-first-output budget, not a per-attempt timer.
    #[ts(type = "number | null")]
    pub first_chunk_ms: Option<u64>,
    /// Maximum idle time between consecutive stream chunks (streaming only).
    #[ts(type = "number | null")]
    pub chunk_ms: Option<u64>,
}

/// Options passed to `LanguageModel::do_generate` / `do_stream`.
///
/// This is the **provider-facing** options struct. Users interact with
/// `GenerateTextOptions` (user-facing) which is converted to `CallOptions`
/// by the `generate_text` / `stream_text` functions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallOptions {
    /// The standardized prompt (message array). Required.
    pub prompt: LanguageModelPrompt,

    /// Maximum tokens to generate.
    pub max_output_tokens: Option<u32>,

    /// Sampling temperature.
    pub temperature: Option<f64>,

    /// Stop sequences.
    pub stop_sequences: Option<Vec<String>>,

    /// Nucleus sampling `top_p`.
    pub top_p: Option<f64>,

    /// Top-k sampling.
    pub top_k: Option<f64>,

    /// Presence penalty.
    pub presence_penalty: Option<f64>,

    /// Frequency penalty.
    pub frequency_penalty: Option<f64>,

    /// Response format (text or JSON).
    pub response_format: Option<ResponseFormat>,

    /// Seed for reproducibility.
    #[ts(type = "number | null")]
    pub seed: Option<u64>,

    /// Tools available to the model (function tools and/or provider-defined tools).
    pub tools: Option<Vec<Tool>>,

    /// How the model should choose tools.
    pub tool_choice: ToolChoice,

    /// Extra HTTP headers.
    pub headers: Option<HashMap<String, String>>,

    /// Provider-specific options (keyed by provider name).
    pub provider_options: Option<HashMap<String, Value>>,

    /// Top-level reasoning effort. Maps to OpenAI `reasoning_effort` and
    /// Anthropic `thinking` config.
    pub reasoning: Option<ReasoningEffort>,

    /// Per-call request body overrides. Deep-merged into the provider-built
    /// request body (after any built-in vendor override) before sending.
    /// `null` values delete the corresponding key. See RFC-0017.
    pub body_overrides: Option<Value>,

    /// Per-call retry count override. `None` uses the provider's configured
    /// Core operation retry. `Some(0)` disables retries.
    pub max_retries: Option<u32>,

    /// Per-call timeout configuration (total / first-chunk / chunk idle).
    /// `None` = no timeouts (provider defaults still apply at the HTTP layer).
    pub timeout: Option<TimeoutConfiguration>,

    /// Session identifier, for grouping consecutive calls into a session
    /// (observability, see RFC-0024). Explicit values take precedence; when
    /// `None` and the optional session inferer is enabled, one may be inferred.
    /// Orthogonal to RFC-0019 session-affinity headers: this field is for
    /// local grouping, while session headers in `headers` are for upstream
    /// routing — both may share an id value but travel different paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Abort signal for cancelling the call.
    ///
    /// Runtime handle — never crosses the JSON boundary (bindings that only
    /// pass JSON cannot set it; Node bridges a JS `AbortSignal` natively).
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// RFC-0023 recording correlation id (internal; never serialized).
    #[serde(skip)]
    #[ts(skip)]
    pub call_id: Option<String>,

    /// RFC-0023 recording context (R7 快照绑定;internal;never serialized)。
    /// 层 A 入口构造一次,provider 复制到 `HttpRequest`。None = 不录制。
    #[serde(skip)]
    #[ts(skip)]
    pub recording_context: Option<crate::recording::RecordingContext>,

    /// Emit raw provider stream chunks as `StreamPart::Raw` (debugging aid).
    /// When `Some(true)`, streaming providers yield one `Raw` part per JSON
    /// SSE event, carrying the **parsed JSON payload** of the event, emitted
    /// before the parsed parts. Excludes the `[DONE]` sentinel; unparsable
    /// chunks emit only `Error`. `null`/`Some(false)` = off.
    /// Currently honored by the OpenAI-compatible family (openai / azure /
    /// openai-compatible registry providers); other provider families ignore
    /// it for now (RFC-0016 M2).
    pub include_raw_chunks: Option<bool>,
}

impl CallOptions {
    /// Clone these options for one composite child step (Router child, MoA
    /// reference or aggregator): the recording context, when present, is
    /// replaced by a child context labeled `step`, so the child's exchanges
    /// and retry attempts are recorded as their own step instead of blending
    /// into the parent operation's.
    #[must_use]
    pub fn for_step(&self, step: impl Into<String>) -> Self {
        let mut child = self.clone();
        child.recording_context = self
            .recording_context
            .as_ref()
            .map(|context| context.child(step.into()));
        child
    }

    /// Create a `CallOptions` with the given prompt and all other fields set
    /// to their defaults (`None` / `ToolChoice::Auto`).
    ///
    /// This is the recommended construction baseline in tests — combined with
    /// struct-update syntax (`CallOptions { tools: Some(..), ..CallOptions::new(prompt) }`)
    /// it avoids spelling out every `None` field, so adding a new optional
    /// field to `CallOptions` no longer requires batch-editing every test.
    #[must_use]
    pub fn new(prompt: LanguageModelPrompt) -> Self {
        CallOptions {
            prompt,
            max_output_tokens: None,
            temperature: None,
            stop_sequences: None,
            top_p: None,
            top_k: None,
            presence_penalty: None,
            frequency_penalty: None,
            response_format: None,
            seed: None,
            tools: None,
            tool_choice: ToolChoice::default(),
            headers: None,
            provider_options: None,
            reasoning: None,
            body_overrides: None,
            max_retries: None,
            timeout: None,
            session_id: None,
            abort_signal: None,
            call_id: None,
            recording_context: None,
            include_raw_chunks: None,
        }
    }
}
