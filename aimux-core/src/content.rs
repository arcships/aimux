//! Content parts shared between user-facing and provider-facing messages.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::language_model_message::ToolResultOutput;
use crate::shared::{
    FileBytes, FileData, GeneratedFileData, SharedProviderOptions, SharedProviderReference,
};

/// A part of a multi-part message.
///
/// Shared between `ModelMessage` (user-facing) and `LanguageModelPrompt` (provider-facing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ContentPart {
    /// A text segment.
    Text {
        text: String,
        /// Provider-specific options for this part (e.g. `openai.promptCacheBreakpoint`).
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// Provider-specific custom content.
    Custom {
        kind: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file generated during reasoning (upstream `ReasoningFilePart`,
    /// `types/content-part.ts:146-180`).
    ReasoningFile {
        data: ReasoningFilePartData,
        media_type: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A request to approve a tool call on a later turn.
    ToolApprovalRequest {
        approval_id: String,
        tool_call_id: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_automatic: Option<bool>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[ts(optional)]
        #[serde(
            default,
            deserialize_with = "crate::wire::present",
            skip_serializing_if = "Option::is_none"
        )]
        input_schema_input: Option<Value>,
    },

    /// An image (raw bytes + MIME type). The JSON holds the bytes as a
    /// base64 string.
    Image {
        #[ts(type = "string")]
        #[serde(with = "crate::wire::base64_bytes")]
        image: Vec<u8>,
        media_type: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file (upstream `FilePart`, `provider-utils` `types/content-part.ts:62-102`):
    /// `data` is tagged `FileData` or a bare base64 string, URL string or
    /// provider reference, and a `data:` URL's media type wins over
    /// `media_type` when the prompt is converted.
    File {
        data: FilePartData,
        media_type: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
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
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        /// Provider-specific options for this part (e.g.
        /// `anthropic.signature`, `anthropic.redactedData`,
        /// `anthropic.cacheControl`).
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A tool call requested by the model.
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        /// Arguments as a JSON value (usually an object).
        input: Value,
        /// Whether the tool call is executed by the provider rather than by
        /// the client. This is part of the prompt contract because providers
        /// need it to replay server tool calls on a later turn.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        /// Provider-specific options for this part (e.g.
        /// `anthropic.cacheControl`).
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// The result of executing a tool call (upstream `ToolResultPart`).
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        output: ToolResultOutput,
        /// Provider-specific options for this part (e.g.
        /// `anthropic.cacheControl`).
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
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
            provider_executed: None,
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

    /// Convenience constructor for a file part with inline bytes, written as
    /// `data: { type: 'data', data: '<base64>' }` (no provider options, no
    /// filename).
    pub fn file(data: Vec<u8>, media_type: impl Into<String>) -> Self {
        Self::file_data(
            FileData::Data {
                data: FileBytes::Binary(data),
            },
            media_type,
        )
    }

    /// Convenience constructor for a file part whose inline data is an
    /// already-base64-encoded string, passed on verbatim (no provider options,
    /// no filename).
    pub fn file_base64(data: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self::file_data(
            FileData::Data {
                data: FileBytes::Base64(data.into()),
            },
            media_type,
        )
    }

    /// Convenience constructor for a file part referenced by URL,
    /// `data: { type: 'url', url }` (no provider options).
    pub fn file_url(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self::file_data(
            FileData::Url {
                url: url.into(),
                original_url: None,
            },
            media_type,
        )
    }

    /// Convenience constructor for a file part referenced by a provider
    /// reference, `data: { type: 'reference', reference: { [provider]: id } }`
    /// (no provider options, no filename).
    pub fn file_reference(
        media_type: impl Into<String>,
        reference: SharedProviderReference,
    ) -> Self {
        Self::file_data(FileData::Reference { reference }, media_type)
    }

    fn file_data(data: FileData, media_type: impl Into<String>) -> Self {
        ContentPart::File {
            data: FilePartData::Tagged(data),
            media_type: media_type.into(),
            filename: None,
            provider_options: None,
        }
    }
}

/// `FilePart.data` (`provider-utils` `types/content-part.ts:75`):
/// `FileData | DataContent | URL | ProviderReference`, in JSON a tagged
/// `FileData`, a string (base64 or a URL) or a bare `{ [provider]: id }`.
///
/// A string is read as the AI SDK reads it when it converts the prompt
/// (`ai` `prompt/file-part-data.ts:125-131`): a URL when it parses as one
/// (a `data:` URL becomes inline data), base64 otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum FilePartData {
    /// `{ type: 'data' | 'url' | 'reference' | 'text', ... }`.
    Tagged(FileData),
    /// A base64 string or a URL string.
    String(String),
    /// A bare provider reference.
    Reference(SharedProviderReference),
}

/// `ReasoningFilePart.data` (`types/content-part.ts:164`):
/// `FileDataData | FileDataUrl | DataContent | URL`, in JSON tagged data or a
/// URL, or a string (base64 or a URL).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum ReasoningFilePartData {
    /// `{ type: 'data' | 'url', ... }`.
    Tagged(GeneratedFileData),
    /// A base64 string or a URL string.
    String(String),
}
