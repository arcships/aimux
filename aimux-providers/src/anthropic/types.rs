//! Anthropic API response types.

#![allow(dead_code)]

use serde::Deserialize;
use serde_json::Value;

// ── Non-streaming response ──

#[derive(Debug, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(default)]
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: AnthropicUsage,
    /// Optional `context_management` object echoed by the API, carrying
    /// `applied_edits`. Left opaque here; the model layer maps it into
    /// `providerMetadata.anthropic.contextManagement`.
    #[serde(default)]
    pub context_management: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Anthropic extended-thinking block. Carries the reasoning text and an
    /// opaque `signature` (required to send the thinking block back in a
    /// multi-turn conversation).
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
    },

    // ── Provider-defined (server-side) tool content blocks ──
    //
    // Fields are decoded permissively (all `#[serde(default)]`, opaque `Value`
    // payloads) so a malformed block never fails the whole response; the model
    // layer decides how (or whether) to surface each kind.
    /// A tool call executed on Anthropic's servers (web_search, code_execution,
    /// web_fetch, advisor, tool_search, ...).
    #[serde(rename = "server_tool_use")]
    ServerToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: Value,
    },
    /// Result of a server-side web search. `content` is either an array of
    /// `web_search_result` objects or a `web_search_tool_result_error` object.
    #[serde(rename = "web_search_tool_result")]
    WebSearchToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
    },
    /// Result of a server-side web fetch.
    #[serde(rename = "web_fetch_tool_result")]
    WebFetchToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
    },
    /// Result of server-side code execution (20250522).
    #[serde(rename = "code_execution_tool_result")]
    CodeExecutionToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
    },
    /// Result of server-side code execution (20250825).
    #[serde(rename = "bash_code_execution_tool_result")]
    BashCodeExecutionToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
    },
    /// Result of server-side code execution (20250825, text-editor variant).
    #[serde(rename = "text_editor_code_execution_tool_result")]
    TextEditorCodeExecutionToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
    },
    /// Result of a server-side tool search.
    #[serde(rename = "tool_search_tool_result")]
    ToolSearchToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
    },
    /// Result of the advisor tool.
    #[serde(rename = "advisor_tool_result")]
    AdvisorToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
    },
    /// A tool call executed via an MCP server.
    #[serde(rename = "mcp_tool_use")]
    McpToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: Value,
        #[serde(default)]
        server_name: String,
    },
    /// Result of an MCP tool call.
    #[serde(rename = "mcp_tool_result")]
    McpToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: Option<bool>,
    },
    /// Redacted thinking block.
    #[serde(rename = "redacted_thinking")]
    RedactedThinking {
        #[serde(default)]
        data: Value,
    },
    /// Catch-all for any content-block type this SDK version does not yet model
    /// (e.g. `compaction`, `fallback`). Decoding never fails for an unknown
    /// `type` tag; the block is simply ignored by the model layer.
    #[serde(other)]
    Other,
}

/// `output_tokens_details` nested inside an Anthropic usage object, carrying
/// the count of thinking/reasoning tokens.
#[derive(Debug, Default, Deserialize)]
pub struct OutputTokenDetails {
    #[serde(default)]
    pub thinking_tokens: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AnthropicUsage {
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens_details: Option<OutputTokenDetails>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
}

// ── Streaming events ──

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartData },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: DeltaBlock },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaBody,
        #[serde(default)]
        usage: Option<AnthropicUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    /// Mid-stream error event: `{"type":"error","error":{"type":"...","message":"..."}}`.
    /// Anthropic emits these for overloaded errors and other in-stream failures
    /// (see the TS cases "first stream chunk is an overloaded error" and
    /// "forward overloaded error during streaming").
    #[serde(rename = "error")]
    Error { error: StreamErrorData },
    #[serde(other)]
    Other,
}

/// The `error` payload nested inside an Anthropic stream `error` event.
#[derive(Debug, Deserialize)]
pub struct StreamErrorData {
    #[serde(default, rename = "type")]
    pub error_type: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct MessageStartData {
    pub id: String,
    pub model: String,
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
pub struct DeltaBlock {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub partial_json: Option<String>,
    /// `thinking_delta` payload — incremental reasoning text.
    #[serde(default)]
    pub thinking: Option<String>,
    /// `signature_delta` payload — incremental signature text.
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageDeltaBody {
    #[serde(default)]
    pub stop_reason: Option<String>,
}
