//! DeepSeek chat API response types (`deepseek-chat-api-types.ts`).
//!
//! Limited versions of the schemas, focused on what the model needs. The
//! `usage` object stays a `Value`: it is returned as `usage.raw`.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct ChatResponse {
    pub id: Option<String>,
    pub created: Option<f64>,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<ResponseChoice>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseChoice {
    pub message: ResponseMessage,
    pub logprobs: Option<Value>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseMessage {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub reasoning: Option<String>,
    pub tool_calls: Option<Vec<ResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseToolCall {
    pub id: Option<String>,
    pub function: ResponseToolCallFunction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatChunk {
    pub id: Option<String>,
    pub created: Option<f64>,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<ChunkChoice>,
    pub usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkChoice {
    pub delta: Option<ChunkDelta>,
    pub logprobs: Option<Value>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkDelta {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub reasoning: Option<String>,
    pub tool_calls: Option<Vec<ChunkToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkToolCall {
    pub index: usize,
    pub id: Option<String>,
    pub function: ChunkToolCallFunction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}
