//! OpenAI API request/response types.

#![allow(dead_code)]

use serde::Deserialize;
use serde_json::Value;

// ── Non-streaming response ──

#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub model: String,
    #[serde(default)]
    pub created: Option<u64>,
    pub choices: Vec<Choice>,
    pub usage: UsageResponse,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: MessageResponse,
    #[serde(default)]
    pub finish_reason: Option<String>,
    /// Logprobs returned when `logprobs` is requested.
    #[serde(default)]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct MessageResponse {
    pub role: String,
    pub content: Option<String>,
    /// Reasoning text (Groq / OpenAI o-series models).
    #[serde(default)]
    pub reasoning: Option<String>,
    /// DeepSeek / 阿里通义等厂商的 reasoning_content 字段。
    /// 优先于 `reasoning`（当两者同时存在时）。
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
    /// Annotations (e.g. URL citations) — stored as raw JSON.
    #[serde(default)]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub struct ToolCallResponse {
    pub id: String,
    pub function: FunctionCallResponse,
}

#[derive(Debug, Deserialize)]
pub struct FunctionCallResponse {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UsageResponse {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    /// Top-level `cached_tokens` — returned by some OpenAI-compatible providers
    /// (notably Moonshot AI) instead of the nested `prompt_tokens_details.cached_tokens`.
    /// When present it takes precedence over the nested value, matching the
    /// Moonshot usage converter.
    #[serde(default)]
    pub cached_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
    /// Cache-write token count. Alibaba (DashScope) reports this under the
    /// field name `cache_creation_input_tokens`; the alias makes both wire
    /// shapes parse into the same field.
    #[serde(default, alias = "cache_creation_input_tokens")]
    pub cache_write_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
    #[serde(default)]
    pub accepted_prediction_tokens: Option<u32>,
    #[serde(default)]
    pub rejected_prediction_tokens: Option<u32>,
}

// ── Streaming response ──

#[derive(Debug, Deserialize)]
pub struct StreamChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<UsageResponse>,
    /// Groq-specific extension: usage is sent in `x_groq.usage` during streaming.
    #[serde(default)]
    pub x_groq: Option<XGroq>,
}

/// Groq streaming extension (`x_groq` field in SSE chunks).
#[derive(Debug, Deserialize, Default)]
pub struct XGroq {
    #[serde(default)]
    pub usage: Option<UsageResponse>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamChoice {
    #[serde(default)]
    pub delta: Delta,
    #[serde(default)]
    pub finish_reason: Option<String>,
    /// Logprobs returned when `logprobs` is requested.
    #[serde(default)]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Delta {
    #[serde(default)]
    pub content: Option<String>,
    /// Reasoning text delta (Groq / OpenAI o-series).
    #[serde(default)]
    pub reasoning: Option<String>,
    /// DeepSeek / 阿里通义等厂商的 reasoning_content delta。
    /// 优先于 `reasoning`。
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct DeltaToolCall {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<DeltaFunction>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DeltaFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

// Suppress unused warning for Value import (used in convert.rs).
#[allow(unused_imports)]
use serde_json::Value as _Value;
