//! The response shapes the chat model reads.
//!
//! Mirrors the `groqChatResponseSchema` and `groqChatChunkSchema` of
//! `groq-chat-language-model.ts`, limited to what the implementation needs.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

// ── Usage ───────────────────────────────────────────────────────────────────

/// The `usage` object (groq@4.0.52 `groq-chat-language-model.ts:564-580`).
/// `raw` is the object as zod parsed it, which `convertGroqUsage` returns as
/// `usage.raw`: a `null` field stays `null` and unknown keys are dropped.
#[derive(Debug, Default)]
pub(crate) struct GroqUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    pub raw: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
}

impl<'de> Deserialize<'de> for GroqUsage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// The schema's keys, with the keys of the nested objects.
        const KEYS: [(&str, &[&str]); 5] = [
            ("prompt_tokens", &[]),
            ("completion_tokens", &[]),
            ("total_tokens", &[]),
            ("prompt_tokens_details", &["cached_tokens"]),
            ("completion_tokens_details", &["reasoning_tokens"]),
        ];

        #[derive(Deserialize)]
        struct Counts {
            #[serde(default)]
            prompt_tokens: Option<u32>,
            #[serde(default)]
            completion_tokens: Option<u32>,
            // Checked to be a number, not read.
            #[serde(default)]
            #[allow(dead_code)]
            total_tokens: Option<u32>,
            #[serde(default)]
            prompt_tokens_details: Option<PromptTokensDetails>,
            #[serde(default)]
            completion_tokens_details: Option<CompletionTokensDetails>,
        }

        let mut object = Map::<String, Value>::deserialize(deserializer)?;
        let mut raw = Map::new();
        for (key, nested) in KEYS {
            let Some(mut value) = object.remove(key) else {
                continue;
            };
            if let Value::Object(fields) = &mut value {
                fields.retain(|field, _| nested.contains(&field.as_str()));
            }
            raw.insert(key.to_string(), value);
        }
        let counts = Counts::deserialize(Value::Object(raw.clone())).map_err(D::Error::custom)?;
        Ok(Self {
            prompt_tokens: counts.prompt_tokens,
            completion_tokens: counts.completion_tokens,
            prompt_tokens_details: counts.prompt_tokens_details,
            completion_tokens_details: counts.completion_tokens_details,
            raw,
        })
    }
}

// ── Non-streaming response ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChatResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    pub choices: Vec<GroqChatChoice>,
    #[serde(default)]
    pub usage: Option<GroqUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChatChoice {
    pub message: GroqChatMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChatMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<GroqToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqToolCall {
    #[serde(default)]
    pub id: Option<String>,
    pub function: GroqToolCallFunction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqToolCallFunction {
    pub name: String,
    pub arguments: String,
}

// ── Streaming response ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChatChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    pub choices: Vec<GroqChunkChoice>,
    #[serde(default)]
    pub x_groq: Option<XGroq>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChunkChoice {
    #[serde(default)]
    pub delta: Option<GroqChunkDelta>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChunkDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<GroqChunkToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChunkToolCall {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(
        default,
        rename = "type",
        deserialize_with = "deserialize_tool_call_type"
    )]
    pub r#type: Option<String>,
    pub function: GroqChunkToolCallFunction,
}

fn deserialize_tool_call_type<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value != "function" {
        return Err(serde::de::Error::custom("expected tool call type function"));
    }
    Ok(Some(value))
}

#[derive(Debug, Deserialize)]
pub(crate) struct GroqChunkToolCallFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct XGroq {
    #[serde(default)]
    pub usage: Option<GroqUsage>,
}
