//! Amazon Bedrock Converse API types.
//!
//! Models the subset of the Converse `/converse` and `/converse-stream`
//! request/response shapes that the Rust provider needs. Bedrock's Converse API
//! is a unified interface across all model providers (Anthropic, Meta, Mistral,
//! etc.), so the request/response format is provider-agnostic.
//!
//! Reference: <https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_Converse.html>

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Non-streaming response (`/converse`) ─────────────────────────────────────

/// Top-level Converse response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BedrockConverseResponse {
    #[serde(default)]
    pub output: Option<BedrockOutput>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<BedrockUsage>,
    #[serde(default)]
    pub additional_model_response_fields: Option<Value>,
    #[serde(default)]
    pub metrics: Option<Value>,
}

/// `output` block — wraps the generated `message`.
#[derive(Debug, Deserialize)]
pub struct BedrockOutput {
    #[serde(default)]
    pub message: Option<BedrockMessage>,
}

/// A message in the response (role + content blocks).
#[derive(Debug, Deserialize)]
pub struct BedrockMessage {
    pub role: String,
    #[serde(default)]
    pub content: Vec<BedrockContentBlock>,
}

/// A content block in a Bedrock message. Bedrock uses a tagged-union-by-field
/// shape: each block has exactly one of `text`, `toolUse`, etc.
#[derive(Debug, Deserialize)]
pub struct BedrockContentBlock {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default, rename = "toolUse")]
    pub tool_use: Option<BedrockToolUse>,
    #[serde(default, rename = "reasoningContent")]
    pub reasoning_content: Option<Value>,
}

/// `toolUse` block.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BedrockToolUse {
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: Value,
}

/// Usage block.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BedrockUsage {
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    #[serde(default, rename = "cacheReadInputTokens")]
    pub cache_read_input_tokens: Option<u32>,
    #[serde(default, rename = "cacheWriteInputTokens")]
    pub cache_write_input_tokens: Option<u32>,
}

// ── Error response ───────────────────────────────────────────────────────────

/// Bedrock error envelope: `{ "message": "...", "type": "..." }` (simplified).
#[derive(Debug, Deserialize)]
pub struct BedrockError {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default, rename = "type")]
    pub error_type: Option<String>,
}
