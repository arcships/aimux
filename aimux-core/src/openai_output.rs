//! OpenAI Chat Completions output format.
//!
//! Converts aimux's internal types ([`GenerateResult`] / [`StreamPart`]) into
//! standard OpenAI Chat Completions structures ([`ChatCompletion`] /
//! [`ChatCompletionChunk`]). This lets any provider (OpenAI, Anthropic, Google,
//! …) be consumed via the OpenAI wire format.
//!
//! # Architecture
//!
//! ```text
//!   provider.do_generate()  →  GenerateResult  →  to_chat_completion()  →  ChatCompletion
//!   provider.do_stream()    →  Stream<StreamPart>  →  to_chat_completion_stream()  →  Stream<Chunk>
//! ```
//!
//! The conversion is a post-processing step — it does not modify the
//! `LanguageModel` trait or existing `generate_text` / `stream_text` APIs.
//!
//! # Round-trip fidelity
//!
//! For OpenAI-compatible providers the path is:
//! `OpenAI JSON → GenerateResult → ChatCompletion`. The content and tool_calls
//! fields round-trip losslessly: arguments go through `from_str ↔ to_string`
//! (a reversible pair), and usage fields map back to their OpenAI names.
//! Response metadata (`object`, `created`, `system_fingerprint`) is either
//! reconstructed from constants or taken from `GenerateResult.response`.

use std::collections::HashMap;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::Stream;
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::result::GenerateContent;
use crate::result::GenerateResult;
use crate::shared::FileData;
use crate::stream_part::StreamPart;
use crate::types::{FinishReason, FinishReasonUnified, Usage};

// ─────────────────────────────────────────────────────────────────────────────
// Non-streaming response types
// ─────────────────────────────────────────────────────────────────────────────

