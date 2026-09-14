//! Assemble the assistant message replayed to the model on the next turn.
//!
//! Rust port of the AI SDK's `toResponseMessages`
//! (`packages/ai/src/generate-text/to-response-messages.ts`): text and
//! reasoning arrive as deltas and must be flushed as positioned segments the
//! moment a part of a different kind lands, so the replayed transcript keeps
//! provider order. Tool calls/results and reasoning signatures are replayed
//! verbatim — Anthropic/Bedrock thinking signatures must round-trip exactly.

use serde_json::Value;

use crate::content::ContentPart;
use crate::message::{MessageContent, ModelMessage, Role};
use crate::result::ReasoningPart;

/// Match the AI SDK's response-message safety rule for invalid tool calls:
/// malformed primitive input must not be replayed as a prompt tool-call input.
/// JavaScript's `typeof value === "object"` includes arrays and null, so those
/// values are intentionally retained here as well.
pub(crate) fn response_tool_call_input(input: &Value, invalid: Option<bool>) -> Value {
    if invalid == Some(true) && !matches!(input, Value::Object(_) | Value::Array(_) | Value::Null) {
        Value::Object(serde_json::Map::new())
    } else {
        input.clone()
    }
}

/// Reasoning signature echoed back on the next turn (Anthropic:
/// `provider_metadata.anthropic.signature`; Bedrock: `.bedrock.signature` /
/// `.amazonBedrock.signature`).
pub(crate) fn extract_reasoning_signature(provider_metadata: Option<&Value>) -> Option<String> {
    let metadata = provider_metadata?;
    ["anthropic", "bedrock", "amazonBedrock"]
        .iter()
        .find_map(|ns| metadata.get(ns)?.get("signature")?.as_str())
        .map(str::to_owned)
}

/// Accumulates response-message content parts in provider order.
///
/// Streaming feeds it per-event (`text_start`/`text_delta`/…); the
/// non-streaming path feeds whole segments (`text`/`reasoning`). Both paths
/// share the flush discipline and the tool-call/result placement rules.
#[derive(Default)]
pub(crate) struct ResponseMessageBuilder {
    parts: Vec<ContentPart>,
    text_buf: String,
    text_provider_options: Option<Value>,
    reasoning_buf: String,
    reasoning_provider_options: Option<Value>,
    reasoning: Vec<ReasoningPart>,
}

/// What the builder produced: the replayable assistant message plus the
/// reasoning aggregate surfaced on the result.
pub(crate) struct ResponseMessages {
    pub messages: Vec<ModelMessage>,
    pub reasoning: Vec<ReasoningPart>,
}

impl ResponseMessageBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Streaming events ────────────────────────────────────────────────

    pub fn text_start(&mut self, provider_metadata: Option<Value>) {
        self.flush_reasoning();
        // A new text segment establishes its position immediately; flush a
        // preceding implicit segment before starting it.
        self.flush_text();
        self.text_provider_options = provider_metadata;
    }

    pub fn text_delta(&mut self, delta: &str, provider_metadata: Option<Value>) {
        self.flush_reasoning();
        self.text_buf.push_str(delta);
        if provider_metadata.is_some() {
            self.text_provider_options = provider_metadata;
        }
    }

    pub fn text_end(&mut self, provider_metadata: Option<Value>) {
        if provider_metadata.is_some() {
            self.text_provider_options = provider_metadata;
        }
        self.flush_text();
    }

    pub fn reasoning_start(&mut self, provider_metadata: Option<Value>) {
        self.flush_text();
        self.flush_reasoning();
        self.reasoning_provider_options = provider_metadata;
    }

    pub fn reasoning_delta(&mut self, delta: &str, provider_metadata: Option<Value>) {
        self.flush_text();
        self.reasoning_buf.push_str(delta);
        if provider_metadata.is_some() {
            self.reasoning_provider_options = provider_metadata;
        }
    }

    pub fn reasoning_end(&mut self, provider_metadata: Option<Value>) {
        self.flush_text();
        if provider_metadata.is_some() {
            self.reasoning_provider_options = provider_metadata;
        }
        self.flush_reasoning();
    }

    // ── Whole segments (non-streaming) ──────────────────────────────────

    pub fn text(&mut self, text: &str, provider_metadata: Option<&Value>) {
        if !text.is_empty() {
            self.parts.push(ContentPart::Text {
                text: text.to_owned(),
                provider_options: provider_metadata.cloned(),
            });
        }
    }

    pub fn reasoning(&mut self, text: &str, provider_metadata: Option<&Value>) {
        // Pushed unconditionally: redacted thinking has empty text but its
        // provider metadata must still be replayed.
        self.reasoning.push(ReasoningPart {
            text: text.to_owned(),
        });
        let signature = extract_reasoning_signature(provider_metadata);
        self.parts.push(ContentPart::Reasoning {
            text: text.to_owned(),
            signature,
            provider_options: provider_metadata.cloned(),
        });
    }

    // ── Tool parts (both paths) ─────────────────────────────────────────

    pub fn tool_call(&mut self, call: &crate::tool::ToolCall) {
        self.flush_text();
        self.flush_reasoning();
        self.parts.push(ContentPart::ToolCall {
            tool_call_id: call.tool_call_id.clone(),
            tool_name: call.tool_name.clone(),
            input: response_tool_call_input(&call.input, call.invalid),
            provider_executed: call.provider_executed,
            thought_signature: call.thought_signature.clone(),
            provider_options: call.provider_metadata.clone(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tool_result(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        result: Value,
        is_error: Option<bool>,
        preliminary: Option<bool>,
        dynamic: Option<bool>,
        provider_options: Option<Value>,
    ) {
        // Preliminary server-tool results are transient stream updates. The
        // provider contract requires a later final result, and only that
        // final value belongs in the replay transcript for the next turn.
        if preliminary == Some(true) {
            return;
        }
        self.flush_text();
        self.flush_reasoning();
        self.parts.push(ContentPart::ToolResult {
            tool_call_id,
            tool_name: Some(tool_name),
            result,
            is_error,
            preliminary,
            dynamic,
            provider_options,
        });
    }

    // ── Finalization ────────────────────────────────────────────────────

    pub fn finish(mut self) -> ResponseMessages {
        self.flush_text();
        self.flush_reasoning();
        let messages = if self.parts.is_empty() {
            Vec::new()
        } else {
            vec![ModelMessage {
                role: Role::Assistant,
                content: MessageContent::Parts(self.parts),
            }]
        };
        ResponseMessages {
            messages,
            reasoning: self.reasoning,
        }
    }

    fn flush_text(&mut self) {
        if !self.text_buf.is_empty() {
            self.parts.push(ContentPart::Text {
                text: std::mem::take(&mut self.text_buf),
                provider_options: self.text_provider_options.take(),
            });
        } else {
            self.text_provider_options = None;
        }
    }

    fn flush_reasoning(&mut self) {
        if self.reasoning_buf.is_empty() && self.reasoning_provider_options.is_none() {
            return;
        }
        let text = std::mem::take(&mut self.reasoning_buf);
        if !text.is_empty() {
            self.reasoning.push(ReasoningPart { text: text.clone() });
        }
        let provider_options = self.reasoning_provider_options.take();
        let signature = extract_reasoning_signature(provider_options.as_ref());
        self.parts.push(ContentPart::Reasoning {
            text,
            signature,
            provider_options,
        });
    }
}
