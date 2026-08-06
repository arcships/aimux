//! Mistral API request/response types.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Non-streaming response ──

#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
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
}

#[derive(Debug, Deserialize)]
pub struct MessageResponse {
    pub role: String,
    /// Content can be a string (legacy) or an array of typed parts
    /// ({type:"text",text}, {type:"thinking",thinking:[...]}).
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
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

#[derive(Debug, Deserialize, Default, Serialize)]
pub struct UsageResponse {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    #[serde(default)]
    pub num_cached_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    // Mistral has used both spellings at different times.
    #[serde(default)]
    pub prompt_token_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
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
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamChoice {
    #[serde(default)]
    pub delta: Delta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Delta {
    /// Content can be null, a string, or an array of typed parts.
    #[serde(default)]
    pub content: Option<Value>,
    /// Mistral streams a complete tool call in a single chunk (no index-based
    /// incremental accumulation like OpenAI).
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct DeltaToolCall {
    pub id: String,
    pub function: DeltaFunction,
}

#[derive(Debug, Deserialize)]
pub struct DeltaFunction {
    pub name: String,
    pub arguments: String,
}
