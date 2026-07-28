//! xAI API response types.

#![allow(dead_code)]

use serde::Deserialize;

// ── Non-streaming response ──

#[derive(Debug, Deserialize)]
pub struct XaiChatResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub choices: Option<Vec<XaiChoice>>,
    #[serde(default)]
    pub usage: Option<XaiUsageResponse>,
    /// Citations (URLs) returned when search is enabled.
    #[serde(default)]
    pub citations: Option<Vec<String>>,
    /// Error code — present when xAI returns an error with HTTP 200.
    #[serde(default)]
    pub code: Option<String>,
    /// Error message — present when xAI returns an error with HTTP 200.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct XaiChoice {
    pub message: XaiMessageResponse,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct XaiMessageResponse {
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    /// Reasoning / thinking content produced by the model.
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<XaiToolCallResponse>>,
}

#[derive(Debug, Deserialize)]
pub struct XaiToolCallResponse {
    pub id: String,
    pub function: XaiFunctionCallResponse,
}

#[derive(Debug, Deserialize)]
pub struct XaiFunctionCallResponse {
    pub name: String,
    pub arguments: String,
}

// ── Usage ──

#[derive(Debug, Deserialize, Clone, Default)]
pub struct XaiUsageResponse {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_tokens_details: Option<XaiPromptTokensDetails>,
    #[serde(default)]
    pub completion_tokens_details: Option<XaiCompletionTokensDetails>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct XaiPromptTokensDetails {
    #[serde(default)]
    pub text_tokens: Option<u32>,
    #[serde(default)]
    pub audio_tokens: Option<u32>,
    #[serde(default)]
    pub image_tokens: Option<u32>,
    #[serde(default)]
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct XaiCompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
    #[serde(default)]
    pub audio_tokens: Option<u32>,
    #[serde(default)]
    pub accepted_prediction_tokens: Option<u32>,
    #[serde(default)]
    pub rejected_prediction_tokens: Option<u32>,
}

// ── Streaming response ──

#[derive(Debug, Deserialize)]
pub struct XaiStreamChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub choices: Vec<XaiStreamChoice>,
    #[serde(default)]
    pub usage: Option<XaiUsageResponse>,
    #[serde(default)]
    pub citations: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct XaiStreamChoice {
    #[serde(default)]
    pub delta: XaiDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub index: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct XaiDelta {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<XaiDeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct XaiDeltaToolCall {
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<XaiDeltaFunction>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct XaiDeltaFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

// Suppress unused warning for Value import.
// (No longer needed — Value import removed.)
