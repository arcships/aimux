//! Google Gemini API request/response types.
//!
//! These types model only the subset of the Gemini `generateContent` /
//! `streamGenerateContent` responses that the Rust provider needs. The
//! Google API is large and evolving, so we keep the types deliberately
//! loose (`serde_json::Value` for the variable-shape fields) — mirroring
//! the TS SDK's "limited version of the schema" approach.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Non-streaming response (`generateContent`) ───────────────────────────────

/// Top-level Gemini `generateContent` response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    /// Stable response id (used as `ResponseMetadata::id`).
    #[serde(default)]
    pub response_id: Option<String>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub usage_metadata: Option<GoogleUsageMetadata>,
    #[serde(default)]
    pub prompt_feedback: Option<Value>,
    /// Echoed by some endpoints (e.g. `modelVersion`).
    #[serde(default)]
    pub model_version: Option<String>,
}

/// A single candidate in the response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    #[serde(default)]
    pub content: Option<CandidateContent>,
    /// `STOP`, `MAX_TOKENS`, `SAFETY`, `RECITATION`, …
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub finish_message: Option<String>,
    #[serde(default)]
    pub safety_ratings: Option<Vec<Value>>,
    #[serde(default)]
    pub grounding_metadata: Option<Value>,
    #[serde(default)]
    pub url_context_metadata: Option<Value>,
    #[serde(default)]
    pub index: Option<i32>,
}

/// `candidate.content` — a role plus an ordered list of parts.
#[derive(Debug, Deserialize)]
pub struct CandidateContent {
    #[serde(default)]
    pub parts: Option<Vec<Value>>,
    #[serde(default)]
    pub role: Option<String>,
}

// ── Usage metadata ───────────────────────────────────────────────────────────

/// `usageMetadata` block. Mirrors `convert-google-usage.ts`'s
/// `GoogleUsageMetadata`.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleUsageMetadata {
    #[serde(default)]
    pub prompt_token_count: Option<u32>,
    #[serde(default)]
    pub candidates_token_count: Option<u32>,
    #[serde(default)]
    pub total_token_count: Option<u32>,
    #[serde(default)]
    pub cached_content_token_count: Option<u32>,
    #[serde(default)]
    pub thoughts_token_count: Option<u32>,
    #[serde(default)]
    pub traffic_type: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub prompt_tokens_details: Option<Vec<TokenDetail>>,
    #[serde(default)]
    pub candidates_tokens_details: Option<Vec<TokenDetail>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenDetail {
    #[serde(default)]
    pub modality: Option<String>,
    #[serde(default)]
    pub token_count: Option<u32>,
}

// ── Streaming chunk (`streamGenerateContent?alt=sse`) ────────────────────────

/// A single SSE chunk in a `streamGenerateContent` stream. Same shape as
/// `GenerateContentResponse` (the `candidates` array may be empty or
/// missing on keep-alive chunks).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamChunk {
    #[serde(default)]
    pub response_id: Option<String>,
    #[serde(default)]
    pub candidates: Option<Vec<Candidate>>,
    #[serde(default)]
    pub usage_metadata: Option<GoogleUsageMetadata>,
    #[serde(default)]
    pub prompt_feedback: Option<Value>,
    #[serde(default)]
    pub model_version: Option<String>,
}

// ── Provider error response ──────────────────────────────────────────────────

/// Google error envelope: `{ "error": { "code": 400, "message": "...", "status": "INVALID_ARGUMENT" } }`.
#[derive(Debug, Deserialize)]
pub struct GoogleErrorEnvelope {
    pub error: GoogleError,
}

#[derive(Debug, Deserialize)]
pub struct GoogleError {
    #[serde(default)]
    pub code: Option<i64>,
    pub message: String,
    #[serde(default)]
    pub status: Option<String>,
}
