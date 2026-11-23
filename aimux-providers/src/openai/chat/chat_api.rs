//! Chat Completions response and chunk types (openai-chat-api.ts).

#![allow(dead_code)]

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Non-streaming response ──

#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    pub choices: Vec<Choice>,
    pub usage: Option<UsageWithRaw>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: MessageResponse,
    #[serde(default)]
    pub finish_reason: Option<String>,
    /// Logprobs returned when `logprobs` is requested.
    #[serde(default)]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct MessageResponse {
    pub role: Option<String>,
    pub content: Option<String>,
    /// Refusal text, visible output like `content` (#461).
    #[serde(default)]
    pub refusal: Option<String>,
    #[serde(default)]
    pub audio: Option<AudioResponse>,
    /// Reasoning text (`reasoning` field of OpenAI-shaped servers).
    #[serde(default)]
    pub reasoning: Option<String>,
    /// `reasoning_content` field of OpenAI-shaped servers (DashScope and others).
    /// 优先于 `reasoning`（当两者同时存在时）。
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
    /// Annotations (e.g. URL citations) — stored as raw JSON.
    #[serde(default)]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub struct AudioResponse {
    pub transcript: Option<String>,
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

/// Keep vendor extensions from the usage subtree even when the full body
/// is omitted. Non-streaming usage retains its strict schema validation.
#[derive(Debug)]
pub struct UsageWithRaw {
    pub typed: UsageResponse,
    pub raw: Value,
}

impl<'de> Deserialize<'de> for UsageWithRaw {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        let typed = UsageResponse::deserialize(&raw).map_err(serde::de::Error::custom)?;
        Ok(Self { typed, raw })
    }
}

#[derive(Debug, Default, Deserialize, Clone, Serialize)]
pub struct UsageResponse {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
    /// Cache-write token count. Alibaba (DashScope) reports this under the
    /// field name `cache_creation_input_tokens`; the alias makes both wire
    /// shapes parse into the same field.
    #[serde(default, alias = "cache_creation_input_tokens")]
    pub cache_write_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
    #[serde(default)]
    pub accepted_prediction_tokens: Option<u32>,
    #[serde(default)]
    pub rejected_prediction_tokens: Option<u32>,
}

// ── Streaming response ──

#[derive(Debug, Deserialize)]
pub struct StreamChunk<'a> {
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub id: Option<Cow<'a, str>>,
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub model: Option<Cow<'a, str>>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default, borrow)]
    pub choices: Vec<StreamChoice<'a>>,
    #[serde(default)]
    pub usage: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamChoice<'a> {
    #[serde(default, borrow)]
    pub delta: Delta<'a>,
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub finish_reason: Option<Cow<'a, str>>,
    /// Logprobs returned when `logprobs` is requested.
    #[serde(default)]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Delta<'a> {
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub content: Option<Cow<'a, str>>,
    /// Refusal text delta, visible output like `content` (#461).
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub refusal: Option<Cow<'a, str>>,
    /// Reasoning text delta (`reasoning` field of OpenAI-shaped servers).
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub reasoning: Option<Cow<'a, str>>,
    /// `reasoning_content` delta of OpenAI-shaped servers (DashScope and others).
    /// 优先于 `reasoning`。
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub reasoning_content: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    pub tool_calls: Option<Vec<DeltaToolCall<'a>>>,
    /// Annotations (e.g. URL citations) carried by a streaming delta.
    #[serde(default)]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub struct DeltaToolCall<'a> {
    /// Required, as in `@ai-sdk/openai` (`index: z.number()`); a chunk without
    /// it fails to parse. Servers that omit it go through `openai_compatible`.
    pub index: usize,
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub id: Option<Cow<'a, str>>,
    #[serde(
        default,
        borrow,
        rename = "type",
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub r#type: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    pub function: Option<DeltaFunction<'a>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DeltaFunction<'a> {
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub name: Option<Cow<'a, str>>,
    #[serde(
        default,
        borrow,
        deserialize_with = "crate::stream::deserialize_optional_cow"
    )]
    pub arguments: Option<Cow<'a, str>>,
}

// Suppress unused warning for Value import (used in convert.rs).
#[allow(unused_imports)]
use serde_json::Value as _Value;
