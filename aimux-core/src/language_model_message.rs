//! Provider-facing prompt types.
//!
//! Aligned with Vercel AI SDK `LanguageModelV4Prompt` / `LanguageModelV4Message`
//! (`provider/src/language-model/v4/language-model-v4-prompt.ts`): a prompt is
//! a list of messages, a message is a union by role, and there is exactly one
//! file part. `convert_to_language_model_prompt` is the single conversion from
//! the user-facing `ModelMessage`s.

use crate::content::ContentPart;
use crate::error::AiMuxError;
use crate::message::{MessageContent, ModelMessage, Role};
use crate::shared::{FileBytes, FileData, GeneratedFileData, SharedProviderOptions};
use std::borrow::Cow;
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
pub struct ToolResultPart {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: ToolResultOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// The provider-facing result of a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
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
#[serde(tag = "type", rename_all = "snake_case")]
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
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum UserPart {
    Text(TextPart),
    File(FilePart),
}

/// Parts allowed in an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
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
#[serde(tag = "type", rename_all = "snake_case")]
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
fn to_assistant_part(
    part: &ContentPart,
    tool_names: &HashMap<String, String>,
) -> Result<AssistantPart, AiMuxError> {
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
        } => file(
            FileData::Data {
                data: FileBytes::Binary(data.clone()),
            },
            media_type,
            filename,
            provider_options,
        ),
        ContentPart::FileBase64 {
            data,
            media_type,
            filename,
            provider_options,
        } => file(
            FileData::Data {
                data: FileBytes::Base64(data.clone()),
            },
            media_type,
            filename,
            provider_options,
        ),
        ContentPart::FileUrl {
            url,
            media_type,
            provider_options,
        } => file(
            FileData::Url {
                url: url.clone(),
                original_url: None,
            },
            media_type,
            &None,
            provider_options,
        ),
        ContentPart::FileReference {
            media_type,
            reference,
            filename,
            provider_options,
        } => file(
            FileData::Reference {
                reference: serde_json::from_value(reference.clone()).map_err(|e| {
                    AiMuxError::InvalidPrompt(format!("invalid file reference: {e}"))
                })?,
            },
            media_type,
            filename,
            provider_options,
        ),
        ContentPart::Reasoning {
            text,
            signature,
            provider_options,
        } => AssistantPart::Reasoning(ReasoningPart {
            text: text.clone(),
            provider_options: with_signature(
                provider_options,
                &["amazonBedrock", "bedrock", "anthropic"],
                "signature",
                signature,
            ),
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
            result,
            tool_name,
            is_error,
            provider_options,
            ..
        } => AssistantPart::ToolResult(ToolResultPart {
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name
                .as_ref()
                .or_else(|| tool_names.get(tool_call_id))
                .cloned()
                .ok_or_else(|| {
                    AiMuxError::InvalidPrompt(format!(
                        "tool result {tool_call_id} has no tool name or preceding tool call"
                    ))
                })?,
            output: if *is_error == Some(true) {
                ToolResultOutput::ErrorText {
                    value: match result {
                        Value::Null => "unknown error".into(),
                        Value::String(value) => value.clone(),
                        _ => result.to_string(),
                    },
                    provider_options: None,
                }
            } else if let Value::String(value) = result {
                ToolResultOutput::Text {
                    value: value.clone(),
                    provider_options: None,
                }
            } else {
                ToolResultOutput::Json {
                    value: result.clone(),
                    provider_options: None,
                }
            },
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
        } => AssistantPart::ReasoningFile(ReasoningFilePart {
            data: data.clone(),
            media_type: media_type.clone(),
            provider_options: provider_options.clone(),
        }),
        ContentPart::ToolApprovalRequest { .. } => {
            return Err(AiMuxError::InvalidPrompt(
                "tool approval requests are only allowed in assistant messages".into(),
            ));
        }
    })
}

fn with_signature(
    options: &Option<SharedProviderOptions>,
    namespaces: &[&str],
    key: &str,
    signature: &Option<String>,
) -> Option<SharedProviderOptions> {
    let Some(signature) = signature else {
        return options.clone();
    };
    let mut options = options.clone().unwrap_or_default();
    let namespace = namespaces
        .iter()
        .find(|name| options.contains_key(**name))
        .copied()
        .unwrap_or(if key == "signature" {
            "anthropic"
        } else {
            "google"
        });
    options
        .entry(namespace.to_owned())
        .or_default()
        .insert(key.to_owned(), Value::String(signature.clone()));
    Some(options)
}

/// Convert user-facing `ModelMessage`s into a `LanguageModelPrompt`.
///
/// - String content is normalized to a single text part.
/// - The five user-facing file variants become one `FilePart`.
/// - If `instructions` is provided, it is prepended as a system message.
///
/// # Errors
///
/// `InvalidPrompt` when a part is not allowed for its role (user: text/file;
/// tool: tool-result), a system message is not plain text, or a file
/// reference is not a `{ provider: id }` map.
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
        if msg.role == Role::Assistant {
            for part in parts.iter() {
                if let ContentPart::ToolCall {
                    tool_call_id,
                    tool_name,
                    ..
                } = part
                {
                    tool_names.insert(tool_call_id.clone(), tool_name.clone());
                }
            }
        }
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
            .map(|part| to_assistant_part(part, &tool_names))
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
