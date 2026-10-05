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
use crate::parse_tool_call::raw_tool_call_text;
use crate::result::{GenerateContent, GenerateResult, GeneratedFile, ReasoningOutput, Source};
use crate::shared::GeneratedFileData;
use crate::stream_part::{StreamPart, TextStreamPart};
use crate::tool::{ToolCall, ToolResult};
use crate::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

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
#[must_use]
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
            GenerateContent::Reasoning(ReasoningOutput { text, .. }) => {
                reasoning_text.push_str(text);
            }
            GenerateContent::ToolCall(call) => {
                // The provider's raw argument text passes through verbatim;
                // OpenAI's wire format requires a JSON object even when the
                // model emitted no arguments at all.
                let arguments = if call.input.trim().is_empty() {
                    "{}".to_string()
                } else {
                    call.input.clone()
                };
                tool_calls.push(ChatCompletionToolCall {
                    id: call.tool_call_id.clone(),
                    tool_type: "function".to_string(),
                    function: ChatCompletionFunction {
                        name: call.tool_name.clone(),
                        arguments,
                    },
                });
            }
            GenerateContent::Source(Source::Url { url, title, .. }) => {
                // Map to OpenAI url_citation annotation.
                let mut ann = serde_json::json!({
                    "type": "url_citation",
                    "url_citation": {
                        "url": url,
                    }
                });
                if let Some(t) = title {
                    ann["url_citation"]["title"] = serde_json::Value::String(t.clone());
                }
                annotations.push(ann);
                // Also append URL to content so non-annotation-aware clients see it.
                if !content_text.is_empty() {
                    content_text.push('\n');
                }
                content_text.push_str(url);
            }
            GenerateContent::File(GeneratedFile {
                data, media_type, ..
            }) => {
                if !content_text.is_empty() {
                    content_text.push('\n');
                }
                content_text.push_str(&file_data_to_text(data, media_type));
            }
            GenerateContent::Custom { .. }
            | GenerateContent::Source(Source::Document { .. })
            | GenerateContent::ReasoningFile(_)
            | GenerateContent::ToolApprovalRequest(_) => {}
            GenerateContent::ToolResult(ToolResult {
                tool_name,
                result,
                is_error,
                ..
            }) => {
                // Degraded mapping: provider-executed tool result as text.
                if !content_text.is_empty() {
                    content_text.push('\n');
                }
                let prefix = if is_error.unwrap_or(false) {
                    format!("[tool error: {tool_name}] ")
                } else {
                    format!("[tool result: {tool_name}] ")
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
        .as_ref()
        .and_then(|response| response.id.clone())
        .unwrap_or_else(|| format!("chatcmpl-{}", random_id()));
    let model = result
        .response
        .as_ref()
        .and_then(|response| response.model_id.clone())
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
///
/// Tool-input deltas are forwarded as the provider sends them, as in the AI
/// SDK; they are never held back. See `stream_text_as_openai`.
#[must_use]
pub fn to_chat_completion_stream(
    stream: Pin<Box<dyn Stream<Item = Result<TextStreamPart, AiMuxError>> + Send>>,
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
                Err(e) if e.is_recoverable_stream_error() => {
                    yield Err(e);
                    continue;
                }
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
    /// Argument bytes already emitted through OpenAI delta chunks.
    arguments: String,
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
        part: &TextStreamPart,
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

            StreamPart::ResponseMetadata(ResponseMetadata { id, model_id, .. }) => {
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
                            arguments: String::new(),
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
                let index = match self.tool_calls.get_mut(id) {
                    Some(acc) => {
                        acc.arguments.push_str(delta);
                        acc.index
                    }
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
                                arguments: delta.clone(),
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

            StreamPart::ToolCall(ToolCall {
                tool_call_id,
                tool_name,
                input,
                invalid,
                error,
                ..
            }) => {
                // Complete tool call (e.g. from non-streaming-style providers).
                // A provider may carry all input on its start frame and emit no
                // deltas. In that case the final call is the first point where
                // the OpenAI adapter can forward those arguments.
                if self.tool_call_opened.contains(tool_call_id) {
                    let full_arguments =
                        parsed_tool_call_arguments(input, *invalid, error.as_ref());
                    let missing_arguments = self
                        .tool_calls
                        .get(tool_call_id)
                        .and_then(|acc| full_arguments.strip_prefix(&acc.arguments))
                        .unwrap_or_default()
                        .to_string();

                    if !missing_arguments.is_empty() {
                        let index = self.tool_calls[tool_call_id].index;
                        if let Some(acc) = self.tool_calls.get_mut(tool_call_id) {
                            acc.arguments.push_str(&missing_arguments);
                        }
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
                                        arguments: Some(missing_arguments),
                                    },
                                }]),
                                ..Default::default()
                            },
                            finish_reason: None,
                            logprobs: None,
                        }];
                        chunks.push(chunk);
                    }
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
                            arguments: parsed_tool_call_arguments(input, *invalid, error.as_ref()),
                        },
                    );
                    self.tool_call_order.push(tool_call_id.clone());
                    self.tool_call_opened.insert(tool_call_id.clone());

                    let arguments = self.tool_calls[tool_call_id].arguments.clone();

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

            StreamPart::ToolResult(ToolResult {
                tool_name, result, ..
            }) => {
                // Degraded: provider-executed tool result as content.
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let text = format!("[tool result: {tool_name}] {result}");
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

            StreamPart::File(GeneratedFile {
                data, media_type, ..
            }) => {
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

            StreamPart::Source(Source::Url { url, .. }) => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let text = url.clone();
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

            StreamPart::Raw { .. }
            | StreamPart::Source(Source::Document { .. })
            | StreamPart::Custom { .. }
            | StreamPart::ReasoningFile(_)
            | StreamPart::ToolApprovalRequest(_) => { /* no OpenAI chat equivalent */ }
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
                content: Some(format!("[error] {error}")),
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
#[must_use]
pub fn encode_chunk_sse(chunk: &ChatCompletionChunk) -> String {
    let json = serde_json::to_string(chunk).unwrap_or_else(|_| "{}".to_string());
    format!("data: {json}\n\n")
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

/// Convert [`GeneratedFileData`] to a text representation (degraded — placed in content).
fn file_data_to_text(data: &GeneratedFileData, media_type: &str) -> String {
    match data {
        GeneratedFileData::Url { url, .. } => url.clone(),
        GeneratedFileData::Data { data } => match data {
            crate::shared::FileBytes::Base64(b64) => {
                format!("data:{media_type};base64,{b64}")
            }
            crate::shared::FileBytes::Binary(bytes) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                format!("data:{media_type};base64,{b64}")
            }
        },
    }
}

/// Current Unix timestamp.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Rejected repairs retain the original call. Recover its text from the
// original error, never from the replacement's validation failure.
/// Render a Core-parsed `StreamPart::ToolCall`'s arguments as OpenAI-compatible
/// wire text: the provider's raw argument text verbatim for an invalid call,
/// compact JSON of the parsed value otherwise.
///
/// Older errors without raw argument text fall back to serializing the
/// parsed input as JSON.
pub(crate) fn parsed_tool_call_arguments(
    input: &Value,
    invalid: Option<bool>,
    error: Option<&AiMuxError>,
) -> String {
    if invalid == Some(true)
        && let Some(error) = error
        && let Some(raw) = raw_tool_call_text(error)
    {
        // Blank text still fails validation against a schema with required
        // properties, but OpenAI's wire format has no representation for
        // "no arguments" other than an empty object — same rule as
        // `to_chat_completion` applies to the unparsed non-streaming path.
        return if raw.trim().is_empty() {
            "{}".to_string()
        } else {
            raw
        };
    }
    // `Value` Display is compact JSON, the `JSON.stringify` equivalent —
    // correct for every shape, `null` included.
    input.to_string()
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
    format!("{ts:012x}{count:012x}")
}