/// A complete Chat Completion response (non-streaming).
///
/// Mirrors the OpenAI `chat.completion` object.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletion {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatCompletionChoice>,
    pub usage: ChatCompletionUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChoice {
    pub index: u32,
    pub message: ChatCompletionMessage,
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatCompletionToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ChatCompletionFunction,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionFunction {
    pub name: String,
    pub arguments: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming response types
// ─────────────────────────────────────────────────────────────────────────────

/// A single Chat Completion chunk (streaming).
///
/// Mirrors the OpenAI `chat.completion.chunk` object.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatCompletionChunkChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatCompletionUsage>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChunkChoice {
    pub index: u32,
    pub delta: ChatCompletionDelta,
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatCompletionChunkToolCall>>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChunkToolCall {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    pub function: ChatCompletionChunkFunction,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChunkFunction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Usage (shared by streaming and non-streaming)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct PromptTokensDetails {
    pub cached_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct CompletionTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Options
// ─────────────────────────────────────────────────────────────────────────────

/// Options for streaming OpenAI-compatible output.
#[derive(Debug, Clone)]
pub struct OpenAiStreamOptions {
    /// Whether to include `usage` in the final chunk
    /// (corresponds to `stream_options.include_usage`).
    pub include_usage: bool,
    /// Whether to emit `reasoning_content` deltas (default `true`).
    pub include_reasoning: bool,
}

impl Default for OpenAiStreamOptions {
    fn default() -> Self {
        Self {
            include_usage: true,
            include_reasoning: true,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Non-streaming conversion: GenerateResult → ChatCompletion
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a [`GenerateResult`] into an OpenAI [`ChatCompletion`].
///
/// `model` is the model ID to place in the response (usually the model the
/// caller invoked). If `result.response.model_id` is present it takes
/// precedence.
pub fn to_chat_completion(result: &GenerateResult, model: &str) -> ChatCompletion {
    let mut content_text = String::new();
    let mut reasoning_text = String::new();
    let mut tool_calls = Vec::new();
    let mut annotations = Vec::new();

    for item in &result.content {
        match item {
            GenerateContent::Text { text, .. } => {
                content_text.push_str(text);
            }
            GenerateContent::Reasoning { text, .. } => {
                reasoning_text.push_str(text);
            }
            GenerateContent::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                let arguments = if input.is_null() {
                    "{}".to_string()
                } else {
                    input.to_string()
                };
                tool_calls.push(ChatCompletionToolCall {
                    id: tool_call_id.clone(),
                    tool_type: "function".to_string(),
                    function: ChatCompletionFunction {
                        name: tool_name.clone(),
                        arguments,
                    },
                });
            }
            GenerateContent::Source { url, title, .. } => {
                // Map to OpenAI url_citation annotation.
                let mut ann = serde_json::json!({
                    "type": "url_citation",
                    "url_citation": {
                        "url": url.clone().unwrap_or_default(),
                    }
                });
                if let Some(t) = title {
                    ann["url_citation"]["title"] = serde_json::Value::String(t.clone());
                }
                annotations.push(ann);
                // Also append URL to content so non-annotation-aware clients see it.
                if let Some(u) = url {
                    if !content_text.is_empty() {
                        content_text.push('\n');
                    }
                    content_text.push_str(u);
                }
            }
            GenerateContent::File {
                data, media_type, ..
            } => {
                if !content_text.is_empty() {
                    content_text.push('\n');
                }
                content_text.push_str(&file_data_to_text(data, media_type));
            }
            GenerateContent::ToolResult {
                tool_name,
                result,
                is_error,
                ..
            } => {
                // Degraded mapping: provider-executed tool result as text.
                if !content_text.is_empty() {
                    content_text.push('\n');
                }
                let prefix = if is_error.unwrap_or(false) {
                    format!("[tool error: {}] ", tool_name)
                } else {
                    format!("[tool result: {}] ", tool_name)
                };
                content_text.push_str(&prefix);
                content_text.push_str(&result.to_string());
            }
        }
    }

    // content: null when there are tool_calls and no text, else the string.
    let content = if content_text.is_empty() && !tool_calls.is_empty() {
        None
    } else {
        Some(content_text)
    };

    let message = ChatCompletionMessage {
        role: "assistant".to_string(),
        content,
        reasoning_content: if reasoning_text.is_empty() {
            None
        } else {
            Some(reasoning_text)
        },
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        annotations: if annotations.is_empty() {
            None
        } else {
            Some(annotations)
        },
    };

    // finish_reason
    let finish_reason = finish_reason_to_openai(&result.finish_reason);

    // logprobs from provider_metadata
    let logprobs = result
        .provider_metadata
        .as_ref()
        .and_then(|pm| pm.get("openai"))
        .and_then(|o| o.get("logprobs"))
        .cloned();

    // id / model / created
    let id = result
        .response
        .id
        .clone()
        .unwrap_or_else(|| format!("chatcmpl-{}", random_id()));
    let model = result
        .response
        .model_id
        .clone()
        .unwrap_or_else(|| model.to_string());
    let created = now_unix();

    ChatCompletion {
        id,
        object: "chat.completion".to_string(),
        created,
        model,
        choices: vec![ChatCompletionChoice {
            index: 0,
            message,
            finish_reason,
            logprobs,
        }],
        usage: usage_to_openai(&result.usage),
        system_fingerprint: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming conversion: Stream<StreamPart> → Stream<ChatCompletionChunk>
// ─────────────────────────────────────────────────────────────────────────────

/// A streaming OpenAI Chat Completions result.
pub struct ChatCompletionStream {
    /// The stream of `ChatCompletionChunk` items.
    pub stream: Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, AiMuxError>> + Send>>,
}

impl std::fmt::Debug for ChatCompletionStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatCompletionStream")
            .field("stream", &"<stream>")
            .finish()
    }
}

/// Convert a `StreamPart` stream into a `ChatCompletionChunk` stream.
///
/// Uses a stateful converter that maintains tool-call indices and accumulates
/// the final usage / finish_reason. The output follows OpenAI SSE conventions:
/// the first chunk carries `delta.role = "assistant"`, content/reasoning/tool
/// deltas follow, and the final chunk carries `finish_reason` (and `usage` if
/// `include_usage` is set).
pub fn to_chat_completion_stream(
    stream: Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>>,
    model: &str,
    options: OpenAiStreamOptions,
) -> ChatCompletionStream {
    let model = model.to_string();
    let include_usage = options.include_usage;
    let include_reasoning = options.include_reasoning;

    let chunk_stream = async_stream::stream! {
        let mut state = StreamState::new(model.clone());

        use futures::StreamExt;
        let mut stream = stream;

        while let Some(part_result) = stream.next().await {
            let part = match part_result {
                Ok(p) => p,
                Err(e) => {
                    // Emit error as a content delta + finish, then stop.
                    if let Some(chunk) = state.error_chunk(&e) {
                        yield Ok(chunk);
                    }
                    if let Some(chunk) = state.final_chunk(include_usage) {
                        yield Ok(chunk);
                    }
                    return;
                }
            };

            let chunks = state.process_part(&part, include_reasoning, include_usage);
            for chunk in chunks {
                yield Ok(chunk);
            }

            // After Finish, stop (but we already emitted the final chunk).
            if matches!(part, StreamPart::Finish { .. }) {
                return;
            }
        }

        // Stream ended without Finish — emit a final chunk.
        if let Some(chunk) = state.final_chunk(include_usage) {
            yield Ok(chunk);
        }
    };

    ChatCompletionStream {
        stream: Box::pin(chunk_stream),
    }
}

/// Internal state for the streaming converter.
struct StreamState {
    id: String,
    model: String,
    created: u64,
    started: bool,
    /// Tool-call accumulators keyed by tool_call_id.
    tool_calls: HashMap<String, ToolCallAccum>,
    tool_call_order: Vec<String>,
    next_tool_index: u32,
    /// Whether each tool_call_id has had its opening chunk emitted.
    tool_call_opened: std::collections::HashSet<String>,
    final_usage: Option<Usage>,
    final_finish_reason: Option<FinishReason>,
    finish_emitted: bool,
}

#[allow(dead_code)]
struct ToolCallAccum {
    index: u32,
    id: String,
    name: String,
}

impl StreamState {
    fn new(model: String) -> Self {
        Self {
            id: format!("chatcmpl-{}", random_id()),
            model,
            created: now_unix(),
            started: false,
            tool_calls: HashMap::new(),
            tool_call_order: Vec::new(),
            next_tool_index: 0,
            tool_call_opened: std::collections::HashSet::new(),
            final_usage: None,
            final_finish_reason: None,
            finish_emitted: false,
        }
    }

    /// Build a base chunk with id/model/created.
    fn base_chunk(&self) -> ChatCompletionChunk {
        ChatCompletionChunk {
            id: self.id.clone(),
            object: "chat.completion.chunk".to_string(),
            created: self.created,
            model: self.model.clone(),
            choices: Vec::new(),
            usage: None,
        }
    }

    /// Ensure the role-frame has been sent; return it if newly created.
    fn ensure_started(&mut self) -> Option<ChatCompletionChunk> {
        if self.started {
            return None;
        }
        self.started = true;
        let mut chunk = self.base_chunk();
        chunk.choices = vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some("assistant".to_string()),
                content: Some(String::new()),
                ..Default::default()
            },
            finish_reason: None,
            logprobs: None,
        }];
        Some(chunk)
    }

    /// Process a single StreamPart, returning zero or more output chunks.
    fn process_part(
        &mut self,
        part: &StreamPart,
        include_reasoning: bool,
        include_usage: bool,
    ) -> Vec<ChatCompletionChunk> {
        let mut chunks = Vec::new();

        match part {
            StreamPart::StreamStart { .. } => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
            }

            StreamPart::ResponseMetadata { id, model_id, .. } => {
                if let Some(id) = id {
                    self.id = id.clone();
                }
                if let Some(m) = model_id {
                    self.model = m.clone();
                }
            }

            StreamPart::TextStart { .. } => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
            }
            StreamPart::TextDelta { delta, .. } => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let mut chunk = self.base_chunk();
                chunk.choices = vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(delta.clone()),
                        ..Default::default()
                    },
                    finish_reason: None,
                    logprobs: None,
                }];
                chunks.push(chunk);
            }
            StreamPart::TextEnd { .. } => {}

            StreamPart::ReasoningStart { .. } => {}
            StreamPart::ReasoningDelta { delta, .. } => {
                if include_reasoning {
                    if let Some(c) = self.ensure_started() {
                        chunks.push(c);
                    }
                    let mut chunk = self.base_chunk();
                    chunk.choices = vec![ChatCompletionChunkChoice {
                        index: 0,
                        delta: ChatCompletionDelta {
                            reasoning_content: Some(delta.clone()),
                            ..Default::default()
                        },
                        finish_reason: None,
                        logprobs: None,
                    }];
                    chunks.push(chunk);
                }
            }
            StreamPart::ReasoningEnd { .. } => {}

            StreamPart::ToolInputStart { id, tool_name, .. } => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                // Assign index.
                let index = if self.tool_calls.contains_key(id) {
                    self.tool_calls[id].index
                } else {
                    let idx = self.next_tool_index;
                    self.next_tool_index += 1;
                    self.tool_calls.insert(
                        id.clone(),
                        ToolCallAccum {
                            index: idx,
                            id: id.clone(),
                            name: tool_name.clone(),
                        },
                    );
                    self.tool_call_order.push(id.clone());
                    idx
                };
                self.tool_call_opened.insert(id.clone());

                let mut chunk = self.base_chunk();
                chunk.choices = vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        tool_calls: Some(vec![ChatCompletionChunkToolCall {
                            index,
                            id: Some(id.clone()),
                            tool_type: Some("function".to_string()),
                            function: ChatCompletionChunkFunction {
                                name: Some(tool_name.clone()),
                                arguments: Some(String::new()),
                            },
                        }]),
                        ..Default::default()
                    },
                    finish_reason: None,
                    logprobs: None,
                }];
                chunks.push(chunk);
            }
            StreamPart::ToolInputDelta { id, delta, .. } => {
                // Ensure started (shouldn't happen without Start, but be safe).
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let index = match self.tool_calls.get(id) {
                    Some(acc) => acc.index,
                    None => {
                        // Delta without Start — allocate a new index.
                        let idx = self.next_tool_index;
                        self.next_tool_index += 1;
                        self.tool_calls.insert(
                            id.clone(),
                            ToolCallAccum {
                                index: idx,
                                id: id.clone(),
                                name: String::new(),
                            },
                        );
                        self.tool_call_order.push(id.clone());
                        idx
                    }
                };

                let mut chunk = self.base_chunk();
                chunk.choices = vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        tool_calls: Some(vec![ChatCompletionChunkToolCall {
                            index,
                            id: None,
                            tool_type: None,
                            function: ChatCompletionChunkFunction {
                                name: None,
                                arguments: Some(delta.clone()),
                            },
                        }]),
                        ..Default::default()
                    },
                    finish_reason: None,
                    logprobs: None,
                }];
                chunks.push(chunk);
            }
            StreamPart::ToolInputEnd { .. } => {}

            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                // Complete tool call (e.g. from non-streaming-style providers).
                // If already opened via ToolInputStart, skip; otherwise emit
                // the full call in one chunk.
                if self.tool_call_opened.contains(tool_call_id) {
                    // Already streamed — the arguments were sent via deltas.
                } else {
                    if let Some(c) = self.ensure_started() {
                        chunks.push(c);
                    }
                    let index = self.next_tool_index;
                    self.next_tool_index += 1;
                    self.tool_calls.insert(
                        tool_call_id.clone(),
                        ToolCallAccum {
                            index,
                            id: tool_call_id.clone(),
                            name: tool_name.clone(),
                        },
                    );
                    self.tool_call_order.push(tool_call_id.clone());
                    self.tool_call_opened.insert(tool_call_id.clone());

                    let arguments = if input.is_null() {
                        "{}".to_string()
                    } else {
                        input.to_string()
                    };

                    let mut chunk = self.base_chunk();
                    chunk.choices = vec![ChatCompletionChunkChoice {
                        index: 0,
                        delta: ChatCompletionDelta {
                            tool_calls: Some(vec![ChatCompletionChunkToolCall {
                                index,
                                id: Some(tool_call_id.clone()),
                                tool_type: Some("function".to_string()),
                                function: ChatCompletionChunkFunction {
                                    name: Some(tool_name.clone()),
                                    arguments: Some(arguments),
                                },
                            }]),
                            ..Default::default()
                        },
                        finish_reason: None,
                        logprobs: None,
                    }];
                    chunks.push(chunk);
                }
            }

            StreamPart::ToolResult {
                tool_name, result, ..
            } => {
                // Degraded: provider-executed tool result as content.
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let text = format!("[tool result: {}] {}", tool_name, result);
                let mut chunk = self.base_chunk();
                chunk.choices = vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(text),
                        ..Default::default()
                    },
                    finish_reason: None,
                    logprobs: None,
                }];
                chunks.push(chunk);
            }

            StreamPart::File {
                data, media_type, ..
            } => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let text = file_data_to_text(data, media_type);
                let mut chunk = self.base_chunk();
                chunk.choices = vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(text),
                        ..Default::default()
                    },
                    finish_reason: None,
                    logprobs: None,
                }];
                chunks.push(chunk);
            }

            StreamPart::Source { url, .. } => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let text = url.clone().unwrap_or_default();
                if !text.is_empty() {
                    let mut chunk = self.base_chunk();
                    chunk.choices = vec![ChatCompletionChunkChoice {
                        index: 0,
                        delta: ChatCompletionDelta {
                            content: Some(text),
                            ..Default::default()
                        },
                        finish_reason: None,
                        logprobs: None,
                    }];
                    chunks.push(chunk);
                }
            }

            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => {
                self.final_usage = Some(usage.clone());
                self.final_finish_reason = Some(finish_reason.clone());
                // Emit the final chunk here.
                if let Some(c) = self.final_chunk_impl(include_usage) {
                    chunks.push(c);
                }
            }

            StreamPart::Error { error } => {
                if let Some(chunk) = self.error_chunk(error) {
                    chunks.push(chunk);
                }
            }

            StreamPart::Raw { .. } => { /* ignore */ }
        }

        chunks
    }

    /// Build an error content chunk.
    fn error_chunk(&mut self, error: &AiMuxError) -> Option<ChatCompletionChunk> {
        if self.finish_emitted {
            return None;
        }
        self.started = true;
        let mut chunk = self.base_chunk();
        chunk.choices = vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                content: Some(format!("[error] {}", error)),
                ..Default::default()
            },
            finish_reason: None,
            logprobs: None,
        }];
        Some(chunk)
    }

    /// Build the final finish chunk (called when the stream ends or on Finish).
    fn final_chunk(&mut self, include_usage: bool) -> Option<ChatCompletionChunk> {
        if self.finish_emitted {
            return None;
        }
        self.final_chunk_impl(include_usage)
    }

    fn final_chunk_impl(&mut self, include_usage: bool) -> Option<ChatCompletionChunk> {
        if self.finish_emitted {
            return None;
        }
        self.finish_emitted = true;
        if !self.started {
            self.started = true;
        }

        let finish_reason_str = self
            .final_finish_reason
            .as_ref()
            .map(finish_reason_to_openai)
            .unwrap_or(Some("stop".to_string()));

        let usage = if include_usage {
            self.final_usage.as_ref().map(usage_to_openai)
        } else {
            None
        };

        let mut chunk = self.base_chunk();
        chunk.choices = vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta::default(),
            finish_reason: finish_reason_str,
            logprobs: None,
        }];
        chunk.usage = usage;
        Some(chunk)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SSE encoding helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Encode a [`ChatCompletionChunk`] as an SSE `data:` line: `data: {json}\n\n`.
