//! User-facing message content: the parts a [`ModelMessage`] holds.
//!
//! The AI SDK's `@ai-sdk/provider-utils` content parts
//! (`types/content-part.ts`, `tool-approval-request.ts`,
//! `tool-approval-response.ts`), grouped into one union per message role as
//! upstream's `UserContent`, `AssistantContent` and `ToolContent` group them
//! (`user-model-message.ts:22`, `assistant-model-message.ts:32`,
//! `tool-model-message.ts:23`). `convert_to_language_model_prompt` turns them
//! into the provider-facing parts of `language_model_message`.
//!
//! The JSON is this layer's wire: `type` tags and field names are snake_case,
//! and a file is one of the five file variants (`image`, `file`,
//! `file_base64`, `file_url`, `file_reference`) rather than upstream's single
//! `file` part with a `FileData` payload.
//!
//! [`ModelMessage`]: crate::message::ModelMessage

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::shared::{GeneratedFileData, SharedProviderOptions, SharedProviderReference};
use crate::types::NamespacedKind;

/// A part of a user message: upstream's `TextPart | ImagePart | FilePart`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum UserContentPart {
    /// A text segment.
    Text {
        text: String,
        /// Provider-specific options for this part (e.g. `openai.promptCacheBreakpoint`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// An image (raw bytes + MIME type).
    ///
    /// Deprecated upstream (`provider-utils` `types/content-part.ts:26-29`):
    /// use a `file` part with an `image` media type. The prompt conversion
    /// turns it into one and logs a deprecation warning.
    Image {
        image: Vec<u8>,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file (raw bytes + MIME type).
    File {
        data: Vec<u8>,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file whose inline data is an already-base64-encoded string.
    ///
    /// The AI SDK `file` part with `data: { type: 'data', data: '<base64>' }`:
    /// `data` holds the base64 string verbatim (it is not decoded), so it
    /// round-trips through providers that emit a `base64` source unchanged.
    FileBase64 {
        data: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file referenced by URL (the AI SDK `file` part with
    /// `data: { type: 'url', url }`).
    FileUrl {
        url: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file referenced by a provider-specific id (the AI SDK `file` part
    /// with `data: { type: 'reference', reference: { openai: 'file-xxx' } }`).
    /// `reference` is a JSON object keyed by provider name.
    FileReference {
        media_type: String,
        reference: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
}

/// A part of an assistant message: upstream's `TextPart | CustomPart |
/// FilePart | ReasoningPart | ReasoningFilePart | ToolCallPart |
/// ToolResultPart | ToolApprovalRequest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum AssistantContentPart {
    /// A text segment.
    Text {
        text: String,
        /// Provider-specific options for this part.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// Provider-specific custom content.
    Custom {
        kind: NamespacedKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file (raw bytes + MIME type).
    File {
        data: Vec<u8>,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file whose inline data is an already-base64-encoded string.
    FileBase64 {
        data: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file referenced by URL.
    FileUrl {
        url: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file referenced by a provider-specific id.
    FileReference {
        media_type: String,
        reference: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A reasoning / thinking segment produced by the model.
    ///
    /// The AI SDK `ReasoningPart` has only `text` and `providerOptions`: a
    /// provider's reasoning signature travels in its namespace (e.g.
    /// `anthropic.signature`, `bedrock.signature`), as the provider wrote it
    /// into the response's provider metadata.
    Reasoning {
        text: String,
        /// Provider-specific options for this part (e.g.
        /// `anthropic.signature`, `anthropic.redactedData`,
        /// `anthropic.cacheControl`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// A file generated during reasoning.
    ReasoningFile {
        data: GeneratedFileData,
        media_type: String,
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
        /// the client. Providers need it to replay server tool calls on a
        /// later turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        /// Provider-specific options for this part (e.g.
        /// `anthropic.cacheControl`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },

    /// The result of a provider-executed tool call.
    ToolResult(ToolResultPart),

    /// A request to approve a tool call (upstream `ToolApprovalRequest`).
    /// Never sent to a provider: the conversion drops it and uses it to match
    /// the user's [`ToolContentPart::ToolApprovalResponse`] to its tool call.
    ToolApprovalRequest {
        approval_id: String,
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_automatic: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_schema_input: Option<Value>,
    },
}

/// A part of a tool message: upstream's `ToolResultPart | ToolApprovalResponse`.
// Prompt content is built once per call and converted, not held in bulk, so
// the size gap between the two parts is not worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum ToolContentPart {
    /// The result of a tool call.
    ToolResult(ToolResultPart),

    /// The user's answer to an [`AssistantContentPart::ToolApprovalRequest`]
    /// (upstream `ToolApprovalResponse`). The conversion passes it to the
    /// provider only when `provider_executed` is set; for any other tool it
    /// only marks the call as answered.
    ToolApprovalResponse {
        approval_id: String,
        approved: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
    },
}

impl UserContentPart {
    /// A text part (no provider options).
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            provider_options: None,
        }
    }
}

impl AssistantContentPart {
    /// A text part (no provider options).
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            provider_options: None,
        }
    }

    /// A tool-call part (no provider options).
    pub fn tool_call(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        input: Value,
    ) -> Self {
        Self::ToolCall {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            input,
            provider_executed: None,
            provider_options: None,
        }
    }
}

impl ToolContentPart {
    /// A tool-result part (no provider options).
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: ToolResultOutput,
    ) -> Self {
        Self::ToolResult(ToolResultPart {
            tool_call_id: tool_call_id.into(),
            tool_name: Some(tool_name.into()),
            output,
            provider_options: None,
        })
    }
}

/// The result of a tool call (upstream `ToolResultPart`).
///
/// JSON: besides the upstream `output` object, the part keeps accepting the
/// earlier `result` form, an arbitrary JSON value read by
/// [`ToolResultOutput::from_result`] (with `is_error: true` the value is an
/// error). `output` holding a value that is not a [`ToolResultOutput`] object
/// is read the same way. A `text`, `json`, `error-text` or `error-json`
/// output without its own `provider_options` is written in the `result`
/// form; any other output is written as `output`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(try_from = "ToolResultPartJson", into = "ToolResultPartJson")]
#[ts(as = "ToolResultPartJson")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ToolResultPart {
    pub tool_call_id: String,
    /// The tool name. Required upstream; when absent here, the conversion
    /// takes it from the preceding tool call with the same id.
    pub tool_name: Option<String>,
    pub output: ToolResultOutput,
    /// Provider-specific options for this part (e.g. `anthropic.cacheControl`).
    pub provider_options: Option<SharedProviderOptions>,
}

/// The JSON of a [`ToolResultPart`]: the upstream `output` or the earlier
/// `result` form.
#[derive(Serialize, Deserialize, TS)]
struct ToolResultPartJson {
    tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    tool_name: Option<String>,
    /// A `ToolResultOutput`, or any other JSON value, read as `result` is.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[ts(optional, as = "Option<OutputJson>", inline)]
    output: Option<Value>,
    /// Any JSON value: a string is a `text` output, anything else `json`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[ts(optional, type = "JsonValue")]
    result: Option<Value>,
    /// With `result`: the value is an error, an `error-text` output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    provider_options: Option<SharedProviderOptions>,
}

/// The TypeScript type of [`ToolResultPartJson::output`].
#[derive(TS)]
#[allow(dead_code)]
enum OutputJson {
    #[ts(untagged)]
    Output(ToolResultOutput),
    #[ts(untagged)]
    Value(Value),
}

/// Keep an explicit `null` apart from a missing field.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// The `type` tags of [`ToolResultOutput`].
const OUTPUT_TYPES: [&str; 6] = [
    "text",
    "json",
    "execution-denied",
    "error-text",
    "error-json",
    "content",
];

impl TryFrom<ToolResultPartJson> for ToolResultPart {
    type Error = String;

    fn try_from(json: ToolResultPartJson) -> Result<Self, Self::Error> {
        let is_typed = |value: &Value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|tag| OUTPUT_TYPES.contains(&tag))
        };
        let output = match (json.result, json.output) {
            (Some(_), Some(_)) => {
                return Err("a tool result has `output` or `result`, not both".into());
            }
            (None, Some(output)) if is_typed(&output) => {
                if json.is_error.is_some() {
                    return Err(
                        "`is_error` applies to a `result` value, not to a typed `output`".into(),
                    );
                }
                serde_json::from_value(output)
                    .map_err(|error| format!("invalid tool result output: {error}"))?
            }
            (Some(value), None) | (None, Some(value)) => {
                ToolResultOutput::from_result(value, json.is_error == Some(true))
            }
            (None, None) => return Err("missing field `output`".into()),
        };
        Ok(Self {
            tool_call_id: json.tool_call_id,
            tool_name: json.tool_name,
            output,
            provider_options: json.provider_options,
        })
    }
}

impl From<ToolResultPart> for ToolResultPartJson {
    fn from(part: ToolResultPart) -> Self {
        let (result, is_error, output) = match part.output {
            ToolResultOutput::Text {
                value,
                provider_options: None,
            } => (Some(Value::String(value)), None, None),
            ToolResultOutput::Json {
                value,
                provider_options: None,
            } => (Some(value), None, None),
            ToolResultOutput::ErrorText {
                value,
                provider_options: None,
            } => (Some(Value::String(value)), Some(true), None),
            ToolResultOutput::ErrorJson {
                value,
                provider_options: None,
            } => (Some(value), Some(true), None),
            output => (
                None,
                None,
                Some(serde_json::to_value(output).expect("a tool result output serializes")),
            ),
        };
        Self {
            tool_call_id: part.tool_call_id,
            tool_name: part.tool_name,
            output,
            result,
            is_error,
            provider_options: part.provider_options,
        }
    }
}

/// The output of a tool call (upstream `ToolResultOutput`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum ToolResultOutput {
    /// Text for the model.
    Text {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// JSON for the model.
    Json {
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// The user denied the tool call.
    ExecutionDenied {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// An error message.
    ErrorText {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// An error as JSON.
    ErrorJson {
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// Multi-part content: text, files, custom parts.
    Content { value: Vec<ToolResultContentPart> },
}

impl ToolResultOutput {
    /// The output for a raw tool result value, as the AI SDK's
    /// `createToolModelOutput` builds it for a tool without `toModelOutput`
    /// (`ai` `prompt/create-tool-model-output.ts:4`): a string is `text`,
    /// anything else `json`; an error is `error-text` holding
    /// `getErrorMessage(value)`, where `null` reads `unknown error`, a string
    /// is itself and anything else is its JSON.
    #[must_use]
    pub fn from_result(value: Value, is_error: bool) -> Self {
        match (value, is_error) {
            (Value::Null, true) => Self::ErrorText {
                value: "unknown error".into(),
                provider_options: None,
            },
            (Value::String(value), true) => Self::ErrorText {
                value,
                provider_options: None,
            },
            (value, true) => Self::ErrorText {
                value: value.to_string(),
                provider_options: None,
            },
            (Value::String(value), false) => Self::Text {
                value,
                provider_options: None,
            },
            (value, false) => Self::Json {
                value,
                provider_options: None,
            },
        }
    }
}

/// One item of a `content` [`ToolResultOutput`] (upstream
/// `content-part.ts:303-546`).
///
/// The `file-*` and `image-*` items are deprecated upstream in favour of
/// `file` (`content-part.ts:361-532`); they are accepted on input and the
/// conversion turns each into a `file` part with a deprecation warning, as
/// upstream's `mapToolResultOutput` does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum ToolResultContentPart {
    /// Text.
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// A file.
    File {
        data: crate::shared::FileData,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// Base64 file data. Deprecated: use `file` with `{ type: 'data' }` data.
    FileData {
        data: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// A file URL. Deprecated: use `file` with `{ type: 'url' }` data.
    FileUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// A provider file id. Deprecated: use `file` with `{ type: 'reference' }`
    /// data.
    FileId {
        file_id: ToolResultFileId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// A provider reference. Deprecated: use `file` with
    /// `{ type: 'reference' }` data.
    FileReference {
        provider_reference: SharedProviderReference,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// Base64 image data. Deprecated: use `file` with an `image` media type.
    ImageData {
        data: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// An image URL. Deprecated: use `file` with an `image` media type.
    ImageUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// A provider image file id. Deprecated: use `file` with an `image`
    /// media type.
    ImageFileId {
        file_id: ToolResultFileId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// A provider image reference. Deprecated: use `file` with an `image`
    /// media type.
    ImageFileReference {
        provider_reference: SharedProviderReference,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// Provider-specific content.
    Custom {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
}

/// The `fileId` of a deprecated `file-id` / `image-file-id` tool output item:
/// an id for the called provider, or ids keyed by provider
/// (`string | Record<string, string>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum ToolResultFileId {
    Id(String),
    ByProvider(SharedProviderReference),
}
