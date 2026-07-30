//! Content parts shared between user-facing and provider-facing messages.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// A part of a multi-part message.
///
/// Shared between `ModelMessage` (user-facing) and `LanguageModelPrompt` (provider-facing).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ContentPart {
    /// A text segment.
    Text {
        text: String,
        /// Provider-specific options for this part (e.g. `openai.promptCacheBreakpoint`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },

    /// An image (raw bytes + MIME type).
    Image {
        image: Vec<u8>,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },

    /// A file (raw bytes + MIME type).
    File {
        data: Vec<u8>,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },

    /// A file whose inline data is an already-base64-encoded string.
    ///
    /// Mirrors the Vercel AI SDK `file` part with `data: { type: 'data', data:
    /// '<base64>' }`: the `data` field holds the raw base64 string verbatim
    /// (it is NOT decoded into bytes), so it round-trips through providers that
    /// emit a `base64` source unchanged.
    FileBase64 {
        data: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },

    /// A file referenced by URL.
    ///
    /// Mirrors the Vercel AI SDK `file` part with `data: { type: 'url', url }`.
    FileUrl {
        url: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },

    /// A file referenced by a provider-specific reference (e.g. an OpenAI file ID).
    ///
    /// Mirrors the Vercel AI SDK `file` part with `data: { type: 'reference',
    /// reference: { openai: 'file-xxx' } }`. The `reference` field is a JSON
    /// object keyed by provider name.
    FileReference {
        media_type: String,
        reference: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },

    /// A reasoning / thinking segment produced by the model.
    ///
    /// Mirrors the Vercel AI SDK `reasoning` part. `signature` is the Anthropic
    /// thinking-block signature (from `providerOptions.anthropic.signature`);
    /// when absent the part is treated as having unsupported metadata (unless
    /// `providerOptions.anthropic.redactedData` is present, in which case a
    /// `redacted_thinking` block is emitted).
    Reasoning {
        text: String,
        signature: Option<String>,
        /// Provider-specific options for this part (e.g.
        /// `anthropic.signature`, `anthropic.redactedData`,
        /// `anthropic.cacheControl`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },

    /// A tool call requested by the model.
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        /// Arguments as a JSON value (usually an object).
        input: Value,
        /// Provider-specific options for this part (e.g.
        /// `anthropic.cacheControl`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },

    /// The result of executing a tool call.
    ToolResult {
        tool_call_id: String,
        /// The tool's output (usually a JSON value or plain text).
        result: Value,
        /// The name of the tool that produced this result (optional on the
        /// user-input side; providers that need it can look it up from the
        /// preceding tool call).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
        /// Whether the result is an error or error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        /// Whether the result is preliminary (replaces prior, e.g. image previews).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preliminary: Option<bool>,
        /// Whether the tool is dynamic (defined at runtime, e.g. MCP tools).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
        /// Provider-specific options for this part (e.g.
        /// `anthropic.cacheControl`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
}

impl ContentPart {
    /// Convenience constructor for a text part (no provider options).
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text {
            text: text.into(),
            provider_options: None,
        }
    }

    /// Convenience constructor for a tool-call part (no provider options).
    pub fn tool_call(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        input: Value,
    ) -> Self {
        ContentPart::ToolCall {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            input,
            provider_options: None,
        }
    }

    /// Convenience constructor for a tool-result part (no provider options).
    pub fn tool_result(tool_call_id: impl Into<String>, result: Value) -> Self {
        ContentPart::ToolResult {
            tool_call_id: tool_call_id.into(),
            result,
            tool_name: None,
            is_error: None,
            preliminary: None,
            dynamic: None,
            provider_options: None,
        }
    }

    /// Convenience constructor for a reasoning/thinking part (no provider
    /// options, no signature).
    pub fn reasoning(text: impl Into<String>) -> Self {
        ContentPart::Reasoning {
            text: text.into(),
            signature: None,
            provider_options: None,
        }
    }

    /// Convenience constructor for an image part (no provider options).
    pub fn image(image: Vec<u8>, media_type: impl Into<String>) -> Self {
        ContentPart::Image {
            image,
            media_type: media_type.into(),
            provider_options: None,
        }
    }

    /// Convenience constructor for a file part (no provider options, no filename).
    pub fn file(data: Vec<u8>, media_type: impl Into<String>) -> Self {
        ContentPart::File {
            data,
            media_type: media_type.into(),
            filename: None,
            provider_options: None,
        }
    }

    /// Convenience constructor for a base64-encoded file part (no provider
    /// options, no filename).
    pub fn file_base64(data: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentPart::FileBase64 {
            data: data.into(),
            media_type: media_type.into(),
            filename: None,
            provider_options: None,
        }
    }

    /// Convenience constructor for a URL-referenced file part (no provider options).
    pub fn file_url(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentPart::FileUrl {
            url: url.into(),
            media_type: media_type.into(),
            provider_options: None,
        }
    }

    /// Convenience constructor for a provider-referenced file part (no provider
    /// options, no filename).
    pub fn file_reference(media_type: impl Into<String>, reference: Value) -> Self {
        ContentPart::FileReference {
            media_type: media_type.into(),
            reference,
            filename: None,
            provider_options: None,
        }
    }
}
