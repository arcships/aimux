//! The Gemini `generateContent` / `streamGenerateContent` response schemas
//! that `google-language-model.ts` declares inline.
//!
//! These types model only the subset of the Gemini `generateContent` /
//! `streamGenerateContent` responses that the Rust provider needs. The
//! Google API is large and evolving, so we keep the types deliberately
//! loose (`serde_json::Value` for the variable-shape fields) — mirroring
//! the TS SDK's "limited version of the schema" approach.

#![allow(dead_code)]

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::GoogleErrorEnvelope;

/// `deserialize_with` for a zod `.nullish()` array: `null` reads as empty.
fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

// ── Non-streaming response (`generateContent`) ───────────────────────────────

/// Top-level Gemini `generateContent` response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    /// Stable response id (used as `ResponseMetadata::id`).
    #[serde(default)]
    pub response_id: Option<String>,
    /// Absent or `null` reads as no candidates (zod `.nullish()` upstream).
    #[serde(default, deserialize_with = "null_as_empty")]
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
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "P: Deserialize<'de>"))]
pub struct Candidate<P = Value> {
    #[serde(default)]
    pub content: Option<CandidateContent<P>>,
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
#[serde(bound(deserialize = "P: Deserialize<'de>"))]
pub struct CandidateContent<P = Value> {
    #[serde(default)]
    pub parts: Option<Vec<P>>,
    #[serde(default)]
    pub role: Option<String>,
}

// ── Usage metadata ───────────────────────────────────────────────────────────

/// `usageMetadata` block. Mirrors `convert-google-usage.ts`'s
/// `GoogleUsageMetadata`. Fields the response did not carry stay absent when
/// the block is echoed in `providerMetadata`, as in the AI SDK.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleUsageMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_token_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates_token_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_token_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_content_token_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thoughts_token_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<Vec<TokenDetail>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates_tokens_details: Option<Vec<TokenDetail>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_prompt_token_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_tokens_details: Option<Vec<TokenDetail>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_prompt_tokens_details: Option<Vec<TokenDetail>>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, Value>,
}

// ── Streaming chunk (`streamGenerateContent?alt=sse`) ────────────────────────

/// A single SSE chunk in a `streamGenerateContent` stream. Same shape as
/// `GenerateContentResponse` (the `candidates` array may be empty or
/// missing on keep-alive chunks).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamChunk<'a> {
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub response_id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    pub candidates: Option<Vec<Candidate<StreamContentPart<'a>>>>,
    #[serde(default, deserialize_with = "lenient_stream_usage")]
    pub usage_metadata: Option<GoogleUsageMetadata>,
    #[serde(default)]
    pub prompt_feedback: Option<Value>,
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub model_version: Option<Cow<'a, str>>,
}

/// Keep text borrowed and preserve the open-ended shapes of all other parts.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StreamContentPart<'a> {
    Object {
        #[serde(default, borrow)]
        text: Option<StreamText<'a>>,
        #[serde(flatten)]
        fields: serde_json::Map<String, Value>,
    },
    Other(serde::de::IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StreamText<'a> {
    Text(#[serde(borrow)] Cow<'a, str>),
    Other(serde::de::IgnoredAny),
}

impl<'a> StreamText<'a> {
    pub(super) fn into_text(self) -> Option<Cow<'a, str>> {
        match self {
            Self::Text(text) => Some(text),
            Self::Other(_) => None,
        }
    }
}

/// One Google SSE event. Error envelopes must be attempted before the very
/// permissive chunk shape, whose fields are all optional.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
// Keep the common chunk inline to avoid allocating a box for every SSE event.
#[allow(clippy::large_enum_variant)]
pub enum GoogleStreamEvent<'a> {
    Error(GoogleErrorEnvelope),
    Chunk(#[serde(borrow)] StreamChunk<'a>),
}

fn lenient_stream_usage<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<GoogleUsageMetadata>, D::Error> {
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value
        .as_ref()
        .and_then(|raw| GoogleUsageMetadata::deserialize(raw).ok()))
}