pub fn encode_chunk_sse(chunk: &ChatCompletionChunk) -> String {
    let json = serde_json::to_string(chunk).unwrap_or_else(|_| "{}".to_string());
    format!("data: {}\n\n", json)
}

/// The SSE terminator frame.
pub const DONE_FRAME: &str = "data: [DONE]\n\n";

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Map an aimux [`FinishReason`] to an OpenAI `finish_reason` string.
fn finish_reason_to_openai(reason: &FinishReason) -> Option<String> {
    match reason.unified {
        FinishReasonUnified::Stop => Some("stop".to_string()),
        FinishReasonUnified::Length => Some("length".to_string()),
        FinishReasonUnified::ContentFilter => Some("content_filter".to_string()),
        FinishReasonUnified::ToolCalls => Some("tool_calls".to_string()),
        FinishReasonUnified::Error => Some("stop".to_string()),
        FinishReasonUnified::Other => reason.raw.clone().or(Some("stop".to_string())),
    }
}

/// Convert aimux [`Usage`] to OpenAI [`ChatCompletionUsage`].
///
/// This is the inverse of `convert_usage` in
/// `aimux-providers/src/openai/model.rs`.
fn usage_to_openai(usage: &Usage) -> ChatCompletionUsage {
    let prompt_tokens = usage.input_tokens.total.unwrap_or(0);
    let completion_tokens = usage.output_tokens.total.unwrap_or(0);

    let cached_tokens = usage.input_tokens.cache_read.unwrap_or(0);
    let cache_write = usage.input_tokens.cache_write;
    let reasoning_tokens = usage.output_tokens.reasoning;

    ChatCompletionUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
        prompt_tokens_details: Some(PromptTokensDetails {
            cached_tokens,
            cache_write_tokens: cache_write,
        }),
        completion_tokens_details: Some(CompletionTokensDetails { reasoning_tokens }),
    }
}

