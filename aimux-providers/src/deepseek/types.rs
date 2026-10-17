//! DeepSeek chat API response types (`deepseek-chat-api-types.ts`).
//!
//! Limited versions of the schemas, focused on what the model needs. The
//! `usage` object stays a `Value`: it is returned as `usage.raw`.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// `deepSeekErrorSchema` turned into the failed-response handler, as the
/// chat model builds it (deepseek@3.0.56 `deepseek-chat-api-types.ts:117-124`,
/// `deepseek-chat-language-model.ts:213-217`): the message is
/// `error.message`; `error.code` (a string or a number), else `error.type`,
/// is the provider code.
pub(crate) fn deepseek_failed_response_handler()
-> aimux_provider_utils::ResponseHandler<aimux_core::error::AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .and_then(|error| error.get("code").or_else(|| error.get("type")))
                .and_then(|value| match value {
                    Value::String(value) => Some(value.clone()),
                    Value::Number(value) => Some(value.to_string()),
                    _ => None,
                }),
        }
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatResponse {
    pub id: Option<String>,
    pub created: Option<f64>,
    pub model: Option<String>,
    #[serde(default, deserialize_with = "deserialize_response_object")]
    pub object: Option<String>,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<ResponseChoice>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseChoice {
    pub index: Option<u32>,
    pub message: ResponseMessage,
    pub logprobs: Option<Value>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseMessage {
    #[serde(default, deserialize_with = "deserialize_role")]
    pub role: Option<String>,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseToolCall {
    pub id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_tool_type")]
    pub r#type: Option<String>,
    pub function: ResponseToolCallFunction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatChunk {
    pub id: Option<String>,
    pub created: Option<f64>,
    pub model: Option<String>,
    #[serde(default, deserialize_with = "deserialize_chunk_object")]
    pub object: Option<String>,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<ChunkChoice>,
    pub usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkChoice {
    pub index: Option<u32>,
    pub delta: Option<ChunkDelta>,
    pub logprobs: Option<Value>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkDelta {
    #[serde(default, deserialize_with = "deserialize_role")]
    pub role: Option<String>,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ChunkToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkToolCall {
    pub index: usize,
    pub id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_tool_type")]
    pub r#type: Option<String>,
    pub function: ChunkToolCallFunction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

fn deserialize_literal<'de, D>(deserializer: D, literal: &str) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if value.as_deref().is_some_and(|value| value != literal) {
        return Err(serde::de::Error::custom(format!("expected {literal}")));
    }
    Ok(value)
}

fn deserialize_response_object<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    deserialize_literal(deserializer, "chat.completion")
}

fn deserialize_chunk_object<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    deserialize_literal(deserializer, "chat.completion.chunk")
}

fn deserialize_role<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    deserialize_literal(deserializer, "assistant")
}

fn deserialize_tool_type<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    deserialize_literal(deserializer, "function")
}
