//! Provider-facing prompt types.
//!
//! Aligned with Vercel AI SDK `LanguageModelV4Prompt` / `LanguageModelV4Message`
//! (`provider/src/language-model/v4/language-model-v4-prompt.ts`): a prompt is
//! a list of messages, a message is a union by role, and there is exactly one
//! file part. `convert_to_language_model_prompt` is the single conversion from
//! the user-facing `ModelMessage`s.

use crate::content::{ContentPart, FilePartData, ReasoningFilePartData};
use crate::error::AiMuxError;
use crate::message::{MessageContent, ModelMessage, Role};
use crate::shared::{FileBytes, FileData, GeneratedFileData, SharedProviderOptions};
use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// The standardized prompt passed to `LanguageModel::do_generate` / `do_stream`.
pub type LanguageModelPrompt = Vec<LanguageModelMessage>;

/// A single provider-facing message, a union by role.
///
/// `provider_options` is message-level (e.g. `anthropic.cacheControl`),
/// mirroring `LanguageModelV4Message.providerOptions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "role",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum LanguageModelMessage {
    System {
        content: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    User {
        content: Vec<UserPart>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    Assistant {
        content: Vec<AssistantPart>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    Tool {
        content: Vec<ToolPart>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
}

/// Text content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextPart {
    pub text: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// File content part (the only file part; `data` is a tagged `FileData`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FilePart {
    pub data: FileData,
    pub media_type: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Reasoning / thinking content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[ts(rename = "LanguageModelReasoningPart")]
pub struct ReasoningPart {
    pub text: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Tool call content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolCallPart {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: Value,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_executed: Option<bool>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Tool result content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolResultPart {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: ToolResultOutput,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// The provider-facing result of a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ToolResultOutput {
    Text {
        value: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    Json {
        value: Value,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    ExecutionDenied {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    ErrorText {
        value: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    ErrorJson {
        value: Value,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    Content {
        value: Vec<ToolResultContent>,
    },
}

/// Content within a tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ToolResultContent {
    Text(TextPart),
    File(FilePart),
    Custom {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
}

/// A file generated as part of reasoning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReasoningFilePart {
    pub data: GeneratedFileData,
    pub media_type: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Provider-specific content identified by its provider and kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CustomPart {
    pub kind: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// The user's decision for a provider-executed tool approval request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolApprovalResponsePart {
    pub approval_id: String,
    pub approved: bool,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Parts allowed in a user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export)]
pub enum UserPart {
    Text(TextPart),
    File(FilePart),
}

/// Parts allowed in an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export)]
pub enum AssistantPart {
    Text(TextPart),
    File(FilePart),
    Reasoning(ReasoningPart),
    ReasoningFile(ReasoningFilePart),
    Custom(CustomPart),
    ToolCall(ToolCallPart),
    ToolResult(ToolResultPart),
}

/// Parts allowed in a tool message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export)]
pub enum ToolPart {
    ToolResult(ToolResultPart),
    ToolApprovalResponse(ToolApprovalResponsePart),
}

impl LanguageModelMessage {
    /// User message with a single text part.
    #[must_use]
    pub fn user_text(text: impl Into<String>) -> Self {
        Self::User {
            content: vec![UserPart::Text(TextPart {
                text: text.into(),
                provider_options: None,
            })],
            provider_options: None,
        }
    }
}

/// Map a user-facing part onto the widest provider part (the assistant union);
/// the role then narrows it.
fn to_assistant_part(part: &ContentPart) -> Result<AssistantPart, AiMuxError> {
    let file = |data, media_type: &String, filename: &Option<String>, po: &Option<_>| {
        AssistantPart::File(FilePart {
            data,
            media_type: media_type.clone(),
            filename: filename.clone(),
            provider_options: po.clone(),
        })
    };
    Ok(match part {
        ContentPart::Text {
            text,
            provider_options,
        } => AssistantPart::Text(TextPart {
            text: text.clone(),
            provider_options: provider_options.clone(),
        }),
        ContentPart::Image {
            image,
            media_type,
            provider_options,
        } => file(
            FileData::Data {
                data: FileBytes::Binary(image.clone()),
            },
            media_type,
            &None,
            provider_options,
        ),
        ContentPart::File {
            data,
            media_type,
            filename,
            provider_options,
        } => {
            let (data, data_url_media_type) = convert_file_part_data(data)?;
            file(
                data,
                data_url_media_type.as_ref().unwrap_or(media_type),
                filename,
                provider_options,
            )
        }
        ContentPart::Reasoning {
            text,
            provider_options,
            ..
        } => AssistantPart::Reasoning(ReasoningPart {
            text: text.clone(),
            provider_options: provider_options.clone(),
        }),
        ContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_executed,
            provider_options,
        } => AssistantPart::ToolCall(ToolCallPart {
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            input: input.clone(),
            provider_executed: *provider_executed,
            provider_options: provider_options.clone(),
        }),
        ContentPart::ToolResult {
            tool_call_id,
            tool_name,
            output,
            provider_options,
        } => AssistantPart::ToolResult(ToolResultPart {
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            output: output.clone(),
            provider_options: provider_options.clone(),
        }),
        ContentPart::Custom {
            kind,
            provider_options,
        } => AssistantPart::Custom(CustomPart {
            kind: kind.clone(),
            provider_options: provider_options.clone(),
        }),
        ContentPart::ReasoningFile {
            data,
            media_type,
            provider_options,
        } => {
            let data = match data {
                ReasoningFilePartData::Tagged(GeneratedFileData::Data { data }) => {
                    FilePartData::Tagged(FileData::Data { data: data.clone() })
                }
                ReasoningFilePartData::Tagged(GeneratedFileData::Url { url, original_url }) => {
                    FilePartData::Tagged(FileData::Url {
                        url: url.clone(),
                        original_url: original_url.clone(),
                    })
                }
                ReasoningFilePartData::String(text) => FilePartData::String(text.clone()),
            };
            // `convert-to-language-model-prompt.ts:313-327`.
            let (data, data_url_media_type) = convert_file_part_data(&data)?;
            let data = match data {
                FileData::Data { data } => GeneratedFileData::Data { data },
                FileData::Url { url, original_url } => GeneratedFileData::Url { url, original_url },
                FileData::Reference { .. } | FileData::Text { .. } => {
                    return Err(AiMuxError::InvalidPrompt(
                        "Unsupported reasoning-file data type".into(),
                    ));
                }
            };
            AssistantPart::ReasoningFile(ReasoningFilePart {
                data,
                media_type: data_url_media_type.unwrap_or_else(|| media_type.clone()),
                provider_options: provider_options.clone(),
            })
        }
        ContentPart::ToolApprovalRequest { .. } => {
            return Err(AiMuxError::InvalidPrompt(
                "tool approval requests are only allowed in assistant messages".into(),
            ));
        }
    })
}

/// `convertToLanguageModelV4FilePart` (`ai` `prompt/file-part-data.ts:89-141`):
/// the tagged provider `FileData` for a file part's `data`, and the media type
/// of a `data:` URL, which takes precedence over the part's `mediaType`.
///
/// A string is a URL when it parses as one and base64 otherwise; a `data:`
/// URL becomes inline base64 data; a URL string that parsing changes keeps
/// the given text as `originalUrl`.
///
/// # Errors
///
/// `InvalidPrompt` (upstream `InvalidDataContentError`) for a `data:` URL
/// passed as inline data or a `data:` URL without a comma.
pub fn convert_file_part_data(
    data: &FilePartData,
) -> Result<(FileData, Option<String>), AiMuxError> {
    match data {
        FilePartData::Tagged(FileData::Data {
            data: FileBytes::Base64(text),
        }) if text.starts_with("data:") => Err(AiMuxError::InvalidPrompt(
            "Data URLs are not valid inline data. Pass them as { type: \"url\", url } instead."
                .into(),
        )),
        FilePartData::Tagged(FileData::Url { url, original_url }) => {
            convert_url_to_file_part_data(url, original_url.clone())
        }
        FilePartData::Tagged(data) => Ok((data.clone(), None)),
        FilePartData::String(text) => match url::Url::parse(text) {
            Ok(parsed) => {
                let (mut data, media_type) = convert_url_to_file_part_data(parsed.as_str(), None)?;
                if let FileData::Url { url, original_url } = &mut data
                    && url != text
                {
                    *original_url = Some(text.clone());
                }
                Ok((data, media_type))
            }
            Err(_) => Ok((
                FileData::Data {
                    data: FileBytes::Base64(text.clone()),
                },
                None,
            )),
        },
        FilePartData::Reference(reference) => Ok((
            FileData::Reference {
                reference: reference.clone(),
            },
            None,
        )),
    }
}

/// `convertUrlToFilePartData` (`file-part-data.ts:27-48`) with `splitDataUrl`
/// (`split-data-url.ts`): a `data:` URL is split into its media type and
/// base64 content; any other URL stays a URL.
fn convert_url_to_file_part_data(
    url: &str,
    original_url: Option<String>,
) -> Result<(FileData, Option<String>), AiMuxError> {
    let data_url = match url::Url::parse(url) {
        Ok(parsed) if parsed.scheme() == "data" => parsed,
        _ => {
            return Ok((
                FileData::Url {
                    url: url.to_owned(),
                    original_url,
                },
                None,
            ));
        }
    };
    let url = data_url.as_str();
    let mut pieces = url.split(',');
    let header = pieces.next().unwrap_or_default();
    let media_type = header
        .split(';')
        .next()
        .and_then(|scheme_and_type| scheme_and_type.split(':').nth(1));
    match (media_type, pieces.next()) {
        (Some(media_type), Some(base64_content)) => Ok((
            FileData::Data {
                data: FileBytes::Base64(base64_content.to_owned()),
            },
            Some(media_type.to_owned()),
        )),
        _ => Err(AiMuxError::InvalidPrompt(format!(
            "Invalid data URL format in content {url}"
        ))),
    }
}

/// Convert user-facing `ModelMessage`s into a `LanguageModelPrompt`.
///
/// - String content is normalized to a single text part.
/// - Image and file parts become one `FilePart`; file `data` is converted as
///   `convert_file_part_data` describes.
/// - If `instructions` is provided, it is prepended as a system message.
///
/// # Errors
///
/// `InvalidPrompt` when a part is not allowed for its role (user: text/file;
/// tool: tool-result), a system message is not plain text, or a file part's
/// data is a `data:` URL given as inline data or a malformed `data:` URL.
pub fn convert_to_language_model_prompt(
    messages: &[ModelMessage],
    instructions: Option<&str>,
) -> Result<LanguageModelPrompt, AiMuxError> {
    let mut result = Vec::new();
    if let Some(instructions) = instructions {
        result.push(LanguageModelMessage::System {
            content: instructions.to_string(),
            provider_options: None,
        });
    }

    for msg in messages {
        let bad = |what: &str| AiMuxError::InvalidPrompt(format!("{:?} message: {what}", msg.role));
        if msg.role == Role::System {
            match &msg.content {
                MessageContent::Text(text) => result.push(LanguageModelMessage::System {
                    content: text.clone(),
                    provider_options: None,
                }),
                _ => return Err(bad("system content must be plain text")),
            }
            continue;
        }
        let parts: Cow<'_, [ContentPart]> = match &msg.content {
            MessageContent::Text(text) => Cow::Owned(vec![ContentPart::text(text)]),
            MessageContent::Parts(parts) => Cow::Borrowed(parts),
        };
        let parts = parts
            .iter()
            .filter(|part| {
                msg.role != Role::Assistant
                    || !matches!(part, ContentPart::ToolApprovalRequest { .. })
            })
            .filter(|part| {
                !matches!(&msg.content, MessageContent::Parts(_))
                    || !matches!(part, ContentPart::Text { text, provider_options }
                        if text.is_empty() && (msg.role == Role::User
                            || (msg.role == Role::Assistant && provider_options.is_none())))
            })
            .map(to_assistant_part)
            .collect::<Result<Vec<_>, _>>()?;
        let provider_options = None;
        result.push(match msg.role {
            Role::System => unreachable!(),
            Role::Assistant => LanguageModelMessage::Assistant {
                content: parts,
                provider_options,
            },
            Role::User => LanguageModelMessage::User {
                content: parts
                    .into_iter()
                    .map(|p| match p {
                        AssistantPart::Text(t) => Ok(UserPart::Text(t)),
                        AssistantPart::File(f) => Ok(UserPart::File(f)),
                        _ => Err(bad("only text and file parts are allowed")),
                    })
                    .collect::<Result<_, _>>()?,
                provider_options,
            },
            Role::Tool => LanguageModelMessage::Tool {
                content: parts
                    .into_iter()
                    .map(|p| match p {
                        AssistantPart::ToolResult(r) => Ok(ToolPart::ToolResult(r)),
                        _ => Err(bad("only tool-result parts are allowed")),
                    })
                    .collect::<Result<_, _>>()?,
                provider_options,
            },
        });
    }

    Ok(result)
}
