//! `CallOptions` — the provider-facing options passed to `do_generate` / `do_stream`.
//!
//! Aligned with V4 `LanguageModelV4CallOptions`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::AbortSignal;
use crate::language_model_message::LanguageModelPrompt;
use crate::shared::SharedProviderOptions;
pub use crate::tool::{
    FunctionTool, FunctionToolInputExample, LanguageModelV4ToolChoice, ProviderTool, Tool,
    ToolChoice, prepare_tool_choice,
};
use crate::types::ReasoningEffort;

/// How the model should format its response.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ResponseFormat {
    /// Plain text (default).
    Text,
    /// JSON output, optionally constrained to a schema.
    Json {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

/// Per-call timeout configuration.
///
/// The object form of the AI SDK's `TimeoutConfiguration`
/// (`ai/src/prompt/request-options.ts:13-22`), a request option of
/// `generateText` / `streamText` (`RequestOptions.timeout`, `:126`).
/// `LanguageModelV4CallOptions` has no timeout. Aimux runs no tools, so
/// upstream's `toolMs` / `tools` keys and the bare-number form of `totalMs`
/// are absent.
///
/// All values are milliseconds; `None` disables the corresponding limit. A
/// `total` timeout also covers retry backoff and the whole streamed response.
///
/// Unknown keys, upstream's `toolMs` / `tools` among them, are rejected
/// (RFC-0042 §7.2): aimux 0.5's `total_ms` is an error naming it, not a call
/// without a timeout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TimeoutConfiguration {
    /// Overall timeout for the entire call (including retries and, for
    /// streaming, the whole stream), in milliseconds.
    // `number`, not the `bigint` ts-rs infers from u64: the JS bindings pass
    // options through `JSON.stringify`, which throws on BigInt. Milliseconds
    // cannot reach 2^53 (~285k years), so precision is never at stake.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
    /// Timeout for one generation step, including that step's attempts and
    /// retry backoff, in milliseconds. Aimux currently has one step.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_ms: Option<u64>,
    /// Timeout waiting for the first stream chunk (streaming only).
    ///
    /// Counted from operation start, so it also bounds stream establishment
    /// and any retries before the first semantic output: it is the
    /// user-perceived time-to-first-output budget, not a per-attempt timer.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_chunk_ms: Option<u64>,
    /// Maximum idle time between consecutive stream chunks (streaming only).
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_ms: Option<u64>,
}

/// Options passed to `LanguageModel::do_generate` / `do_stream`.
///
/// This is the **provider-facing** options struct. Users interact with
/// `GenerateTextOptions` (user-facing) which is converted to `CallOptions`
/// by the `generate_text` / `stream_text` functions.
///
/// The retry count, the timeout and the session id are settings of the
/// operation (`OperationSettings`), not call options, as in the AI SDK
/// (`language-model-v4-call-options.ts:8-138` has none of them).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CallOptions {
    /// The standardized prompt (message array). Required.
    pub prompt: LanguageModelPrompt,

    /// Maximum tokens to generate.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,

    /// Sampling temperature.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Stop sequences.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,

    /// Nucleus sampling `top_p`.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// Top-k sampling.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<f64>,

    /// Presence penalty.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,

    /// Frequency penalty.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,

    /// Response format (text or JSON).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,

    /// Seed for reproducibility.
    #[ts(optional, type = "number")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,

    /// Tools available to the model (function tools and/or provider-defined tools).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,

    /// How the model should choose tools. `generate_text` / `stream_text`
    /// always set it through [`prepare_tool_choice`], as the AI SDK does.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<LanguageModelV4ToolChoice>,

    /// Extra HTTP headers.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Provider-specific options (keyed by provider name).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,

    /// Top-level reasoning effort. Maps to OpenAI `reasoning_effort` and
    /// Anthropic `thinking` config.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningEffort>,

    /// Abort signal for cancelling the call.
    ///
    /// Runtime handle — never crosses the JSON boundary (bindings that only
    /// pass JSON cannot set it; Node bridges a JS `AbortSignal` natively).
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// RFC-0023 recording context (R7 快照绑定;internal;never serialized)。
    /// 层 A 入口构造一次,provider 复制到 `HttpRequest`。None = 不录制。
    #[serde(skip)]
    #[ts(skip)]
    pub recording_context: Option<crate::recording::RecordingContext>,

    /// Emit raw provider stream chunks as `StreamPart::Raw` (the AI SDK's
    /// `includeRawChunks`). When `Some(true)`, a streaming model yields one
    /// `Raw` part per SSE event, carrying the event's parsed JSON, before the
    /// parts made from it; the `[DONE]` sentinel is excluded. A chunk that is
    /// not JSON yields `Raw` with `null` in Groq and Anthropic, as the AI SDK
    /// does, and only `Error` elsewhere. `None` / `Some(false)` = off.
    /// Honored by OpenAI chat, OpenAI-compatible chat and completion (registry
    /// presets and Azure included), Groq, DeepSeek, Anthropic Messages, and
    /// the OpenAI and ElevenLabs transcription streams; the other model
    /// families do not emit `Raw` yet.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_raw_chunks: Option<bool>,
}

impl CallOptions {
    /// Create a `CallOptions` with the given prompt and all other fields set
    /// to `None`.
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
            tool_choice: None,
            headers: None,
            provider_options: None,
            reasoning: None,
            abort_signal: None,
            recording_context: None,
            include_raw_chunks: None,
        }
    }
}
