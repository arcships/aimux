//! OpenAI Responses API response/streaming types.
//!
//! The Responses API (`/v1/responses`) uses a completely different wire format
//! from chat completions: requests carry an `input` array (not `messages`),
//! and streaming events are typed as `response.output_text.delta`,
//! `response.function_call_arguments.delta`, etc.
//!
//! Most of the response/chunk payload is a large discriminated union over the
//! `type` field. Rather than model every variant with serde (the union has
//! dozens of members and evolves frequently), we deserialize each SSE event /
//! JSON response into a [`serde_json::Value`] and dispatch on `type` in the
//! model code. Only [`ResponsesUsage`] — which has a stable, well-defined
//! shape — gets a dedicated typed struct.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

// ── Usage ────────────────────────────────────────────────────────────────────

/// Token usage returned by the Responses API.
///
/// Mirrors the TS `OpenAIResponsesUsage`. Field names are snake_case to match
/// the wire format.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub input_tokens_details: Option<ResponsesInputTokensDetails>,
    #[serde(default)]
    pub output_tokens_details: Option<ResponsesOutputTokensDetails>,
    /// Some OpenAI-compatible upstreams include a top-level `total_tokens`.
    #[serde(default)]
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesInputTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
    #[serde(default)]
    pub cache_write_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesOutputTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
}
