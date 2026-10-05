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
use crate::shared::{FileBytes, FileData, SharedProviderOptions};
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
pub struct ReasoningPart {
    pub text: String,
    pub signature: Option<String>,
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
    pub thought_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// Tool result content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolResultPart {
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub result: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preliminary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<bool>,
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
    ToolCall(ToolCallPart),
    ToolResult(ToolResultPart),
}

/// Parts allowed in a tool message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ToolPart {
    ToolResult(ToolResultPart),
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
            FileData::Url { url: url.clone() },
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
            signature: signature.clone(),
            provider_options: provider_options.clone(),
        }),
        ContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_executed,
            thought_signature,
            provider_options,
        } => AssistantPart::ToolCall(ToolCallPart {
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            input: input.clone(),
            provider_executed: *provider_executed,
            thought_signature: thought_signature.clone(),
            provider_options: provider_options.clone(),
        }),
        ContentPart::ToolResult {
            tool_call_id,
            result,
            tool_name,
            is_error,
            preliminary,
            dynamic,
            provider_options,
        } => AssistantPart::ToolResult(ToolResultPart {
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            result: result.clone(),
            is_error: *is_error,
            preliminary: *preliminary,
            dynamic: *dynamic,
            provider_options: provider_options.clone(),
        }),
    })
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
