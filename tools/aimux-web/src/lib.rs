//! Console wire DTOs (RFC-0029 §5.3). Shared by the web server and codegen.
//!
//! TypeScript is written by the aimux-codegen TypeScript emitter into
//! `web/src/types/generated/`. Never edit those files by hand.

use std::collections::HashMap;

use aimux_core::content::ToolResultOutput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One model call from the console (RFC-0029 §5.3).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WireCallRequest {
    /// Registry name or native protocol name ("openai", "deepseek", …).
    pub provider: String,
    /// Model id (free text, e.g. "gpt-4o").
    pub model: String,
    /// API key: `None` (Settings-saved key or the provider's registered env
    /// var), `Some("env:VAR")`, or a plaintext literal — the latter only
    /// while the console is loopback-bound (RFC-0029 §5.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL override (proxies / local endpoints).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Stream the response as SSE (`TextStreamPart` events). Default true.
    #[serde(default = "default_true")]
    pub stream: bool,
    /// Route through the loaded mock model (offline mode, RFC-0023 P3).
    #[serde(default)]
    pub mock: bool,
    #[serde(default)]
    pub options: WireOptions,
    /// Session grouping id (RFC-0024). The console reuses one id per agent run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Informational step marker (the backend assigns the authoritative step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    pub messages: Vec<WireMessage>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WireOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<WireTool>>,
    /// A `ResponseFormat` object: `{"type":"text"}` or `{"type":"json",…}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_raw_chunks: Option<bool>,
}

/// A function tool definition (JSON Schema parameters).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WireTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema describing the parameters.
    pub parameters: Value,
}

/// A chat message in the wire format.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WireMessage {
    /// "system" | "user" | "assistant" | "tool".
    pub role: String,
    pub content: Vec<WireContentPart>,
}

/// A content part in the wire format.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum WireContentPart {
    Text {
        text: String,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
    },
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        /// The core type, imported by the frontend through the `@aimux/types` alias.
        output: ToolResultOutput,
    },
}

/// Response for non-streaming calls.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WireCallResponse {
    pub text: String,
    pub finish_reason: Value,
    pub usage: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<WireMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// SSE `meta` event payload — the frontend trace anchor (RFC-0029 §5.2).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WireMeta {
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    pub outcome: String,
}
