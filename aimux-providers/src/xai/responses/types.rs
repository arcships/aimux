//! xAI Responses API response types.
//!
//! Mirrors the TS `xai-responses-api.ts` Zod schemas. We use `serde_json::Value`
//! for most fields since the xAI Responses API has a very large and evolving
//! event taxonomy (~65 streaming event types). Only the fields we actually
//! read are strongly typed; everything else passes through as `Value`.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

// ── Non-streaming response ──

/// Top-level Responses API response (`xaiResponsesResponseSchema`).
#[derive(Debug, Deserialize)]
pub struct XaiResponsesResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    pub output: Vec<serde_json::Value>,
    #[serde(default)]
    pub usage: Option<XaiResponsesUsage>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub prompt_cache_key: Option<String>,
    #[serde(default)]
    pub safety_identifier: Option<String>,
}

/// Usage object for the Responses API (`xaiResponsesUsageSchema`).
#[derive(Debug, Deserialize, Clone, Default, Serialize)]
pub struct XaiResponsesUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<XaiResponsesInputTokensDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<XaiResponsesOutputTokensDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_sources_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_server_side_tools_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_in_usd_ticks: Option<u64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Clone, Default, Serialize)]
pub struct XaiResponsesInputTokensDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Clone, Default, Serialize)]
pub struct XaiResponsesOutputTokensDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

// ── Streaming ──
// Streaming events are parsed as `serde_json::Value` and dispatched by `type`.

/// A streaming SSE event from the Responses API. The `event_type` field
/// corresponds to the `type` field in the JSON payload.
#[must_use]
pub fn event_type(event: &serde_json::Value) -> &str {
    event.get("type").and_then(|v| v.as_str()).unwrap_or("")
}
