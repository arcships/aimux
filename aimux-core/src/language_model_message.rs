//! Provider-facing prompt types.
//!
//! Aligned with Vercel AI SDK `LanguageModelV4Prompt` / `LanguageModelV4Message`
//! (`provider/src/language-model/v4/language-model-v4-prompt.ts`): a prompt is
//! a list of messages, a message is a union by role, and there is exactly one
//! file part. `convert_to_language_model_prompt` is the single conversion from
//! the user-facing `ModelMessage`s.

use crate::content::{self as user, AssistantContentPart, ToolContentPart, UserContentPart};
use crate::error::AiMuxError;
use crate::message::{AssistantContent, ModelMessage, UserContent};
use crate::shared::{FileBytes, FileData, GeneratedFileData, SharedProviderOptions};
use std::collections::HashMap;

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
#[serde(tag = "role", rename_all = "snake_case")]
#[ts(export)]
pub enum LanguageModelMessage {
    System {
        content: String,
        provider_options: Option<SharedProviderOptions>,
    },
    User {
        content: Vec<UserPart>,
        provider_options: Option<SharedProviderOptions>,
    },
    Assistant {
        content: Vec<AssistantPart>,
        provider_options: Option<SharedProviderOptions>,
    },
    Tool {
        content: Vec<ToolPart>,
        provider_options: Option<SharedProviderOptions>,
    },
}

/// Text content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TextPart {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// File content part (the only file part; `data` is a tagged `FileData`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FilePart {
    pub data: FileData,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Reasoning / thinking content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(rename = "LanguageModelReasoningPart")]
