//! Mistral API request/response types.

#![allow(dead_code)]

use serde::Deserialize;
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

#[derive(Debug)]
pub struct UsageResponse {
    pub fields: UsageFields,
    pub raw: Value,
}

impl<'de> Deserialize<'de> for UsageResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        let fields = serde_json::from_value(raw.clone()).map_err(serde::de::Error::custom)?;
        Ok(Self { fields, raw })
    }
}

#[derive(Debug, Deserialize)]
pub struct UsageFields {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub prompt_audio_seconds: Option<f64>,
    pub request_count: Option<f64>,
    pub service_tier: Option<String>,
    pub num_cached_tokens: Option<u32>,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub prompt_token_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub struct PromptTokensDetails {
    pub cached_tokens: Option<u32>,
    pub audio_tokens: Option<f64>,
    pub messages: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub struct CompletionTokensDetails {
    pub reasoning_tokens: Option<f64>,
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
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct DeltaToolCall {
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    pub function: DeltaFunction,
}

#[derive(Debug, Deserialize)]
pub struct DeltaFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}
