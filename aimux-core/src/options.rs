//! `CallOptions` — the provider-facing options passed to `do_generate` / `do_stream`.
//!
//! Aligned with V4 `LanguageModelV4CallOptions`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::language_model_message::LanguageModelPrompt;
pub use crate::tool::{FunctionTool, ProviderTool, Tool, ToolChoice};
use crate::shared::AbortSignal;
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
    pub total_ms: Option<u64>,
    /// Timeout waiting for the first stream chunk (streaming only).
    pub first_chunk_ms: Option<u64>,
    /// Maximum idle time between consecutive stream chunks (streaming only).
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
    /// `RetryConfig.max_retries`. `Some(0)` disables retries.
    pub max_retries: Option<u32>,

    /// Per-call timeout configuration (total / first-chunk / chunk idle).
    /// `None` = no timeouts (provider defaults still apply at the HTTP layer).
    pub timeout: Option<TimeoutConfiguration>,

    /// Abort signal for cancelling the call.
    ///
    /// Runtime handle — never crosses the JSON boundary (bindings that only
    /// pass JSON cannot set it; Node bridges a JS `AbortSignal` natively).
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,
}

impl CallOptions {
    /// Create a `CallOptions` with the given prompt and all other fields set
    /// to their defaults (`None` / `ToolChoice::Auto`).
    ///
    /// This is the recommended construction baseline in tests — combined with
    /// struct-update syntax (`CallOptions { tools: Some(..), ..CallOptions::new(prompt) }`)
    /// it avoids spelling out every `None` field, so adding a new optional
    /// field to `CallOptions` no longer requires batch-editing every test.
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
            abort_signal: None,
        }
    }
}
