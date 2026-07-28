//! Cohere API request/response types.

#![allow(dead_code)]

use serde::Deserialize;
use serde_json::Value;

// ── Non-streaming response ──

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub generation_id: Option<String>,
    pub message: MessageResponse,
    pub finish_reason: String,
    pub usage: UsageResponse,
}

#[derive(Debug, Deserialize)]
pub struct MessageResponse {
    pub role: String,
    /// Content items: text or thinking.
    #[serde(default)]
    pub content: Option<Vec<ContentItem>>,
    /// Tool plan string (narration of tool use).
    #[serde(default)]
    pub tool_plan: Option<String>,
    /// Tool calls requested by the model.
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
    /// Citations from RAG documents.
    #[serde(default)]
    pub citations: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentItem {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Debug, Deserialize)]
pub struct ToolCallResponse {
    pub id: String,
    pub function: FunctionCallResponse,
}

#[derive(Debug, Deserialize)]
pub struct FunctionCallResponse {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
pub struct UsageResponse {
    #[serde(default)]
    pub billed_units: Option<TokenPair>,
    pub tokens: TokenPair,
}

#[derive(Debug, Deserialize)]
pub struct TokenPair {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ── Streaming response ──
//
// Cohere streams SSE events with a named `event:` field. The JSON payload
// always has a `type` field matching the event name. We parse as a generic
// `Value` and dispatch on the `type` field in the model code.

#[derive(Debug, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub delta: Option<StreamDelta>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamDelta {
    #[serde(default)]
    pub message: Option<StreamMessage>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<StreamUsage>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamMessage {
    /// Content can be {type:"text",text:""} or {type:"thinking",thinking:""}.
    #[serde(default)]
    pub content: Option<Value>,
    /// Tool call data (for tool-call-start / tool-call-delta).
    #[serde(default)]
    pub tool_calls: Option<Value>,
    /// Tool plan string.
    #[serde(default)]
    pub tool_plan: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StreamUsage {
    pub tokens: TokenPair,
}
