//! Cohere API request/response types.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Non-streaming response ──

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub generation_id: Option<String>,
    pub message: MessageResponse,
    pub finish_reason: String,
    pub usage: UsageResponse,
}

#[derive(Debug, Deserialize)]
pub struct MessageResponse {
    pub role: String,
    /// Content items: text or thinking.
    #[serde(default)]
    pub content: Option<Vec<ContentItem>>,
    /// Tool plan string (narration of tool use).
    #[serde(default)]
    pub tool_plan: Option<String>,
    /// Tool calls requested by the model.
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
    /// Citations from RAG documents.
    #[serde(default)]
    pub citations: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentItem {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Debug, Deserialize)]
pub struct ToolCallResponse {
    pub id: String,
    pub function: FunctionCallResponse,
}

#[derive(Debug, Deserialize)]
pub struct FunctionCallResponse {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug)]
pub struct UsageResponse {
    pub tokens: TokenPair,
    pub raw: Value,
}

impl<'de> Deserialize<'de> for UsageResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        #[derive(Deserialize)]
        struct Validated {
            tokens: TokenPair,
            #[serde(default)]
            billed_units: Option<TokenPair>,
            #[serde(default)]
            cached_tokens: Option<f64>,
        }
        let validated: Validated =
            serde_json::from_value(raw.clone()).map_err(serde::de::Error::custom)?;
        Ok(Self {
            tokens: validated.tokens,
            raw,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TokenPair {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ── Streaming response ──
//
// Cohere streams SSE events with a named `event:` field. The JSON payload
// always has a `type` field matching the event name. We parse as a generic
// `Value` and dispatch on the `type` field in the model code.

#[derive(Debug, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub index: Option<f64>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub delta: Option<StreamDelta>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamDelta {
    #[serde(default)]
    pub message: Option<StreamMessage>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<StreamUsage>,
}

#[derive(Debug, Deserialize, Default)]
pub struct StreamMessage {
    /// Content can be {type:"text",text:""} or {type:"thinking",thinking:""}.
    #[serde(default)]
    pub content: Option<Value>,
    /// Tool call data (for tool-call-start / tool-call-delta).
    #[serde(default)]
    pub tool_calls: Option<Value>,
    /// Tool plan string.
    #[serde(default)]
    pub tool_plan: Option<String>,
}

pub type StreamUsage = UsageResponse;

impl StreamEvent {
    pub fn parse(raw: Value) -> Result<Self, aimux_core::error::AiMuxError> {
        let valid = match raw["type"].as_str() {
            Some("citation-start" | "citation-end" | "tool-call-end") => true,
            Some("message-start") => raw
                .get("id")
                .is_none_or(|id| id.is_null() || id.is_string()),
            Some("content-end") => raw["index"].is_number(),
            Some("content-start" | "content-delta") => {
                let content = &raw["delta"]["message"]["content"];
                raw["index"].is_number()
                    && if raw["type"] == "content-delta" {
                        content["text"].is_string() || content["thinking"].is_string()
                    } else {
                        match content["type"].as_str() {
                            Some("text") => content["text"].is_string(),
                            Some("thinking") => content["thinking"].is_string(),
                            _ => false,
                        }
                    }
            }
            Some("message-end") => {
                raw["delta"]["finish_reason"].is_string() && raw["delta"]["usage"].is_object()
            }
            Some("tool-plan-delta") => raw["delta"]["message"]["tool_plan"].is_string(),
            Some("tool-call-start" | "tool-call-delta") => {
                let tool = &raw["delta"]["message"]["tool_calls"];
                tool["function"]["arguments"].is_string()
                    && (raw["type"] == "tool-call-delta"
                        || (tool["id"].is_string()
                            && tool["type"] == "function"
                            && tool["function"]["name"].is_string()))
            }
            _ => false,
        };
        if valid {
            // Strip fields outside the selected upstream event schema before deserialization.
            let mut value = serde_json::json!({ "type": raw["type"] });
            match raw["type"].as_str().unwrap() {
                "content-start" | "content-delta" => {
                    value["index"] = raw["index"].clone();
                    value["delta"] = serde_json::json!({ "message": { "content": raw["delta"]["message"]["content"] } });
                }
                "content-end" => value["index"] = raw["index"].clone(),
                "message-start" => value["id"] = raw["id"].clone(),
                "message-end" => {
                    value["delta"] = serde_json::json!({ "finish_reason": raw["delta"]["finish_reason"], "usage": raw["delta"]["usage"] })
                }
                "tool-call-start" | "tool-call-delta" => {
                    value["delta"] = serde_json::json!({ "message": { "tool_calls": raw["delta"]["message"]["tool_calls"] } })
                }
                "tool-plan-delta" => {
                    value["delta"] = serde_json::json!({ "message": { "tool_plan": raw["delta"]["message"]["tool_plan"] } })
                }
                _ => {}
            }
            if let Ok(event) = serde_json::from_value(value) {
                return Ok(event);
            }
        }
        Err(aimux_core::error::AiMuxError::InvalidResponseData(format!(
            "Invalid Cohere stream event: {raw}"
        )))
    }
}
