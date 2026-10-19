//! OpenAI Chat Completions output format.
//!
//! Converts aimux's internal types ([`GenerateResult`] / [`TextStreamPart`]) into
//! standard OpenAI Chat Completions structures ([`ChatCompletion`] /
//! [`ChatCompletionChunk`]). This lets any provider (OpenAI, Anthropic, Google,
//! …) be consumed via the OpenAI wire format.
//!
//! # Architecture
//!
//! ```text
//!   provider.do_generate()  →  GenerateResult  →  to_chat_completion()  →  ChatCompletion
//!   stream_text()           →  Stream<TextStreamPart>  →  to_chat_completion_stream()  →  Stream<Chunk>
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
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::Stream;
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::generate::GenerateTextResult;
use crate::language_model::{LanguageModel, SupportedUrls};
use crate::options::CallOptions;
use crate::parse_tool_call::original_tool_call_text;
use crate::result::StreamResult;
use crate::result::{
    ContentPart, GenerateContent, GenerateResult, GeneratedFileOutput, ReasoningOutput, Source,
    TypedToolError, TypedToolResult,
};
use crate::shared::GeneratedFileData;
use crate::stream_part::{StreamPart, TextStreamPart};
use crate::tool::ToolCall;
use crate::types::{
    FinishReasonUnified, LanguageModelUsage, ResponseMetadata, as_language_model_usage,
};

// ─────────────────────────────────────────────────────────────────────────────
// Non-streaming response types
// ─────────────────────────────────────────────────────────────────────────────

/// A complete Chat Completion response (non-streaming).
///
/// Mirrors the OpenAI `chat.completion` object.
#[derive(Debug, Clone, Serialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ChatCompletionChoice {
    pub index: u32,
    pub message: ChatCompletionMessage,
    pub finish_reason: Option<String>,
    /// The Chat Completions logprobs object, `{ "content": [...], "refusal":
    /// null }`, each entry `{ token, logprob, bytes, top_logprobs }`, when
    /// the calling provider returned logprobs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ChatCompletionToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ChatCompletionFunction,
}