pub struct ReasoningPart {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Tool call content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolCallPart {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_executed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Tool result content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[ts(rename = "LanguageModelToolResultPart")]
pub struct ToolResultPart {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: ToolResultOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// The provider-facing result of a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export)]
#[ts(rename = "LanguageModelToolResultOutput")]
pub enum ToolResultOutput {
    Text {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    Json {
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    ExecutionDenied {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    ErrorText {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    ErrorJson {
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    Content {
        value: Vec<ToolResultContent>,
    },
}

/// Content within a tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export)]
pub enum ToolResultContent {
    Text(TextPart),
    File(FilePart),
    Custom {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
}

/// A file generated as part of reasoning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReasoningFilePart {
    pub data: GeneratedFileData,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Provider-specific content identified by its provider and kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CustomPart {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// The user's decision for a provider-executed tool approval request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolApprovalResponsePart {
    pub approval_id: String,
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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

fn file_part(
    data: FileData,
    media_type: &str,
    filename: Option<&String>,
    provider_options: Option<&SharedProviderOptions>,
) -> FilePart {
    FilePart {
        data,
        media_type: media_type.to_owned(),
        filename: filename.cloned(),
        provider_options: provider_options.cloned(),
    }
}

fn reference(reference: &Value) -> Result<FileData, AiMuxError> {
    Ok(FileData::Reference {
        reference: serde_json::from_value(reference.clone())
            .map_err(|e| AiMuxError::InvalidPrompt(format!("invalid file reference: {e}")))?,
    })
}

fn bytes(data: &[u8]) -> FileData {
    FileData::Data {
        data: FileBytes::Binary(data.to_vec()),
    }
}

fn base64(data: &str) -> FileData {
    FileData::Data {
        data: FileBytes::Base64(data.to_owned()),
    }
}

fn url(url: &str) -> FileData {
    FileData::Url {
        url: url.to_owned(),
        original_url: None,
    }
}

fn text_part(text: &str, provider_options: Option<&SharedProviderOptions>) -> TextPart {
    TextPart {
        text: text.to_owned(),
        provider_options: provider_options.cloned(),
    }
}

/// A user part as the provider sees it: text, or one `file` part.
fn user_part(part: &UserContentPart) -> Result<UserPart, AiMuxError> {
    Ok(match part {
        UserContentPart::Text {
            text,
            provider_options,
        } => UserPart::Text(text_part(text, provider_options.as_ref())),
        UserContentPart::Image {
            image,
            media_type,
            provider_options,
        } => UserPart::File(file_part(
            bytes(image),
            media_type,
            None,
            provider_options.as_ref(),
        )),
        UserContentPart::File {
            data,
            media_type,
            filename,
            provider_options,
        } => UserPart::File(file_part(
            bytes(data),
            media_type,
            filename.as_ref(),
            provider_options.as_ref(),
        )),
        UserContentPart::FileBase64 {
            data,
            media_type,
            filename,
            provider_options,
        } => UserPart::File(file_part(
            base64(data),
            media_type,
            filename.as_ref(),
            provider_options.as_ref(),
        )),
        UserContentPart::FileUrl {
            url: file_url,
            media_type,
            provider_options,
        } => UserPart::File(file_part(
            url(file_url),
            media_type,
            None,
            provider_options.as_ref(),
        )),
        UserContentPart::FileReference {
            media_type,
            reference: file_reference,
            filename,
            provider_options,
        } => UserPart::File(file_part(
            reference(file_reference)?,
            media_type,
            filename.as_ref(),
            provider_options.as_ref(),
        )),
    })
}

/// An assistant part as the provider sees it. `ToolApprovalRequest` is
/// filtered out before this is called.
fn assistant_part(
    part: &AssistantContentPart,
    tool_names: &HashMap<String, String>,
) -> Result<AssistantPart, AiMuxError> {
    Ok(match part {
        AssistantContentPart::Text {
            text,
            provider_options,
        } => AssistantPart::Text(text_part(text, provider_options.as_ref())),
        AssistantContentPart::Custom {
            kind,
            provider_options,
        } => AssistantPart::Custom(CustomPart {
            kind: kind.clone(),
            provider_options: provider_options.clone(),
        }),
        AssistantContentPart::File {
            data,
            media_type,
            filename,
            provider_options,
        } => AssistantPart::File(file_part(
            bytes(data),
            media_type,
            filename.as_ref(),
            provider_options.as_ref(),
        )),
        AssistantContentPart::FileBase64 {
            data,
            media_type,
            filename,
            provider_options,
        } => AssistantPart::File(file_part(
            base64(data),
            media_type,
            filename.as_ref(),
            provider_options.as_ref(),
        )),
        AssistantContentPart::FileUrl {
            url: file_url,
            media_type,
            provider_options,
        } => AssistantPart::File(file_part(
            url(file_url),
            media_type,
            None,
            provider_options.as_ref(),
        )),
        AssistantContentPart::FileReference {
            media_type,
            reference: file_reference,
            filename,
            provider_options,
        } => AssistantPart::File(file_part(
            reference(file_reference)?,
            media_type,
            filename.as_ref(),
            provider_options.as_ref(),
        )),
        AssistantContentPart::Reasoning {
            text,
            provider_options,
        } => AssistantPart::Reasoning(ReasoningPart {
            text: text.clone(),
            provider_options: provider_options.clone(),
        }),
        AssistantContentPart::ReasoningFile {
            data,
            media_type,
            provider_options,
        } => AssistantPart::ReasoningFile(ReasoningFilePart {
            data: data.clone(),
            media_type: media_type.clone(),
            provider_options: provider_options.clone(),
        }),
        AssistantContentPart::ToolCall {
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
        AssistantContentPart::ToolResult(result) => {
            AssistantPart::ToolResult(tool_result_part(result, tool_names)?)
        }
        AssistantContentPart::ToolApprovalRequest { .. } => {
            unreachable!("tool approval requests are filtered out before conversion")
        }
    })
}

fn tool_result_part(
    part: &user::ToolResultPart,
    tool_names: &HashMap<String, String>,
) -> Result<ToolResultPart, AiMuxError> {
    let tool_call_id = &part.tool_call_id;
    Ok(ToolResultPart {
        tool_call_id: tool_call_id.clone(),
        tool_name: part
            .tool_name
            .as_ref()
            .or_else(|| tool_names.get(tool_call_id))
            .cloned()
            .ok_or_else(|| {
                AiMuxError::InvalidPrompt(format!(
                    "tool result {tool_call_id} has no tool name or preceding tool call"
                ))
            })?,
        output: tool_result_output(&part.output),
        provider_options: part.provider_options.clone(),
    })
}

/// Upstream `mapToolResultOutput` (`ai` `prompt/convert-to-language-model-prompt.ts:610`).
fn tool_result_output(output: &user::ToolResultOutput) -> ToolResultOutput {
    match output.clone() {
        user::ToolResultOutput::Text {
            value,
            provider_options,
        } => ToolResultOutput::Text {
            value,
            provider_options,
        },
        user::ToolResultOutput::Json {
            value,
            provider_options,
        } => ToolResultOutput::Json {
            value,
            provider_options,
        },
        user::ToolResultOutput::ExecutionDenied {
            reason,
            provider_options,
        } => ToolResultOutput::ExecutionDenied {
            reason,
            provider_options,
        },
        user::ToolResultOutput::ErrorText {
            value,
            provider_options,
        } => ToolResultOutput::ErrorText {
            value,
            provider_options,
        },
        user::ToolResultOutput::ErrorJson {
            value,
            provider_options,
        } => ToolResultOutput::ErrorJson {
            value,
            provider_options,
        },
        user::ToolResultOutput::Content { value } => ToolResultOutput::Content {
            value: value
                .into_iter()
                .map(|item| match item {
                    user::ToolResultContentPart::Text {
                        text,
                        provider_options,
                    } => ToolResultContent::Text(TextPart {
                        text,
                        provider_options,
                    }),
                    user::ToolResultContentPart::File {
                        data,
                        media_type,
                        filename,
                        provider_options,
                    } => ToolResultContent::File(FilePart {
                        data,
                        media_type,
                        filename,
                        provider_options,
                    }),
                    user::ToolResultContentPart::Custom { provider_options } => {
                        ToolResultContent::Custom { provider_options }
                    }
                })
                .collect(),
        },
    }
}

/// Convert one message (upstream `convertToLanguageModelMessage`,
/// `ai` `prompt/convert-to-language-model-prompt.ts:193`).
fn convert_message(
    message: &ModelMessage,
    tool_names: &HashMap<String, String>,
) -> Result<LanguageModelMessage, AiMuxError> {
    Ok(match message {
        ModelMessage::System {
            content,
            provider_options,
        } => LanguageModelMessage::System {
            content: content.clone(),
            provider_options: provider_options.clone(),
        },
        ModelMessage::User {
            content,
            provider_options,
        } => LanguageModelMessage::User {
            content: match content {
                UserContent::Text(text) => vec![UserPart::Text(text_part(text, None))],
                UserContent::Parts(parts) => parts
                    .iter()
                    .map(user_part)
                    .filter(|part| {
                        // remove empty text parts (`:242-243`)
                        !matches!(part, Ok(UserPart::Text(TextPart { text, .. })) if text.is_empty())
                    })
                    .collect::<Result<_, _>>()?,
            },
            provider_options: provider_options.clone(),
        },
        ModelMessage::Assistant {
            content,
            provider_options,
        } => LanguageModelMessage::Assistant {
            content: match content {
                AssistantContent::Text(text) => vec![AssistantPart::Text(text_part(text, None))],
                AssistantContent::Parts(parts) => parts
                    .iter()
                    // remove empty text parts without provider options (`:263-269`)
                    .filter(|part| {
                        !matches!(part, AssistantContentPart::Text { text, provider_options: None }
                            if text.is_empty())
                    })
                    // approval requests are not sent to the provider (`:270-281`)
                    .filter(|part| !matches!(part, AssistantContentPart::ToolApprovalRequest { .. }))
                    .map(|part| assistant_part(part, tool_names))
                    .collect::<Result<_, _>>()?,
            },
            provider_options: provider_options.clone(),
        },
        ModelMessage::Tool {
            content,
            provider_options,
        } => LanguageModelMessage::Tool {
            content: content
                .iter()
                .filter_map(|part| match part {
                    ToolContentPart::ToolResult(result) => {
                        Some(tool_result_part(result, tool_names).map(ToolPart::ToolResult))
                    }
                    // only a provider-executed tool's approval reaches the
                    // provider (`:374-378`, `:395-402`)
                    ToolContentPart::ToolApprovalResponse {
                        approval_id,
                        approved,
                        reason,
                        provider_executed,
                    } => (*provider_executed == Some(true)).then(|| {
                        Ok(ToolPart::ToolApprovalResponse(ToolApprovalResponsePart {
                            approval_id: approval_id.clone(),
                            approved: *approved,
                            reason: reason.clone(),
                            provider_options: None,
                        }))
                    }),
                })
                .collect::<Result<_, _>>()?,
            provider_options: provider_options.clone(),
        },
    })
}

/// Convert user-facing `ModelMessage`s into a `LanguageModelPrompt`
/// (upstream `convertToLanguageModelPrompt`,
/// `ai` `prompt/convert-to-language-model-prompt.ts:36`).
///
/// - `instructions`, when given, becomes the leading system message.
/// - Each message keeps its role and its message-level `provider_options`.
/// - String content becomes a single text part.
/// - The five user-facing file variants become one `FilePart`.
/// - Empty text parts are dropped (on an assistant message only when they
///   have no provider options), as are assistant tool approval requests and
///   tool approval responses for tools the provider does not execute; a tool
///   message left empty is dropped.
///
/// # Errors
///
/// `InvalidPrompt` when a tool result has no tool name and no preceding tool
/// call, or a file reference is not a `{ provider: id }` map.
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

    let mut tool_names = HashMap::new();
    for message in messages {
        if let ModelMessage::Assistant {
            content: AssistantContent::Parts(parts),
            ..
        } = message
        {
            for part in parts {
                if let AssistantContentPart::ToolCall {
                    tool_call_id,
                    tool_name,
                    ..
                } = part
                {
                    tool_names.insert(tool_call_id.clone(), tool_name.clone());
                }
            }
        }
        result.push(convert_message(message, &tool_names)?);
    }

    // Filter out empty tool messages, e.g. one that held only tool approval
    // responses that were removed (`:177-183`).
    result.retain(|message| {
        !matches!(message, LanguageModelMessage::Tool { content, .. } if content.is_empty())
    });
    Ok(result)
}
