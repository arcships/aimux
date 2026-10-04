//! The response shapes the chat model reads.
//!
//! Mirrors the `groqChatResponseSchema` and `groqChatChunkSchema` of
//! `groq-chat-language-model.ts`, limited to what the implementation needs.

use serde::{Deserialize, Serialize};

// ── Usage ───────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct GroqUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PromptTokensDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CompletionTokensDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

// ── Non-streaming response ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChatResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    pub choices: Vec<GroqChatChoice>,
    #[serde(default)]
    pub usage: Option<GroqUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChatChoice {
    pub message: GroqChatMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChatMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<GroqToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqToolCall {
    #[serde(default)]
    pub id: Option<String>,
    pub function: GroqToolCallFunction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqToolCallFunction {
    pub name: String,
    pub arguments: String,
}

// ── Streaming response ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChatChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    pub choices: Vec<GroqChunkChoice>,
    #[serde(default)]
    pub x_groq: Option<XGroq>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChunkChoice {
    #[serde(default)]
    pub delta: Option<GroqChunkDelta>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChunkDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<GroqChunkToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChunkToolCall {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub r#type: Option<String>,
    pub function: GroqChunkToolCallFunction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChunkToolCallFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct XGroq {
    #[serde(default)]
    pub usage: Option<GroqUsage>,
}
