//! Wire types of the OpenAI-compatible chat-completions API.
//!
//! Deliberately loose: compatible servers omit fields the OpenAI API always
//! sends (`usage`, tool-call ids), so everything but the essentials is
//! optional, as in the AI SDK's `z.looseObject` schemas.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Non-streaming response ──

#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<UsageResponse>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Choice {
    pub message: MessageResponse,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageResponse {
    #[serde(default)]
    pub content: Option<Value>,
    /// Reasoning text (`reasoning`, Groq and others).
    #[serde(default)]
    pub reasoning: Option<String>,
    /// Reasoning text (`reasoning_content`, DeepSeek and others); wins over
    /// `reasoning` when both are present.
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
    /// Annotations (URL citations), kept as raw JSON.
    #[serde(default)]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolCallResponse {
    #[serde(default)]
    pub id: Option<String>,
    pub function: FunctionCallResponse,
    /// Google-hosted compatible endpoints carry the thought signature here.
    #[serde(default)]
    pub extra_content: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FunctionCallResponse {
    pub name: String,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub(crate) struct UsageResponse {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    /// Top-level `cached_tokens` (Moonshot); wins over the nested value.
    #[serde(default)]
    pub cached_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub(crate) struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
    /// Alibaba (DashScope) reports cache writes as `cache_creation_input_tokens`.
    #[serde(default, alias = "cache_creation_input_tokens")]
    pub cache_write_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub(crate) struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
    #[serde(default)]
    pub accepted_prediction_tokens: Option<u32>,
    #[serde(default)]
    pub rejected_prediction_tokens: Option<u32>,
}

// ── Streaming response ──

#[derive(Debug, Deserialize)]
pub(crate) struct StreamChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct StreamChoice {
    #[serde(default)]
    pub delta: Delta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct Delta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
    #[serde(default)]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeltaToolCall {
    /// Some compatible providers omit it (`index: z.number().nullish()`); the
    /// tracker falls back to id / name.
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<DeltaFunction>,
    #[serde(default)]
    pub extra_content: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct DeltaFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}
