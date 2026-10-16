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
//! The JSON is the AI SDK's: kebab-case `type` tags, camelCase fields, and
//! one `file` part whose `data` is upstream's `FilePart.data` ([`FilePartData`]).
//!
//! [`ModelMessage`]: crate::message::ModelMessage

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::shared::{
    FileBytes, FileData, GeneratedFileData, SharedProviderOptions, SharedProviderReference,
};
use crate::types::NamespacedKind;

/// A part of a user message: upstream's `TextPart | ImagePart | FilePart`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
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
    /// turns it into one and logs a deprecation warning. The JSON holds the
    /// bytes as a base64 string.
    Image {
        #[schemars(with = "String", extend("x-aimux-codec" = "base64"))]
        #[serde(with = "crate::wire::base64_bytes")]
        image: Vec<u8>,
        media_type: String,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
}

/// A part of an assistant message: upstream's `TextPart | CustomPart |
/// FilePart | ReasoningPart | ReasoningFilePart | ToolCallPart |
/// ToolResultPart | ToolApprovalRequest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
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

    /// A file (upstream `FilePart`, `provider-utils` `types/content-part.ts:62-102`):
    /// `data` is tagged `FileData` or a bare base64 string, URL string or
    /// provider reference, and a `data:` URL's media type wins over
    /// `media_type` when the prompt is converted.
    File {
        data: FilePartData,
        media_type: String,
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

    /// A file generated during reasoning (upstream `ReasoningFilePart`,
    /// `types/content-part.ts:146-180`).
    ReasoningFile {
        data: ReasoningFilePartData,
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
        #[serde(
            default,
            deserialize_with = "crate::wire::present",
            skip_serializing_if = "Option::is_none"
        )]
        #[schemars(extend("x-aimux-codec" = "presence"))]
        input_schema_input: Option<Value>,
    },
}

/// A part of a tool message: upstream's `ToolResultPart | ToolApprovalResponse`.
// Prompt content is built once per call and converted, not held in bulk, so
// the size gap between the two parts is not worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
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

    /// An image part (no provider options).
    pub fn image(image: Vec<u8>, media_type: impl Into<String>) -> Self {
        Self::Image {
            image,
            media_type: media_type.into(),
            provider_options: None,
        }
    }

    /// A file part with inline bytes, written as
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

    /// A file part whose inline data is an already-base64-encoded string,
    /// passed on verbatim (no provider options, no filename).
    pub fn file_base64(data: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self::file_data(
            FileData::Data {
                data: FileBytes::Base64(data.into()),
            },
            media_type,
        )
    }

    /// A file part referenced by URL, `data: { type: 'url', url }` (no
    /// provider options).
    pub fn file_url(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self::file_data(
            FileData::Url {
                url: url.into(),
                original_url: None,
            },
            media_type,
        )
    }

    /// A file part referenced by a provider reference,
    /// `data: { type: 'reference', reference: { [provider]: id } }` (no
    /// provider options, no filename).
    pub fn file_reference(
        media_type: impl Into<String>,
        reference: SharedProviderReference,
    ) -> Self {
        Self::file_data(FileData::Reference { reference }, media_type)
    }

    fn file_data(data: FileData, media_type: impl Into<String>) -> Self {
        Self::File {
            data: FilePartData::Tagged(data),
            media_type: media_type.into(),
            filename: None,
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

/// The result of a tool call (upstream `ToolResultPart`,
/// `provider-utils` `types/content-part.ts:240-268`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultPart {
    pub tool_call_id: String,
    /// The tool name. Required upstream; when absent here, the conversion
    /// takes it from the preceding tool call with the same id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub output: ToolResultOutput,
    /// Provider-specific options for this part (e.g. `anthropic.cacheControl`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// The output of a tool call (upstream `ToolResultOutput`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(extend("x-aimux-codec" = "ordered-untagged"))]
pub enum ToolResultFileId {
    Id(String),
    ByProvider(SharedProviderReference),
}

/// `FilePart.data` (`provider-utils` `types/content-part.ts:75`):
/// `FileData | DataContent | URL | ProviderReference`, in JSON a tagged
/// `FileData`, a string (base64 or a URL) or a bare `{ [provider]: id }`.
///
/// A string is read as the AI SDK reads it when it converts the prompt
/// (`ai` `prompt/file-part-data.ts:125-131`): a URL when it parses as one
/// (a `data:` URL becomes inline data), base64 otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
#[schemars(extend("x-aimux-codec" = "ordered-untagged"))]
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
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
#[schemars(extend("x-aimux-codec" = "ordered-untagged"))]
pub enum ReasoningFilePartData {
    /// `{ type: 'data' | 'url', ... }`.
    Tagged(GeneratedFileData),
    /// A base64 string or a URL string.
    String(String),
}

// ── Deserialize by JSON kind ────────────────────────────────────────────────
//
// The two file data types serialize untagged, but are read by JSON kind: an
// untagged `Deserialize` reports only "data did not match any variant", which
// hides the tagged data's own error (RFC-0042 §7.2,
// `rfc/0042-v4-type-boundaries.md:211`), and it read an object with an
// unknown `type` as a provider reference whose provider is "type".

/// An object with a `type` key is tagged file data and never a provider
/// reference, as the AI SDK decides (`ai@7.0.127`
/// `src/prompt/file-part-data.ts:14-21` `isTaggedFileData`;
/// `@ai-sdk/provider-utils@5.0.55` `src/is-provider-reference.ts:9-21`, which
/// requires `!('type' in data)`), so an unknown `type` is a decode error
/// naming it. A string is the string form; any other object is a reference.
impl<'de> Deserialize<'de> for FilePartData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KindVisitor;

        impl<'de> serde::de::Visitor<'de> for KindVisitor {
            type Value = FilePartData;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(
                    "file data: a tagged object, a base64 or URL string, or a provider reference",
                )
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(FilePartData::String(value.to_owned()))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(FilePartData::String(value))
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                let object = serde_json::Map::deserialize(
                    serde::de::value::MapAccessDeserializer::new(map),
                )?;
                let tagged = object.contains_key("type");
                let object = Value::Object(object);
                if tagged {
                    FileData::deserialize(object).map(FilePartData::Tagged)
                } else {
                    SharedProviderReference::deserialize(object).map(FilePartData::Reference)
                }
                .map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_any(KindVisitor)
    }
}

/// A string is the string form, an object is tagged generated-file data with
/// its own error.
impl<'de> Deserialize<'de> for ReasoningFilePartData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KindVisitor;

        impl<'de> serde::de::Visitor<'de> for KindVisitor {
            type Value = ReasoningFilePartData;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter
                    .write_str("reasoning file data: a tagged object or a base64 or URL string")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ReasoningFilePartData::String(value.to_owned()))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(ReasoningFilePartData::String(value))
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                GeneratedFileData::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(ReasoningFilePartData::Tagged)
            }
        }

        deserializer.deserialize_any(KindVisitor)
    }
}