/// Convert [`FileData`] to a text representation (degraded — placed in content).
fn file_data_to_text(data: &FileData, media_type: &str) -> String {
    match data {
        FileData::Url { url } => url.clone(),
        FileData::Data { data } => match data {
            crate::shared::FileBytes::Base64(b64) => {
                format!("data:{};base64,{}", media_type, b64)
            }
            crate::shared::FileBytes::Binary(bytes) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                format!("data:{};base64,{}", media_type, b64)
            }
        },
        FileData::Reference { reference } => serde_json::to_string(reference).unwrap_or_default(),
        FileData::Text { text } => text.clone(),
    }
}

/// Current Unix timestamp.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Generate a short random ID (24 hex chars, similar to OpenAI's chatcmpl IDs).
fn random_id() -> String {
    // Use a simple counter + timestamp for deterministic-enough uniqueness.
    // This is not cryptographically random — it only needs to be unique within
    // a process for response identification.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = now_unix();
    format!("{:012x}{:012x}", ts, count)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::GenerateResult;
    use crate::types::TokenUsage;
    use crate::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};
    use futures::StreamExt;
    use serde_json::json;

    fn make_result(content: Vec<GenerateContent>) -> GenerateResult {
        GenerateResult {
            content,
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            },
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: ResponseMetadata {
                id: Some("chatcmpl-test123".to_string()),
                timestamp: None,
                model_id: Some("gpt-4o".to_string()),
            },
            request_body: None,
            response_headers: None,
        }
    }

    #[test]
    fn test_text_only_non_streaming() {
        let result = make_result(vec![GenerateContent::Text {
            text: "Hello world".to_string(),
            provider_metadata: None,
        }]);
        let completion = to_chat_completion(&result, "gpt-4o");

        assert_eq!(completion.object, "chat.completion");
        assert_eq!(completion.id, "chatcmpl-test123");
        assert_eq!(completion.model, "gpt-4o");
        assert_eq!(completion.choices.len(), 1);
        assert_eq!(completion.choices[0].index, 0);
        assert_eq!(
            completion.choices[0].message.content.as_deref(),
            Some("Hello world")
        );
        assert_eq!(completion.choices[0].message.role, "assistant");
        assert_eq!(completion.choices[0].finish_reason.as_deref(), Some("stop"));
        assert!(completion.choices[0].message.tool_calls.is_none());
        assert!(completion.choices[0].message.reasoning_content.is_none());
    }

    #[test]
    fn test_tool_call_non_streaming() {
        let result = make_result(vec![
            GenerateContent::Text {
                text: "Let me check.".to_string(),
                provider_metadata: None,
            },
            GenerateContent::ToolCall {
                tool_call_id: "call_abc".to_string(),
                tool_name: "get_weather".to_string(),
                input: json!({"city": "Tokyo"}),
                provider_executed: None,
                dynamic: None,
                thought_signature: None,
                provider_metadata: None,
            },
        ]);
        // Set finish_reason to ToolCalls.
        let mut result = result;
        result.finish_reason = FinishReason {
            unified: FinishReasonUnified::ToolCalls,
            raw: None,
        };

        let completion = to_chat_completion(&result, "gpt-4o");

        let msg = &completion.choices[0].message;
        assert_eq!(msg.content.as_deref(), Some("Let me check."));
        let tool_calls = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_abc");
        assert_eq!(tool_calls[0].tool_type, "function");
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(tool_calls[0].function.arguments, r#"{"city":"Tokyo"}"#);
        assert_eq!(
            completion.choices[0].finish_reason.as_deref(),
            Some("tool_calls")
        );
    }

    #[test]
    fn test_tool_call_null_content() {
        // Tool call with no text → content should be null.
        let result = make_result(vec![GenerateContent::ToolCall {
            tool_call_id: "call_abc".to_string(),
            tool_name: "get_weather".to_string(),
            input: json!({"city": "Tokyo"}),
            provider_executed: None,
            dynamic: None,
            thought_signature: None,
            provider_metadata: None,
        }]);
        let completion = to_chat_completion(&result, "gpt-4o");

        assert_eq!(completion.choices[0].message.content, None);
        assert!(completion.choices[0].message.tool_calls.is_some());
    }

    #[test]
    fn test_reasoning_non_streaming() {
        let result = make_result(vec![
            GenerateContent::Reasoning {
                text: "Thinking...".to_string(),
                provider_metadata: None,
            },
            GenerateContent::Text {
                text: "Answer".to_string(),
                provider_metadata: None,
            },
        ]);
        let completion = to_chat_completion(&result, "gpt-4o");

        assert_eq!(
            completion.choices[0].message.reasoning_content.as_deref(),
            Some("Thinking...")
        );
        assert_eq!(
            completion.choices[0].message.content.as_deref(),
            Some("Answer")
        );
    }

    #[test]
    fn test_usage_round_trip() {
        let usage = Usage {
            input_tokens: TokenUsage {
                total: Some(100),
                cache_read: Some(30),
                cache_write: Some(10),
                ..Default::default()
            },
            output_tokens: TokenUsage {
                total: Some(50),
                reasoning: Some(20),
                ..Default::default()
            },
            raw: None,
        };
        let mut result = make_result(vec![]);
        result.usage = usage;

        let completion = to_chat_completion(&result, "gpt-4o");
        let u = &completion.usage;

        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 50);
        assert_eq!(u.total_tokens, 150);
        assert_eq!(u.prompt_tokens_details.as_ref().unwrap().cached_tokens, 30);
        assert_eq!(
            u.prompt_tokens_details.as_ref().unwrap().cache_write_tokens,
            Some(10)
        );
        assert_eq!(
            u.completion_tokens_details
                .as_ref()
                .unwrap()
                .reasoning_tokens,
            Some(20)
        );
    }

    #[test]
    fn test_finish_reason_mapping() {
        let cases = [
            (FinishReasonUnified::Stop, "stop"),
            (FinishReasonUnified::Length, "length"),
            (FinishReasonUnified::ContentFilter, "content_filter"),
            (FinishReasonUnified::ToolCalls, "tool_calls"),
            (FinishReasonUnified::Error, "stop"),
        ];
        for (unified, expected) in cases {
            let fr = FinishReason { unified, raw: None };
            assert_eq!(finish_reason_to_openai(&fr).as_deref(), Some(expected));
        }
    }

    // ── Streaming tests ──

    async fn collect_stream(stream: ChatCompletionStream) -> Vec<ChatCompletionChunk> {
        let mut chunks = Vec::new();
        let mut s = stream.stream;
        while let Some(result) = s.next().await {
            chunks.push(result.unwrap());
        }
        chunks
    }

    #[tokio::test]
    async fn test_stream_text_only() {
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::TextStart {
                id: "0".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::TextDelta {
                id: "0".to_string(),
                delta: "Hello ".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::TextDelta {
                id: "0".to_string(),
                delta: "world".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::TextEnd {
                id: "0".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ];

        let input_stream = Box::pin(futures::stream::iter(parts));
        let result =
            to_chat_completion_stream(input_stream, "gpt-4o", OpenAiStreamOptions::default());
        let chunks = collect_stream(result).await;

        // First chunk: role frame.
        assert_eq!(
            chunks[0].choices[0].delta.role.as_deref(),
            Some("assistant")
        );
        assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some(""));

        // Content deltas.
        assert_eq!(
            chunks[1].choices[0].delta.content.as_deref(),
            Some("Hello ")
        );
        assert_eq!(chunks[2].choices[0].delta.content.as_deref(), Some("world"));

        // Final chunk: finish_reason.
        let last = chunks.last().unwrap();
        assert_eq!(last.choices[0].finish_reason.as_deref(), Some("stop"));
        assert!(last.choices[0].delta.content.is_none());
    }

    #[tokio::test]
    async fn test_stream_tool_calls() {
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::ToolInputStart {
                id: "call_1".to_string(),
                tool_name: "get_weather".to_string(),
                provider_executed: None,
                dynamic: None,
                title: None,
                provider_metadata: None,
            }),
            Ok(StreamPart::ToolInputDelta {
                id: "call_1".to_string(),
                delta: "{\"city\":".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ToolInputDelta {
                id: "call_1".to_string(),
                delta: "\"Tokyo\"}".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ToolInputEnd {
                id: "call_1".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::ToolCalls,
                    raw: None,
                },
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ];

        let input_stream = Box::pin(futures::stream::iter(parts));
        let result =
            to_chat_completion_stream(input_stream, "gpt-4o", OpenAiStreamOptions::default());
        let chunks = collect_stream(result).await;

        // Find the tool_call opening chunk (with id + name).
        let open_chunk = chunks
            .iter()
            .find(|c| {
                c.choices
                    .first()
                    .and_then(|ch| ch.delta.tool_calls.as_ref())
                    .and_then(|tcs| tcs.first())
                    .and_then(|tc| tc.id.as_deref())
                    == Some("call_1")
            })
            .expect("tool call opening chunk not found");

        let tc = open_chunk.choices[0]
            .delta
            .tool_calls
            .as_ref()
            .unwrap()
            .first()
            .unwrap();
        assert_eq!(tc.index, 0);
        assert_eq!(tc.id.as_deref(), Some("call_1"));
        assert_eq!(tc.tool_type.as_deref(), Some("function"));
        assert_eq!(tc.function.name.as_deref(), Some("get_weather"));

        // Find argument delta chunks.
        let arg_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| {
                c.choices
                    .first()
                    .and_then(|ch| ch.delta.tool_calls.as_ref())
                    .and_then(|tcs| tcs.first())
                    .is_some_and(|tc| tc.id.is_none() && tc.function.arguments.is_some())
            })
            .collect();
        assert_eq!(arg_chunks.len(), 2);
        assert_eq!(
            arg_chunks[0].choices[0].delta.tool_calls.as_ref().unwrap()[0]
                .function
                .arguments
                .as_deref(),
            Some("{\"city\":")
        );

        // Final chunk.
        let last = chunks.last().unwrap();
        assert_eq!(last.choices[0].finish_reason.as_deref(), Some("tool_calls"));
    }

    #[tokio::test]
    async fn test_stream_multiple_tool_calls_index() {
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::ToolInputStart {
                id: "call_a".to_string(),
                tool_name: "tool_a".to_string(),
                provider_executed: None,
                dynamic: None,
                title: None,
                provider_metadata: None,
            }),
            Ok(StreamPart::ToolInputStart {
                id: "call_b".to_string(),
                tool_name: "tool_b".to_string(),
                provider_executed: None,
                dynamic: None,
                title: None,
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::ToolCalls,
                    raw: None,
                },
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ];

        let input_stream = Box::pin(futures::stream::iter(parts));
        let result =
            to_chat_completion_stream(input_stream, "gpt-4o", OpenAiStreamOptions::default());
        let chunks = collect_stream(result).await;

        // Find both tool call opening chunks.
        let open_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| {
                c.choices
                    .first()
                    .and_then(|ch| ch.delta.tool_calls.as_ref())
                    .and_then(|tcs| tcs.first())
                    .and_then(|tc| tc.id.as_deref())
                    .is_some()
            })
            .collect();
        assert_eq!(open_chunks.len(), 2);

        let tc0 = &open_chunks[0].choices[0].delta.tool_calls.as_ref().unwrap()[0];
        let tc1 = &open_chunks[1].choices[0].delta.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc0.index, 0);
        assert_eq!(tc0.id.as_deref(), Some("call_a"));
        assert_eq!(tc1.index, 1);
        assert_eq!(tc1.id.as_deref(), Some("call_b"));
    }

    #[tokio::test]
    async fn test_stream_reasoning() {
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::ReasoningStart {
                id: "reasoning-0".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningDelta {
                id: "reasoning-0".to_string(),
                delta: "Hmm...".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningEnd {
                id: "reasoning-0".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::TextDelta {
                id: "0".to_string(),
                delta: "Answer".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ];

        let input_stream = Box::pin(futures::stream::iter(parts));
        let result =
            to_chat_completion_stream(input_stream, "gpt-4o", OpenAiStreamOptions::default());
        let chunks = collect_stream(result).await;

        // Find reasoning chunk.
        let reasoning_chunk = chunks
            .iter()
            .find(|c| c.choices[0].delta.reasoning_content.is_some())
            .expect("reasoning chunk not found");
        assert_eq!(
            reasoning_chunk.choices[0]
                .delta
                .reasoning_content
                .as_deref(),
            Some("Hmm...")
        );

        // Find text chunk.
        let text_chunk = chunks
            .iter()
            .find(|c| {
                c.choices[0]
                    .delta
                    .content
                    .as_deref()
                    .is_some_and(|s| s == "Answer")
            })
            .expect("text chunk not found");
        assert_eq!(
            text_chunk.choices[0].delta.content.as_deref(),
            Some("Answer")
        );
    }

    #[tokio::test]
    async fn test_stream_usage_in_final_chunk() {
        let usage = Usage {
            input_tokens: TokenUsage {
                total: Some(42),
                ..Default::default()
            },
            output_tokens: TokenUsage {
                total: Some(10),
                ..Default::default()
            },
            raw: None,
        };

        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::TextDelta {
                id: "0".to_string(),
                delta: "Hi".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage,
                provider_metadata: None,
            }),
        ];

        let input_stream = Box::pin(futures::stream::iter(parts));
        let result = to_chat_completion_stream(
            input_stream,
            "gpt-4o",
            OpenAiStreamOptions {
                include_usage: true,
                include_reasoning: true,
            },
        );
        let chunks = collect_stream(result).await;

        let last = chunks.last().unwrap();
        assert_eq!(last.usage.as_ref().unwrap().prompt_tokens, 42);
        assert_eq!(last.usage.as_ref().unwrap().completion_tokens, 10);
        assert_eq!(last.usage.as_ref().unwrap().total_tokens, 52);
    }

    #[tokio::test]
    async fn test_stream_no_usage_when_disabled() {
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: Usage {
                    input_tokens: TokenUsage {
                        total: Some(42),
                        ..Default::default()
                    },
                    output_tokens: TokenUsage::default(),
                    raw: None,
                },
                provider_metadata: None,
            }),
        ];

        let input_stream = Box::pin(futures::stream::iter(parts));
        let result = to_chat_completion_stream(
            input_stream,
            "gpt-4o",
            OpenAiStreamOptions {
                include_usage: false,
                include_reasoning: true,
            },
        );
        let chunks = collect_stream(result).await;

        let last = chunks.last().unwrap();
        assert!(last.usage.is_none());
    }

    #[tokio::test]
    async fn test_stream_error() {
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::TextDelta {
                id: "0".to_string(),
                delta: "partial".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Error {
                error: AiMuxError::Provider("something went wrong".to_string()),
            }),
        ];

        let input_stream = Box::pin(futures::stream::iter(parts));
        let result =
            to_chat_completion_stream(input_stream, "gpt-4o", OpenAiStreamOptions::default());
        let chunks = collect_stream(result).await;

        // Error should appear as a content delta.
        let error_chunk = chunks
            .iter()
            .find(|c| {
                c.choices[0]
                    .delta
                    .content
                    .as_deref()
                    .is_some_and(|s| s.contains("[error]"))
            })
            .expect("error chunk not found");

        assert!(
            error_chunk.choices[0]
                .delta
                .content
                .as_deref()
                .unwrap()
                .contains("something went wrong")
        );

        // Final chunk should still have finish_reason.
        let last = chunks.last().unwrap();
        assert_eq!(last.choices[0].finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn test_stream_completes_without_finish() {
        // Stream ends without Finish — should still emit a final chunk.
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::TextDelta {
                id: "0".to_string(),
                delta: "Hello".to_string(),
                provider_metadata: None,
            }),
        ];

        let input_stream = Box::pin(futures::stream::iter(parts));
        let result =
            to_chat_completion_stream(input_stream, "gpt-4o", OpenAiStreamOptions::default());
        let chunks = collect_stream(result).await;

        let last = chunks.last().unwrap();
        assert_eq!(last.choices[0].finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn test_sse_encoding() {
        let chunk = ChatCompletionChunk {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 1234567890,
            model: "gpt-4o".to_string(),
            choices: vec![ChatCompletionChunkChoice {
                index: 0,
                delta: ChatCompletionDelta {
                    content: Some("Hi".to_string()),
                    ..Default::default()
                },
                finish_reason: None,
                logprobs: None,
            }],
            usage: None,
        };

        let sse = encode_chunk_sse(&chunk);
        assert!(sse.starts_with("data: {"));
        assert!(sse.ends_with("\n\n"));
        assert!(sse.contains("\"content\":\"Hi\""));
    }

    #[test]
    fn test_done_frame() {
        assert_eq!(DONE_FRAME, "data: [DONE]\n\n");
    }
}