#[derive(Debug, Clone, Serialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ChatCompletionChunkChoice {
    pub index: u32,
    pub delta: ChatCompletionDelta,
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ChatCompletionChunkToolCall {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    pub function: ChatCompletionChunkFunction,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct PromptTokensDetails {
    pub cached_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
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
    let content = result.content.iter().map(|item| match item {
        GenerateContent::Text { text, .. } => CompletionContent::Text(text),
        GenerateContent::Reasoning(ReasoningOutput { text, .. }) => {
            CompletionContent::Reasoning(text)
        }
        // The provider's raw argument text passes through verbatim; OpenAI's
        // wire format requires a JSON object even when the model emitted no
        // arguments at all.
        GenerateContent::ToolCall(call) => CompletionContent::ToolCall {
            id: &call.tool_call_id,
            name: &call.tool_name,
            arguments: if call.input.trim().is_empty() {
                "{}".to_string()
            } else {
                call.input.clone()
            },
        },
        GenerateContent::Source(source) => CompletionContent::Source(source),
        GenerateContent::File(file) => {
            CompletionContent::File(file_data_to_text(&file.data, &file.media_type))
        }
        GenerateContent::ToolResult(result) => CompletionContent::ToolResult {
            tool_name: &result.tool_name,
            output: &result.result,
            is_error: result.is_error == Some(true),
        },
        GenerateContent::Custom { .. }
        | GenerateContent::ReasoningFile(_)
        | GenerateContent::ToolApprovalRequest(_) => CompletionContent::None,
    });
    let response = result.response.as_ref();
    build_chat_completion(
        content,
        finish_reason_to_openai(
            result.finish_reason.unified,
            result.finish_reason.raw.as_deref(),
        ),
        result.provider_metadata.as_ref(),
        response.and_then(|response| response.id.clone()),
        response
            .and_then(|response| response.model_id.clone())
            .unwrap_or_else(|| model.to_string()),
        usage_to_openai(&as_language_model_usage(&result.usage)),
    )
}

/// Convert a [`GenerateTextResult`] into an OpenAI [`ChatCompletion`]: the
/// content of every step, the last step's finish reason, id, model and
/// logprobs, and the usage summed over the steps. Tool calls carry the
/// Core-parsed (and possibly repaired) arguments.
#[must_use]
pub(crate) fn text_result_to_chat_completion(
    result: &GenerateTextResult,
    model: &str,
) -> ChatCompletion {
    let content = result.content().into_iter().map(|item| match item {
        ContentPart::Text { text, .. } => CompletionContent::Text(text),
        ContentPart::Reasoning(ReasoningOutput { text, .. }) => CompletionContent::Reasoning(text),
        ContentPart::ToolCall(call) => CompletionContent::ToolCall {
            id: &call.tool_call_id,
            name: &call.tool_name,
            arguments: parsed_tool_call_arguments(&call.input, call.invalid, call.error.as_ref()),
        },
        ContentPart::Source(source) => CompletionContent::Source(source),
        ContentPart::File { file, .. } => {
            CompletionContent::File(format!("data:{};base64,{}", file.media_type, file.base64()))
        }
        ContentPart::ToolResult(result) => CompletionContent::ToolResult {
            tool_name: &result.tool_name,
            output: &result.output,
            is_error: false,
        },
        // A provider's tool error renders as the generated path renders it;
        // the error of a call that could not be parsed has no Chat
        // Completions form.
        ContentPart::ToolError(error) if error.provider_executed == Some(true) => {
            CompletionContent::ToolResult {
                tool_name: &error.tool_name,
                output: &error.error,
                is_error: true,
            }
        }
        ContentPart::Custom { .. }
        | ContentPart::ReasoningFile(_)
        | ContentPart::ToolError(_)
        | ContentPart::ToolApprovalRequest(_)
        | ContentPart::ToolApprovalResponse(_) => CompletionContent::None,
    });
    let final_step = result.final_step();
    let model = if final_step.response.model_id.is_empty() {
        model.to_string()
    } else {
        final_step.response.model_id.clone()
    };
    build_chat_completion(
        content,
        finish_reason_to_openai(
            final_step.finish_reason,
            final_step.raw_finish_reason.as_deref(),
        ),
        final_step.provider_metadata.as_ref(),
        Some(final_step.response.id.clone()).filter(|id| !id.is_empty()),
        model,
        usage_to_openai(result.usage()),
    )
}

/// A content item as the non-streaming converter maps it.
enum CompletionContent<'a> {
    Text(&'a str),
    Reasoning(&'a str),
    ToolCall {
        id: &'a str,
        name: &'a str,
        arguments: String,
    },
    Source(&'a Source),
    /// A generated file as text: its URL or a `data:` URL.
    File(String),
    /// A provider-executed tool result or error.
    ToolResult {
        tool_name: &'a str,
        output: &'a serde_json::Value,
        is_error: bool,
    },
    /// No Chat Completions equivalent.
    None,
}

fn build_chat_completion<'a>(
    content: impl Iterator<Item = CompletionContent<'a>>,
    finish_reason: Option<String>,
    provider_metadata: Option<&crate::types::ProviderMetadata>,
    id: Option<String>,
    model: String,
    usage: ChatCompletionUsage,
) -> ChatCompletion {
    let mut content_text = String::new();
    let mut reasoning_text = String::new();
    let mut tool_calls = Vec::new();
    let mut annotations = Vec::new();

    for item in content {
        match item {
            CompletionContent::Text(text) => {
                content_text.push_str(text);
            }
            CompletionContent::Reasoning(text) => {
                reasoning_text.push_str(text);
            }
            CompletionContent::ToolCall {
                id,
                name,
                arguments,
            } => {
                tool_calls.push(ChatCompletionToolCall {
                    id: id.to_string(),
                    tool_type: "function".to_string(),
                    function: ChatCompletionFunction {
                        name: name.to_string(),
                        arguments,
                    },
                });
            }
            CompletionContent::Source(Source::Url { url, title, .. }) => {
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
            CompletionContent::File(text) => {
                if !content_text.is_empty() {
                    content_text.push('\n');
                }
                content_text.push_str(&text);
            }
            CompletionContent::Source(Source::Document { .. }) | CompletionContent::None => {}
            CompletionContent::ToolResult {
                tool_name,
                output,
                is_error,
            } => {
                // Degraded mapping: provider-executed tool result as text.
                if !content_text.is_empty() {
                    content_text.push('\n');
                }
                let prefix = if is_error {
                    format!("[tool error: {tool_name}] ")
                } else {
                    format!("[tool result: {tool_name}] ")
                };
                content_text.push_str(&prefix);
                content_text.push_str(&output.to_string());
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

    // logprobs from the calling provider's metadata namespace. Upstream
    // writes them under the provider's own key, not a fixed `openai`: the
    // Responses model under `providerOptionsName` (`azure` for Azure,
    // @ai-sdk/openai 4.0.83 src/responses/openai-responses-language-model.ts:324,
    // :1080), DeepSeek under `deepseek`. One model call carries one
    // provider's namespace, so the namespace holding `logprobs` is the
    // caller's.
    let logprobs = provider_metadata
        .and_then(|pm| {
            pm.values()
                .find_map(|namespace| namespace.get("logprobs").filter(|value| !value.is_null()))
        })
        .and_then(chat_logprobs);

    ChatCompletion {
        id: id.unwrap_or_else(|| format!("chatcmpl-{}", random_id())),
        object: "chat.completion".to_string(),
        created: now_unix(),
        model,
        choices: vec![ChatCompletionChoice {
            index: 0,
            message,
            finish_reason,
            logprobs,
        }],
        usage,
        system_fingerprint: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming conversion: Stream<TextStreamPart> → Stream<ChatCompletionChunk>
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

/// Convert a `TextStreamPart` stream into a `ChatCompletionChunk` stream.
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
    chat_completion_stream(stream, model, options, None)
}

/// The response id and model a provider reported in its stream
/// (`StreamPart::ResponseMetadata`), recorded by the model that
/// [`observe`](Self::observe) returns. `stream_text` folds that part into the
/// step result, so its `TextStreamPart` stream carries the id and model only
/// at the end of the step.
#[derive(Clone, Debug, Default)]
pub struct ResponseIdentity(Arc<Mutex<Option<ResponseMetadata>>>);

impl ResponseIdentity {
    /// `model`, recording the `ResponseMetadata` of its streams here. A
    /// transparent decorator: everything else is forwarded.
    #[must_use]
    pub fn observe<'a>(&self, model: &'a dyn LanguageModel) -> ResponseIdentityTap<'a> {
        ResponseIdentityTap {
            model,
            identity: self.clone(),
        }
    }

    fn reported(&self) -> Option<ResponseMetadata> {
        self.0.lock().ok()?.clone()
    }
}

/// [`to_chat_completion_stream`] whose chunks carry the provider's response id
/// and model once `identity` holds them (RFC-0026 §4.4: `ResponseMetadata`
/// records id/model). They arrive with the model's first chunk, after the
/// call's `Start`, so the role frame waits for the first content, the end of
/// the stream or an error. Without a report the chunks keep the generated id
/// and `model`.
#[must_use]
pub fn to_chat_completion_stream_with_identity(
    stream: Pin<Box<dyn Stream<Item = Result<TextStreamPart, AiMuxError>> + Send>>,
    model: &str,
    options: OpenAiStreamOptions,
    identity: ResponseIdentity,
) -> ChatCompletionStream {
    chat_completion_stream(stream, model, options, Some(identity))
}

fn chat_completion_stream(
    stream: Pin<Box<dyn Stream<Item = Result<TextStreamPart, AiMuxError>> + Send>>,
    model: &str,
    options: OpenAiStreamOptions,
    identity: Option<ResponseIdentity>,
) -> ChatCompletionStream {
    let model = model.to_string();
    let include_usage = options.include_usage;
    let include_reasoning = options.include_reasoning;

    let chunk_stream = async_stream::stream! {
        let mut state = StreamState::new(model.clone(), identity);

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
                    if let Some(chunk) = state.pending_start() {
                        yield Ok(chunk);
                    }
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
            if matches!(part, TextStreamPart::Finish { .. }) {
                return;
            }
        }

        // Stream ended without Finish — emit a final chunk.
        if let Some(chunk) = state.pending_start() {
            yield Ok(chunk);
        }
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
    final_usage: Option<LanguageModelUsage>,
    final_finish_reason: Option<(FinishReasonUnified, Option<String>)>,
    finish_emitted: bool,
    /// The provider's response id and model, read when the role frame is sent.
    identity: Option<ResponseIdentity>,
    /// `Start` was seen and its role frame waits for the identity.
    start_pending: bool,
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
    fn new(model: String, identity: Option<ResponseIdentity>) -> Self {
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
            identity,
            start_pending: false,
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
        let reported = self.identity.as_ref().and_then(ResponseIdentity::reported);
        if let Some(reported) = reported {
            if let Some(id) = reported.id {
                self.id = id;
            }
            if let Some(model) = reported.model_id {
                self.model = model;
            }
        }
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

    /// The role frame of a `Start` still waiting for content, sent before an
    /// error or the final chunk.
    fn pending_start(&mut self) -> Option<ChatCompletionChunk> {
        if self.start_pending {
            self.ensure_started()
        } else {
            None
        }
    }

    /// Process a single TextStreamPart, returning zero or more output chunks.
    fn process_part(
        &mut self,
        part: &TextStreamPart,
        include_reasoning: bool,
        include_usage: bool,
    ) -> Vec<ChatCompletionChunk> {
        let mut chunks = Vec::new();

        match part {
            TextStreamPart::Start {} if self.identity.is_some() => self.start_pending = true,
            TextStreamPart::Start {} => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
            }

            TextStreamPart::FinishStep { response, .. } => {
                // Response metadata arrives at step end; emitted chunks keep their identity.
                if !self.started {
                    self.id.clone_from(&response.id);
                    self.model.clone_from(&response.model_id);
                }
            }

            TextStreamPart::TextStart { .. } => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
            }
            TextStreamPart::TextDelta { text: delta, .. } => {
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
            TextStreamPart::TextEnd { .. } => {}

            TextStreamPart::ReasoningStart { .. } => {}
            TextStreamPart::ReasoningDelta { text: delta, .. } => {
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
            TextStreamPart::ReasoningEnd { .. } => {}

            TextStreamPart::ToolInputStart { id, tool_name, .. } => {
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
            TextStreamPart::ToolInputDelta { id, delta, .. } => {
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
            TextStreamPart::ToolInputEnd { .. } => {}

            TextStreamPart::ToolCall(ToolCall {
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

            TextStreamPart::ToolResult(TypedToolResult {
                tool_name, output, ..
            }) => {
                // Degraded: provider-executed tool result as content.
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let text = format!("[tool result: {tool_name}] {output}");
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

            TextStreamPart::ToolError(TypedToolError {
                tool_name,
                error,
                provider_executed: Some(true),
                ..
            }) => {
                // Degraded: provider-executed tool error as content, as the
                // generated path renders it.
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let text = format!("[tool error: {tool_name}] {error}");
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

            TextStreamPart::File(GeneratedFileOutput { file, .. }) => {
                if let Some(c) = self.ensure_started() {
                    chunks.push(c);
                }
                let text = format!("data:{};base64,{}", file.media_type, file.base64());
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

            TextStreamPart::Source(Source::Url { url, .. }) => {
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

            TextStreamPart::Finish {
                finish_reason,
                raw_finish_reason,
                total_usage,
            } => {
                self.final_usage = Some(total_usage.clone());
                self.final_finish_reason = Some((*finish_reason, raw_finish_reason.clone()));
                if let Some(c) = self.pending_start() {
                    chunks.push(c);
                }
                // Emit the final chunk here.
                if let Some(c) = self.final_chunk_impl(include_usage) {
                    chunks.push(c);
                }
            }

            TextStreamPart::Error { error } => {
                if let Some(chunk) = self.pending_start() {
                    chunks.push(chunk);
                }
                if let Some(chunk) = self.error_chunk(error) {
                    chunks.push(chunk);
                }
            }

            TextStreamPart::Raw { .. }
            | TextStreamPart::StartStep { .. }
            | TextStreamPart::Abort { .. }
            | TextStreamPart::Source(Source::Document { .. })
            | TextStreamPart::Custom { .. }
            | TextStreamPart::ReasoningFile(_)
            | TextStreamPart::ToolError(_)
            | TextStreamPart::ToolOutputDenied(_)
            | TextStreamPart::ToolApprovalRequest(_)
            | TextStreamPart::ToolApprovalResponse(_) => { /* no OpenAI chat equivalent */ }
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
            .map(|(unified, raw)| finish_reason_to_openai(*unified, raw.as_deref()))
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

/// A model that records the `ResponseMetadata` of its streams into a
/// [`ResponseIdentity`]; see [`ResponseIdentity::observe`].
pub struct ResponseIdentityTap<'a> {
    model: &'a dyn LanguageModel,
    identity: ResponseIdentity,
}

#[async_trait::async_trait]
impl LanguageModel for ResponseIdentityTap<'_> {
    fn specification_version(&self) -> &'static str {
        self.model.specification_version()
    }

    fn provider(&self) -> &str {
        self.model.provider()
    }

    fn model_id(&self) -> &str {
        self.model.model_id()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        self.model.do_generate(options).await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        use futures::StreamExt;
        let mut result = self.model.do_stream(options).await?;
        let identity = self.identity.clone();
        result.stream = Box::pin(result.stream.inspect(move |part| {
            if let Ok(StreamPart::ResponseMetadata(metadata)) = part
                && let Ok(mut identity) = identity.0.lock()
            {
                *identity = Some(metadata.clone());
            }
        }));
        Ok(result)
    }

    async fn supported_urls(&self) -> SupportedUrls {
        self.model.supported_urls().await
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

/// A provider's `logprobs` metadata in the Chat Completions wire shape:
/// `{ "content": [...], "refusal": null }`.
///
/// The OpenAI chat model stores the token list `choice.logprobs.content`
/// (@ai-sdk/openai 4.0.83 src/chat/openai-chat-language-model.ts:459-461), and
/// the Responses model one token list per text part
/// (src/responses/openai-responses-language-model.ts:1062-1080), which
/// becomes one list in output order. A provider that stores the whole
/// logprobs object (DeepSeek's `{ content, reasoning_content }`,
/// @ai-sdk/deepseek 3.0.56 src/chat/deepseek-chat-language-model.ts:497) is
/// passed on as it is. An entry without `bytes` gets `null`, which the wire
/// format allows for a token with no byte representation.
fn chat_logprobs(logprobs: &Value) -> Option<Value> {
    fn token(entry: &Value) -> Value {
        let mut entry = entry.clone();
        if let Some(object) = entry.as_object_mut() {
            object.entry("bytes").or_insert(Value::Null);
            let top = object
                .entry("top_logprobs")
                .or_insert_with(|| Value::Array(Vec::new()));
            for alternative in top.as_array_mut().into_iter().flatten() {
                if let Some(alternative) = alternative.as_object_mut() {
                    alternative.entry("bytes").or_insert(Value::Null);
                }
            }
        }
        entry
    }
    match logprobs {
        Value::Object(_) => Some(logprobs.clone()),
        Value::Array(items) => {
            let content: Vec<Value> = if items.iter().all(Value::is_array) {
                items
                    .iter()
                    .filter_map(Value::as_array)
                    .flatten()
                    .map(token)
                    .collect()
            } else {
                items.iter().map(token).collect()
            };
            Some(serde_json::json!({ "content": content, "refusal": null }))
        }
        _ => None,
    }
}

/// Map a finish reason to an OpenAI `finish_reason` string.
fn finish_reason_to_openai(unified: FinishReasonUnified, raw: Option<&str>) -> Option<String> {
    match unified {
        FinishReasonUnified::Stop => Some("stop".to_string()),
        FinishReasonUnified::Length => Some("length".to_string()),
        FinishReasonUnified::ContentFilter => Some("content_filter".to_string()),
        FinishReasonUnified::ToolCalls => Some("tool_calls".to_string()),
        FinishReasonUnified::Error => Some("stop".to_string()),
        FinishReasonUnified::Other => Some(raw.unwrap_or("stop").to_string()),
    }
}

/// Convert a [`LanguageModelUsage`] to OpenAI [`ChatCompletionUsage`].
///
/// This is the inverse of `convert_usage` in
/// `aimux-providers/src/openai/model.rs`.
fn usage_to_openai(usage: &LanguageModelUsage) -> ChatCompletionUsage {
    let prompt_tokens = usage.input_tokens.unwrap_or(0);
    let completion_tokens = usage.output_tokens.unwrap_or(0);

    let cached_tokens = usage.input_token_details.cache_read_tokens.unwrap_or(0);
    let cache_write = usage.input_token_details.cache_write_tokens;
    let reasoning_tokens = usage.output_token_details.reasoning_tokens;

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
/// Render a Core-parsed `TextStreamPart::ToolCall`'s arguments as OpenAI-compatible
/// wire text: the provider's raw argument text verbatim for an invalid call,
/// compact JSON of the parsed value otherwise.
///
/// Errors without the original argument text (older errors, or the error
/// of a rejected repair) fall back to serializing the parsed input as JSON.
pub(crate) fn parsed_tool_call_arguments(
    input: &Value,
    invalid: Option<bool>,
    error: Option<&AiMuxError>,
) -> String {
    if invalid == Some(true)
        && let Some(error) = error
        && let Some(raw) = original_tool_call_text(input, error)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::GenerateResult;
    use crate::result::ReasoningOutput;
    use crate::shared::ResponseInfo;
    use crate::tool::RawToolCall;
    use crate::types::{
        FinishReason, FinishReasonUnified, InputTokenUsage, OutputTokenUsage, Usage,
    };
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
            response: Some(ResponseInfo {
                id: Some("chatcmpl-test123".to_string()),
                timestamp: None,
                model_id: Some("gpt-4o".to_string()),
                headers: None,
                body: None,
            }),
            request: None,
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
            GenerateContent::ToolCall(RawToolCall {
                tool_call_id: "call_abc".to_string(),
                tool_name: "get_weather".to_string(),
                input: r#"{"city":"Tokyo"}"#.to_string(),
                provider_executed: None,
                dynamic: None,
                provider_metadata: None,
            }),
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
    fn test_raw_tool_call_arguments_are_not_double_encoded() {
        let result = make_result(vec![GenerateContent::ToolCall(RawToolCall {
            tool_call_id: "call_raw".to_string(),
            tool_name: "get_weather".to_string(),
            input: r#"{"city":"Tokyo"}"#.to_string(),
            provider_executed: None,
            dynamic: None,
            provider_metadata: None,
        })]);

        let completion = to_chat_completion(&result, "gpt-4o");
        assert_eq!(
            completion.choices[0].message.tool_calls.as_ref().unwrap()[0]
                .function
                .arguments,
            r#"{"city":"Tokyo"}"#
        );
    }

    #[test]
    fn blank_invalid_tool_arguments_render_as_an_empty_object() {
        // Both OpenAI renderers: blank raw text has no wire representation
        // other than `{}`.
        let error = AiMuxError::InvalidToolInput {
            tool_name: "get_weather".to_string(),
            tool_input: String::new(),
            cause: "Type validation failed: missing 'city'".to_string(),
        };
        let arguments = parsed_tool_call_arguments(&json!({}), Some(true), Some(&error));
        assert_eq!(arguments, "{}");

        let result = make_result(vec![GenerateContent::ToolCall(RawToolCall {
            tool_call_id: "call_blank".to_string(),
            tool_name: "get_weather".to_string(),
            input: "  ".to_string(),
            provider_executed: None,
            dynamic: None,
            provider_metadata: None,
        })]);
        assert_eq!(
            to_chat_completion(&result, "gpt-4o").choices[0]
                .message
                .tool_calls
                .as_ref()
                .unwrap()[0]
                .function
                .arguments,
            "{}"
        );
    }

    #[test]
    fn parsed_tool_call_arguments_null_is_not_rewritten_to_empty_object() {
        // A valid call whose parsed input happens to be `null` (no schema
        // constraint rejected it) must round-trip as `null`, not `{}`.
        let arguments = parsed_tool_call_arguments(&Value::Null, None, None);
        assert_eq!(arguments, "null");
    }

    /// Regression coverage for the P1 finding on PR #165: `input: Value`
    /// alone cannot distinguish a validly parsed JSON string from malformed
    /// text wrapped as a fallback string, so an invalid call's arguments are
    /// recovered from its typed error instead. Driven end to end so the
    /// error/`input` pairing `parse_tool_call` actually produces is pinned
    /// too, not just the renderer.
    #[tokio::test]
    async fn invalid_tool_calls_stream_their_raw_arguments_verbatim() {
        let failing_repair = crate::parse_tool_call::ToolCallRepair::new(|_context| async {
            Err(AiMuxError::Other("repair model failed".to_string()))
        });
        let cases: [(&str, &str, Option<&crate::parse_tool_call::ToolCallRepair>); 3] = [
            // Valid JSON string rejected by the schema: the quotes must
            // survive, or the arguments field stops being JSON at all.
            ("get_weather", r#""hello""#, None),
            // Unknown tool *and* malformed text: `NoSuchTool` carries no raw
            // text of its own, so it comes off the unparsable `input`.
            ("unknown_tool", r#"{"a":"#, None),
            // A failed repair callback wraps the pre-repair error.
            ("get_weather", r#"{"a":"#, Some(&failing_repair)),
        ];
        let tools = [crate::tool::Tool::Function(crate::tool::FunctionTool::new(
            "get_weather",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ))];

        for (tool_name, raw_input, repair) in cases {
            let parsed = crate::parse_tool_call::parse_tool_call(
                RawToolCall {
                    tool_call_id: "call_1".to_string(),
                    tool_name: tool_name.to_string(),
                    input: raw_input.to_string(),
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                },
                Some(&tools),
                repair,
                &[],
                None,
            )
            .await;
            assert_eq!(parsed.invalid, Some(true), "{raw_input}");

            let parts: Vec<Result<TextStreamPart, AiMuxError>> =
                vec![Ok(TextStreamPart::ToolCall(ToolCall {
                    tool_call_id: parsed.tool_call_id,
                    tool_name: parsed.tool_name,
                    input: parsed.input,
                    provider_executed: parsed.provider_executed,
                    dynamic: parsed.dynamic,
                    invalid: parsed.invalid,
                    error: parsed.error,
                    provider_metadata: parsed.provider_metadata,
                }))];
            let chunks = collect_stream(to_chat_completion_stream(
                Box::pin(futures::stream::iter(parts)),
                "gpt-4o",
                OpenAiStreamOptions::default(),
            ))
            .await;

            let arguments = chunks
                .iter()
                .find_map(|c| {
                    c.choices
                        .first()?
                        .delta
                        .tool_calls
                        .as_ref()?
                        .first()?
                        .function
                        .arguments
                        .clone()
                })
                .expect("tool call arguments chunk not found");
            assert_eq!(arguments, raw_input, "tool {tool_name}");
        }
    }

    #[test]
    fn test_tool_call_null_content() {
        // Tool call with no text → content should be null.
        let result = make_result(vec![GenerateContent::ToolCall(RawToolCall {
            tool_call_id: "call_abc".to_string(),
            tool_name: "get_weather".to_string(),
            input: r#"{"city":"Tokyo"}"#.to_string(),
            provider_executed: None,
            dynamic: None,
            provider_metadata: None,
        })]);
        let completion = to_chat_completion(&result, "gpt-4o");

        assert_eq!(completion.choices[0].message.content, None);
        assert!(completion.choices[0].message.tool_calls.is_some());
    }

    #[test]
    fn test_reasoning_non_streaming() {
        let result = make_result(vec![
            GenerateContent::Reasoning(ReasoningOutput {
                text: "Thinking...".to_string(),
                provider_metadata: None,
            }),
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
            input_tokens: InputTokenUsage {
                total: Some(100),
                cache_read: Some(30),
                cache_write: Some(10),
                ..Default::default()
            },
            output_tokens: OutputTokenUsage {
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
            assert_eq!(
                finish_reason_to_openai(unified, None).as_deref(),
                Some(expected)
            );
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
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Ok(TextStreamPart::TextStart {
                id: "0".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::TextDelta {
                id: "0".to_string(),
                text: "Hello ".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::TextDelta {
                id: "0".to_string(),
                text: "world".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::TextEnd {
                id: "0".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::Finish {
                finish_reason: FinishReasonUnified::Stop,
                raw_finish_reason: None,
                total_usage: LanguageModelUsage::default(),
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
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Ok(TextStreamPart::ToolInputStart {
                id: "call_1".to_string(),
                tool_name: "get_weather".to_string(),
                provider_executed: None,
                dynamic: None,
                title: None,
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ToolInputDelta {
                id: "call_1".to_string(),
                delta: "{\"city\":".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ToolInputDelta {
                id: "call_1".to_string(),
                delta: "\"Tokyo\"}".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ToolInputEnd {
                id: "call_1".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ToolCall(ToolCall {
                tool_call_id: "call_1".to_string(),
                tool_name: "get_weather".to_string(),
                input: json!({ "city": "Tokyo" }),
                provider_executed: None,
                dynamic: None,
                invalid: None,
                error: None,
                provider_metadata: None,
            })),
            Ok(TextStreamPart::Finish {
                finish_reason: FinishReasonUnified::ToolCalls,
                raw_finish_reason: None,
                total_usage: LanguageModelUsage::default(),
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
    async fn test_stream_complete_call_backfills_arguments_absent_from_deltas() {
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::ToolInputStart {
                id: "call_1".to_string(),
                tool_name: "fetch".to_string(),
                provider_executed: Some(true),
                dynamic: None,
                title: None,
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ToolInputEnd {
                id: "call_1".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ToolCall(ToolCall {
                tool_call_id: "call_1".to_string(),
                tool_name: "fetch".to_string(),
                input: json!({ "url": "https://example.com" }),
                provider_executed: Some(true),
                dynamic: None,
                invalid: None,
                error: None,
                provider_metadata: None,
            })),
            Ok(TextStreamPart::Finish {
                finish_reason: FinishReasonUnified::Stop,
                raw_finish_reason: None,
                total_usage: LanguageModelUsage::default(),
            }),
        ];

        let result = to_chat_completion_stream(
            Box::pin(futures::stream::iter(parts)),
            "claude",
            OpenAiStreamOptions::default(),
        );
        let chunks = collect_stream(result).await;
        let arguments = chunks
            .iter()
            .filter_map(|chunk| {
                chunk
                    .choices
                    .first()?
                    .delta
                    .tool_calls
                    .as_ref()?
                    .first()?
                    .function
                    .arguments
                    .as_deref()
            })
            .collect::<String>();

        assert_eq!(arguments, r#"{"url":"https://example.com"}"#);
    }

    #[tokio::test]
    async fn test_stream_multiple_tool_calls_index() {
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Ok(TextStreamPart::ToolInputStart {
                id: "call_a".to_string(),
                tool_name: "tool_a".to_string(),
                provider_executed: None,
                dynamic: None,
                title: None,
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ToolInputStart {
                id: "call_b".to_string(),
                tool_name: "tool_b".to_string(),
                provider_executed: None,
                dynamic: None,
                title: None,
                provider_metadata: None,
            }),
            Ok(TextStreamPart::Finish {
                finish_reason: FinishReasonUnified::ToolCalls,
                raw_finish_reason: None,
                total_usage: LanguageModelUsage::default(),
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
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Ok(TextStreamPart::ReasoningStart {
                id: "reasoning-0".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ReasoningDelta {
                id: "reasoning-0".to_string(),
                text: "Hmm...".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::ReasoningEnd {
                id: "reasoning-0".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::TextDelta {
                id: "0".to_string(),
                text: "Answer".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::Finish {
                finish_reason: FinishReasonUnified::Stop,
                raw_finish_reason: None,
                total_usage: LanguageModelUsage::default(),
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
            input_tokens: InputTokenUsage {
                total: Some(42),
                ..Default::default()
            },
            output_tokens: OutputTokenUsage {
                total: Some(10),
                ..Default::default()
            },
            raw: None,
        };

        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Ok(TextStreamPart::TextDelta {
                id: "0".to_string(),
                text: "Hi".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::Finish {
                finish_reason: FinishReasonUnified::Stop,
                raw_finish_reason: None,
                total_usage: as_language_model_usage(&usage),
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
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Ok(TextStreamPart::Finish {
                finish_reason: FinishReasonUnified::Stop,
                raw_finish_reason: None,
                total_usage: as_language_model_usage(&Usage {
                    input_tokens: InputTokenUsage {
                        total: Some(42),
                        ..Default::default()
                    },
                    output_tokens: OutputTokenUsage::default(),
                    raw: None,
                }),
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
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Ok(TextStreamPart::TextDelta {
                id: "0".to_string(),
                text: "partial".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::Error {
                error: AiMuxError::ApiCall(Box::new(crate::error::ApiCallError::new(
                    "something went wrong",
                    "https://example.test/v1",
                    serde_json::json!({}),
                ))),
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
    async fn recoverable_frame_error_does_not_truncate_chat_completion_stream() {
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Err(AiMuxError::JsonParse("malformed SSE data".into())),
            Ok(TextStreamPart::TextDelta {
                id: "0".to_string(),
                text: "after-error".to_string(),
                provider_metadata: None,
            }),
            Ok(TextStreamPart::Finish {
                finish_reason: FinishReasonUnified::Stop,
                raw_finish_reason: Some("stop".into()),
                total_usage: LanguageModelUsage::default(),
            }),
        ];

        let result = to_chat_completion_stream(
            Box::pin(futures::stream::iter(parts)),
            "gpt-4o",
            OpenAiStreamOptions::default(),
        );
        let outcomes: Vec<_> = result.stream.collect().await;
        let error_index = outcomes
            .iter()
            .position(|outcome| matches!(outcome, Err(AiMuxError::JsonParse(_))))
            .expect("parse error should be preserved as an error item");

        assert!(outcomes[error_index + 1..].iter().any(|outcome| matches!(
            outcome,
            Ok(chunk)
                if chunk.choices.first().and_then(|choice| choice.delta.content.as_deref())
                    == Some("after-error")
        )));
        assert!(outcomes.iter().any(|outcome| matches!(
            outcome,
            Ok(chunk)
                if chunk.choices.first().and_then(|choice| choice.finish_reason.as_deref())
                    == Some("stop")
        )));
    }

    #[tokio::test]
    async fn test_stream_completes_without_finish() {
        // Stream ends without Finish — should still emit a final chunk.
        let parts: Vec<Result<TextStreamPart, AiMuxError>> = vec![
            Ok(TextStreamPart::Start {}),
            Ok(TextStreamPart::TextDelta {
                id: "0".to_string(),
                text: "Hello".to_string(),
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
