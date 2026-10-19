//! Provider-facing prompt types.
//!
//! Aligned with Vercel AI SDK `LanguageModelV4Prompt` / `LanguageModelV4Message`
//! (`provider/src/language-model/v4/language-model-v4-prompt.ts`): a prompt is
//! a list of messages, a message is a union by role, and there is exactly one
//! file part. `convert_to_language_model_prompt` is the single conversion from
//! the user-facing `ModelMessage`s.

use crate::content::{
    self as user, AssistantContentPart, FilePartData, ReasoningFilePartData, ToolContentPart,
    UserContentPart,
};
use crate::download::{DownloadFunction, DownloadedFile, PlannedDownload};
use crate::error::AiMuxError;
use crate::error::ErrorCause;
use crate::language_model::SupportedUrls;
use crate::message::{AssistantContent, ModelMessage, UserContent};
use crate::shared::{
    FileBytes, FileData, GeneratedFileData, SharedProviderOptions, SharedProviderReference,
};
use crate::types::{NamespacedKind, Warning};
use indexmap::IndexSet;
use std::collections::{HashMap, HashSet};

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
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum LanguageModelMessage {
    System {
        content: String,
        #[ts(optional)]
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    User {
        content: Vec<UserPart>,
        #[ts(optional)]
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    Assistant {
        content: Vec<AssistantPart>,
        #[ts(optional)]
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    Tool {
        content: Vec<ToolPart>,
        #[ts(optional)]
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
}

/// Text content part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct TextPart {
    pub text: String,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// File content part (the only file part; `data` is a tagged `FileData`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
#[ts(rename = "LanguageModelToolResultPart")]
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
#[cfg_attr(feature = "ts-export", ts(export))]
#[ts(rename = "LanguageModelToolResultOutput")]
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
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct CustomPart {
    pub kind: NamespacedKind,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// The user's decision for a provider-executed tool approval request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum UserPart {
    Text(TextPart),
    File(FilePart),
}

/// Parts allowed in an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[cfg_attr(feature = "ts-export", ts(export))]
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

fn text_part(text: &str, provider_options: Option<&SharedProviderOptions>) -> TextPart {
    TextPart {
        text: text.to_owned(),
        provider_options: provider_options.cloned(),
    }
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
        }) if text.starts_with("data:") => Err(AiMuxError::invalid_prompt(
            "Data URLs are not valid inline data. Pass them as { type: \"url\", url } instead.",
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
        _ => Err(AiMuxError::invalid_prompt(format!(
            "Invalid data URL format in content {url}"
        ))),
    }
}

/// The files downloaded for a prompt's URLs, by URL.
pub(crate) type DownloadedAssets = HashMap<String, DownloadedFile>;

/// A file part holding a file downloaded for its URL (upstream
/// `convertPartToLanguageModelPart`, `:567-578`): the downloaded media type
/// replaces the part's when the part's is not a full `type/subtype`.
fn downloaded_file_part(
    file: &DownloadedFile,
    media_type: &str,
    filename: Option<String>,
    provider_options: Option<SharedProviderOptions>,
) -> FilePart {
    let media_type = match &file.media_type {
        Some(downloaded) if !is_full_media_type(media_type) => downloaded,
        _ => media_type,
    };
    FilePart {
        data: bytes(&file.data),
        media_type: media_type.to_owned(),
        filename,
        provider_options,
    }
}

/// Upstream `isFullMediaType` (provider-utils@5.0.55
/// `detect-media-type.ts:381-388`): `type/subtype` with a subtype that is not
/// empty and not `*`.
fn is_full_media_type(media_type: &str) -> bool {
    media_type
        .split_once('/')
        .is_some_and(|(_, subtype)| !subtype.is_empty() && subtype != "*")
}

/// Download the prompt's file URLs, the AI SDK's `downloadAssets`
/// (`ai` `prompt/convert-to-language-model-prompt.ts:444-540`).
///
/// It plans the URL of every user `file` part and of every `file` item of
/// a `content` tool output (in tool and assistant messages), in prompt
/// order, marks the URLs the model fetches itself (`supported_urls`), and
/// calls `download` once with all of them. As upstream, assistant file parts
/// and the deprecated `file-url` / `image-url` tool output items are not
/// downloaded. Neither is a `data:` URL, which upstream turns into inline
/// data before planning and aimux sends as it is.
///
/// # Errors
///
/// The download function's error.
pub(crate) async fn download_assets(
    messages: &[ModelMessage],
    download: &DownloadFunction,
    supported_urls: &SupportedUrls,
) -> Result<DownloadedAssets, AiMuxError> {
    fn plan_output(output: &user::ToolResultOutput, plan: &mut impl FnMut(&str, &str)) {
        if let user::ToolResultOutput::Content { value } = output {
            for item in value {
                if let user::ToolResultContentPart::File {
                    data: FileData::Url { url, .. },
                    media_type,
                    ..
                } = item
                {
                    plan(url, media_type);
                }
            }
        }
    }

    let mut planned = Vec::new();
    let mut plan = |url: &str, media_type: &str| {
        if url
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
        {
            return;
        }
        planned.push(PlannedDownload {
            url: url.to_owned(),
            is_url_supported_by_model: supported_urls.is_supported(url, media_type),
        });
    };
    for message in messages {
        match message {
            ModelMessage::User {
                content: UserContent::Parts(parts),
                ..
            } => {
                for part in parts {
                    if let UserContentPart::File {
                        data, media_type, ..
                    } = part
                        && let Ok((FileData::Url { url, .. }, data_url_media_type)) =
                            convert_file_part_data(data)
                    {
                        plan(&url, data_url_media_type.as_deref().unwrap_or(media_type));
                    }
                }
            }
            ModelMessage::Tool { content, .. } => {
                for part in content {
                    if let ToolContentPart::ToolResult(result) = part {
                        plan_output(&result.output, &mut plan);
                    }
                }
            }
            ModelMessage::Assistant {
                content: AssistantContent::Parts(parts),
                ..
            } => {
                for part in parts {
                    if let AssistantContentPart::ToolResult(result) = part {
                        plan_output(&result.output, &mut plan);
                    }
                }
            }
            _ => {}
        }
    }
    let files = download.download(planned.clone()).await?;
    Ok(planned
        .into_iter()
        .zip(files)
        .filter_map(|(planned, file)| file.map(|file| (planned.url, file)))
        .collect())
}

/// A user part as the provider sees it: text, or one `file` part.
///
/// An `image` part is deprecated upstream (`provider-utils`
/// `types/content-part.ts:29`); it becomes a `file` part with a deprecation
/// warning (`ai` `prompt/convert-to-language-model-prompt.ts:229-238`).
fn user_part(
    part: &UserContentPart,
    assets: &DownloadedAssets,
    warnings: &mut Vec<Warning>,
) -> Result<UserPart, AiMuxError> {
    Ok(match part {
        UserContentPart::Text {
            text,
            provider_options,
        } => UserPart::Text(text_part(text, provider_options.as_ref())),
        UserContentPart::Image {
            image,
            media_type,
            provider_options,
        } => {
            warnings.push(Warning::Deprecated {
                setting: r#""image" content part"#.into(),
                message: r#"The "image" content part type is deprecated. Use a "file" part with mediaType: 'image' (or a more specific image/* subtype) instead."#.into(),
            });
            UserPart::File(file_part(
                bytes(image),
                media_type,
                None,
                provider_options.as_ref(),
            ))
        }
        UserContentPart::File {
            data,
            media_type,
            filename,
            provider_options,
        } => {
            let (data, data_url_media_type) = convert_file_part_data(data)?;
            let media_type = data_url_media_type.as_deref().unwrap_or(media_type);
            UserPart::File(match &data {
                FileData::Url { url, .. } if assets.contains_key(url) => downloaded_file_part(
                    &assets[url],
                    media_type,
                    filename.clone(),
                    provider_options.clone(),
                ),
                _ => file_part(
                    data,
                    media_type,
                    filename.as_ref(),
                    provider_options.as_ref(),
                ),
            })
        }
    })
}

/// An assistant part as the provider sees it. `ToolApprovalRequest` is
/// filtered out before this is called.
fn assistant_part(
    part: &AssistantContentPart,
    context: &mut Context<'_>,
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
        } => {
            let (data, data_url_media_type) = convert_file_part_data(data)?;
            AssistantPart::File(file_part(
                data,
                data_url_media_type.as_deref().unwrap_or(media_type),
                filename.as_ref(),
                provider_options.as_ref(),
            ))
        }
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
                    return Err(AiMuxError::invalid_prompt(
                        "Unsupported reasoning-file data type",
                    ));
                }
            };
            AssistantPart::ReasoningFile(ReasoningFilePart {
                data,
                media_type: data_url_media_type.unwrap_or_else(|| media_type.clone()),
                provider_options: provider_options.clone(),
            })
        }
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
            AssistantPart::ToolResult(tool_result_part(result, context)?)
        }
        AssistantContentPart::ToolApprovalRequest { .. } => {
            unreachable!("tool approval requests are filtered out before conversion")
        }
    })
}

/// State shared by the conversion of all messages of one prompt.
struct Context<'a> {
    /// Tool names by tool call id, from the assistant tool calls seen so far.
    tool_names: HashMap<String, String>,
    /// The provider id (`'openai'` for `'openai.chat'`), used only to turn a
    /// deprecated string `fileId` into a provider reference.
    provider: Option<&'a str>,
    /// Deprecation warnings of the message being converted.
    warnings: Vec<Warning>,
    /// The files downloaded for the prompt's URLs.
    assets: &'a DownloadedAssets,
}

fn tool_result_part(
    part: &user::ToolResultPart,
    context: &mut Context<'_>,
) -> Result<ToolResultPart, AiMuxError> {
    let tool_call_id = &part.tool_call_id;
    Ok(ToolResultPart {
        tool_call_id: tool_call_id.clone(),
        tool_name: part
            .tool_name
            .as_ref()
            .or_else(|| context.tool_names.get(tool_call_id))
            .cloned()
            .ok_or_else(|| {
                AiMuxError::invalid_prompt(format!(
                    "tool result {tool_call_id} has no tool name or preceding tool call"
                ))
            })?,
        output: tool_result_output(
            &part.output,
            context.provider,
            context.assets,
            &mut context.warnings,
        )?,
        provider_options: part.provider_options.clone(),
    })
}

/// Upstream `mapToolResultOutput` (`ai` `prompt/convert-to-language-model-prompt.ts:610-797`).
///
/// Only a `content` output changes: each deprecated item becomes a `file`
/// part, with upstream's deprecation warning.
fn tool_result_output(
    output: &user::ToolResultOutput,
    provider: Option<&str>,
    assets: &DownloadedAssets,
    warnings: &mut Vec<Warning>,
) -> Result<ToolResultOutput, AiMuxError> {
    Ok(match output.clone() {
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
                .map(|item| tool_result_content(item, provider, assets, warnings))
                .collect::<Result<_, _>>()?,
        },
    })
}

fn deprecated(warnings: &mut Vec<Warning>, kind: &str, message: String) {
    warnings.push(Warning::Deprecated {
        setting: format!(r#""tool-result" content of type "{kind}""#),
        message,
    });
}

fn deprecated_reference(warnings: &mut Vec<Warning>, kind: &str) {
    deprecated(
        warnings,
        kind,
        format!(
            r#"The "{kind}" type for tool result content is deprecated. Use the "file" type with mediaType and {{ type: 'reference', reference }} instead."#
        ),
    );
}

/// A URL as upstream's `new URL(url)` gives it to the provider: normalized,
/// with the given spelling kept as `original_url` when normalizing changed
/// it (`:666-676`, `:741-747`).
fn normalized_url(given: &str) -> Result<FileData, AiMuxError> {
    let parsed = url::Url::parse(given).map_err(|e| AiMuxError::InvalidPrompt {
        message: format!("Invalid URL: {given}: {e}"),
        cause: Some(ErrorCause::source(&e)),
    })?;
    let normalized = parsed.to_string();
    Ok(FileData::Url {
        original_url: (normalized != given).then(|| given.to_owned()),
        url: normalized,
    })
}

fn file_reference(reference: SharedProviderReference) -> FileData {
    FileData::Reference { reference }
}

/// One item of a `content` tool output; the arms follow upstream's `switch`
/// (`:631-795`).
fn tool_result_content(
    item: user::ToolResultContentPart,
    provider: Option<&str>,
    assets: &DownloadedAssets,
    warnings: &mut Vec<Warning>,
) -> Result<ToolResultContent, AiMuxError> {
    use user::ToolResultContentPart as Item;
    let file = |data, media_type: &str, filename, provider_options| {
        ToolResultContent::File(FilePart {
            data,
            media_type: media_type.to_owned(),
            filename,
            provider_options,
        })
    };
    Ok(match item {
        Item::Text {
            text,
            provider_options,
        } => ToolResultContent::Text(TextPart {
            text,
            provider_options,
        }),
        Item::File {
            data,
            media_type,
            filename,
            provider_options,
        } => match &data {
            FileData::Url { url, .. } if assets.contains_key(url) => ToolResultContent::File(
                downloaded_file_part(&assets[url], &media_type, filename, provider_options),
            ),
            _ => file(data, &media_type, filename, provider_options),
        },
        Item::FileData {
            data,
            media_type,
            filename,
            provider_options,
        } => {
            deprecated(
                warnings,
                "file-data",
                r#"The "file-data" type for tool result content is deprecated. Use the "file" type with mediaType and { type: 'data', data } instead."#.into(),
            );
            file(base64(&data), &media_type, filename, provider_options)
        }
        Item::FileUrl {
            url: given,
            media_type,
            provider_options,
        } => {
            let data = normalized_url(&given)?;
            let deprecation = r#"The "file-url" type for tool result content is deprecated. Use the "file" type with mediaType and { type: 'url', url } instead."#;
            let (media_type, message) = match media_type {
                Some(media_type) => (media_type, deprecation.to_owned()),
                None => {
                    let inferred = media_type_from_url(&given);
                    let inference = if inferred == OCTET_STREAM {
                        "Unable to infer media type from URL. Defaulting to 'application/octet-stream'."
                            .to_owned()
                    } else {
                        format!("Inferred media type '{inferred}' from URL.")
                    };
                    (
                        inferred.to_owned(),
                        format!(
                            r#"The "file-url" tool result content part with URL "{given}" is missing a "mediaType". {inference} {deprecation}"#
                        ),
                    )
                }
            };
            deprecated(warnings, "file-url", message);
            file(data, &media_type, None, provider_options)
        }
        Item::FileId {
            file_id,
            provider_options,
        } => {
            deprecated_reference(warnings, "file-id");
            let reference = file_id_reference(file_id, provider)?;
            file(
                file_reference(reference),
                "application",
                None,
                provider_options,
            )
        }
        Item::FileReference {
            provider_reference,
            provider_options,
        } => {
            deprecated_reference(warnings, "file-reference");
            file(
                file_reference(provider_reference),
                "application",
                None,
                provider_options,
            )
        }
        Item::ImageData {
            data,
            media_type,
            provider_options,
        } => {
            deprecated(
                warnings,
                "image-data",
                r#"The "image-data" type for tool result content is deprecated. Use the "file" type with mediaType and { type: 'data', data } instead."#.into(),
            );
            file(base64(&data), &media_type, None, provider_options)
        }
        Item::ImageUrl {
            url: given,
            provider_options,
        } => {
            let data = normalized_url(&given)?;
            deprecated(
                warnings,
                "image-url",
                r#"The "image-url" type for tool result content is deprecated. Use the "file" type with mediaType 'image' (or a specific image/* subtype) and { type: 'url', url } instead."#.into(),
            );
            file(data, "image", None, provider_options)
        }
        Item::ImageFileId {
            file_id,
            provider_options,
        } => {
            deprecated_reference(warnings, "image-file-id");
            let reference = file_id_reference(file_id, provider)?;
            file(file_reference(reference), "image", None, provider_options)
        }
        Item::ImageFileReference {
            provider_reference,
            provider_options,
        } => {
            deprecated_reference(warnings, "image-file-reference");
            file(
                file_reference(provider_reference),
                "image",
                None,
                provider_options,
            )
        }
        Item::Custom { provider_options } => ToolResultContent::Custom { provider_options },
    })
}

/// Upstream `convertFileIdToProviderReference` (`:799-818`): a string id
/// belongs to the provider being called.
fn file_id_reference(
    file_id: user::ToolResultFileId,
    provider: Option<&str>,
) -> Result<SharedProviderReference, AiMuxError> {
    match file_id {
        user::ToolResultFileId::ByProvider(reference) => Ok(reference),
        user::ToolResultFileId::Id(id) => {
            let provider = provider.ok_or_else(|| {
                AiMuxError::invalid_prompt(
                    "Cannot convert string fileId to provider reference without a provider ID. \
                     Use a Record<string, string> fileId or switch to the file-reference type.",
                )
            })?;
            Ok(SharedProviderReference::from([(provider.to_owned(), id)]))
        }
    }
}

const OCTET_STREAM: &str = "application/octet-stream";

/// Upstream `getMediaTypeFromUrl` (`:821-865`): the media type for the URL
/// path's extension, or `application/octet-stream`.
fn media_type_from_url(given: &str) -> &'static str {
    let Ok(parsed) = url::Url::parse(given) else {
        return OCTET_STREAM;
    };
    let extension = parsed
        .path()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        _ => OCTET_STREAM,
    }
}

/// Convert one message (upstream `convertToLanguageModelMessage`,
/// `ai` `prompt/convert-to-language-model-prompt.ts:193`).
fn convert_message(
    message: &ModelMessage,
    context: &mut Context<'_>,
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
                    .map(|part| user_part(part, context.assets, &mut context.warnings))
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
                    .map(|part| assistant_part(part, context))
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
                        Some(tool_result_part(result, context).map(ToolPart::ToolResult))
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
/// - `provider` is the called provider's id, the part of
///   `LanguageModel::provider()` before the first `.` (as upstream passes
///   `stepModel.provider.split('.')[0]`, `generate-text.ts:970`). It is only
///   used to turn a deprecated string `fileId` into a provider reference.
/// - Each message keeps its role and its message-level `provider_options`.
/// - String content becomes a single text part.
/// - The user-facing file variants become one `FilePart`; deprecated
///   `image` parts and deprecated tool output items (`file-data`,
///   `file-url`, `file-id`, `file-reference`, `image-*`) become `file`
///   parts and log a deprecation warning.
/// - Empty text parts are dropped (on an assistant message only when they
///   have no provider options), as are assistant tool approval requests and
///   tool approval responses for tools the provider does not execute.
/// - Consecutive tool messages become one; the earlier message's
///   `provider_options` are merged into its last part (`:104-130`).
/// - A tool message left empty is dropped.
/// - File URLs are sent as they are. `generate_text` and `stream_text` first
///   download the ones the model does not fetch itself
///   ([`crate::download::DownloadFunction`]).
///
/// # Errors
///
/// `InvalidPrompt` when a tool result has no tool name and no preceding tool
/// call, a file reference is not a `{ provider: id }` map, a deprecated
/// tool output URL does not parse, or a string `fileId` has no `provider`.
/// `MissingToolResults` when a tool call the provider did not execute has
/// no tool result and no approval response before the next user or system
/// message, or by the end of the prompt (`:132-175`).
pub fn convert_to_language_model_prompt(
    messages: &[ModelMessage],
    instructions: Option<&str>,
    provider: Option<&str>,
) -> Result<LanguageModelPrompt, AiMuxError> {
    convert_with_warnings(messages, instructions, provider, &DownloadedAssets::new())
        .map(|(prompt, _)| prompt)
}

/// [`convert_to_language_model_prompt`] with the files [`download_assets`]
/// downloaded for the prompt's URLs: a downloaded URL becomes inline data.
pub(crate) fn convert_with_downloaded_assets(
    messages: &[ModelMessage],
    instructions: Option<&str>,
    provider: Option<&str>,
    assets: &DownloadedAssets,
) -> Result<LanguageModelPrompt, AiMuxError> {
    convert_with_warnings(messages, instructions, provider, assets).map(|(prompt, _)| prompt)
}

/// [`convert_to_language_model_prompt`], also returning the deprecation
/// warnings it logged.
fn convert_with_warnings(
    messages: &[ModelMessage],
    instructions: Option<&str>,
    provider: Option<&str>,
    assets: &DownloadedAssets,
) -> Result<(LanguageModelPrompt, Vec<Warning>), AiMuxError> {
    let mut result = Vec::new();
    if let Some(instructions) = instructions {
        result.push(LanguageModelMessage::System {
            content: instructions.to_string(),
            provider_options: None,
        });
    }

    let mut context = Context {
        tool_names: HashMap::new(),
        provider,
        warnings: Vec::new(),
        assets,
    };
    let mut warnings = Vec::new();
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
                    context
                        .tool_names
                        .insert(tool_call_id.clone(), tool_name.clone());
                }
            }
        }
        let converted = convert_message(message, &mut context)?;
        // upstream logs each message's warnings once it is converted
        // (`:246-248`, `:364-366`, `:407-409`)
        for warning in context.warnings.drain(..) {
            log_warning(&warning);
            warnings.push(warning);
        }
        push_combining_tool_messages(&mut result, converted);
    }

    check_tool_results(&result, &approved_tool_call_ids(messages))?;

    // Filter out empty tool messages, e.g. one that held only tool approval
    // responses that were removed (`:177-183`).
    result.retain(|message| {
        !matches!(message, LanguageModelMessage::Tool { content, .. } if content.is_empty())
    });
    Ok((result, warnings))
}

/// Append a converted message, combining consecutive tool messages into one
/// (`:104-130`): the earlier message's `provider_options` are merged into its
/// last part (the part's own options win), and the combined message takes
/// the later message's `provider_options`.
fn push_combining_tool_messages(
    messages: &mut Vec<LanguageModelMessage>,
    message: LanguageModelMessage,
) {
    let (
        Some(LanguageModelMessage::Tool {
            content: last_content,
            provider_options: last_options,
        }),
        LanguageModelMessage::Tool {
            content,
            provider_options,
        },
    ) = (messages.last_mut(), &message)
    else {
        messages.push(message);
        return;
    };
    if let (Some(last_part), Some(_)) = (last_content.last_mut(), last_options.as_ref()) {
        let part_options = match last_part {
            ToolPart::ToolResult(part) => &mut part.provider_options,
            ToolPart::ToolApprovalResponse(part) => &mut part.provider_options,
        };
        *part_options = crate::shared::merge_objects(last_options.as_ref(), part_options.as_ref());
    }
    last_content.extend(content.iter().cloned());
    last_options.clone_from(provider_options);
}

/// The tool calls that have a tool approval response, approved or not
/// (`:57-87`).
fn approved_tool_call_ids(messages: &[ModelMessage]) -> HashSet<&str> {
    let mut tool_call_ids = HashMap::new();
    for message in messages {
        if let ModelMessage::Assistant {
            content: AssistantContent::Parts(parts),
            ..
        } = message
        {
            for part in parts {
                if let AssistantContentPart::ToolApprovalRequest {
                    approval_id,
                    tool_call_id,
                    ..
                } = part
                {
                    tool_call_ids.insert(approval_id.as_str(), tool_call_id.as_str());
                }
            }
        }
    }
    let mut approved = HashSet::new();
    for message in messages {
        if let ModelMessage::Tool { content, .. } = message {
            for part in content {
                if let ToolContentPart::ToolApprovalResponse { approval_id, .. } = part
                    && let Some(tool_call_id) = tool_call_ids.get(approval_id.as_str())
                {
                    approved.insert(*tool_call_id);
                }
            }
        }
    }
    approved
}

/// Every tool call the provider did not execute needs a tool result (or an
/// approval response) before the next user or system message and by the end
/// of the prompt (`:132-175`). The ids are reported in call order.
fn check_tool_results(
    messages: &[LanguageModelMessage],
    approved: &HashSet<&str>,
) -> Result<(), AiMuxError> {
    let mut open: IndexSet<&str> = IndexSet::new();
    let check = |open: &mut IndexSet<&str>| {
        open.retain(|id| !approved.contains(id));
        if open.is_empty() {
            Ok(())
        } else {
            Err(AiMuxError::MissingToolResults {
                tool_call_ids: open.iter().map(|id| (*id).to_owned()).collect(),
            })
        }
    };
    for message in messages {
        match message {
            LanguageModelMessage::Assistant { content, .. } => {
                for part in content {
                    if let AssistantPart::ToolCall(call) = part
                        && call.provider_executed != Some(true)
                    {
                        open.insert(&call.tool_call_id);
                    }
                }
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    if let ToolPart::ToolResult(result) = part {
                        open.shift_remove(result.tool_call_id.as_str());
                    }
                }
            }
            LanguageModelMessage::User { .. } | LanguageModelMessage::System { .. } => {
                check(&mut open)?;
            }
        }
    }
    check(&mut open)
}

/// Log a conversion warning in upstream `logWarnings`' wording
/// (`ai` `logger/log-warnings.ts:71-73`).
fn log_warning(warning: &Warning) {
    if let Warning::Deprecated { setting, message } = warning {
        tracing::warn!("Deprecated: \"{setting}\". {message}");
    }
}

#[cfg(test)]
mod tests {
    //! C37: the deprecated tool output items and the deprecated `image` user
    //! part, against `ai@7.0.127` `prompt/convert-to-language-model-prompt.ts`.

    use super::*;
    use serde_json::json;

    fn tool_output(items: Value) -> Vec<ModelMessage> {
        serde_json::from_value(json!([
            { "role": "tool", "content": [
                { "type": "tool-result", "toolCallId": "c1", "toolName": "t",
                  "output": { "type": "content", "value": items } }
            ] }
        ]))
        .expect("valid messages")
    }

    fn convert_output(
        items: Value,
        provider: Option<&str>,
    ) -> Result<(Vec<ToolResultContent>, Vec<Warning>), AiMuxError> {
        let (prompt, warnings) = convert_with_warnings(
            &tool_output(items),
            None,
            provider,
            &DownloadedAssets::new(),
        )?;
        let [LanguageModelMessage::Tool { content, .. }] = &prompt[..] else {
            panic!("one tool message: {prompt:?}")
        };
        let [ToolPart::ToolResult(result)] = &content[..] else {
            panic!("one tool result: {content:?}")
        };
        let ToolResultOutput::Content { value } = &result.output else {
            panic!("content output: {:?}", result.output)
        };
        Ok((value.clone(), warnings))
    }

    fn file(data: FileData, media_type: &str, filename: Option<&str>) -> ToolResultContent {
        ToolResultContent::File(FilePart {
            data,
            media_type: media_type.into(),
            filename: filename.map(Into::into),
            provider_options: None,
        })
    }

    fn deprecated(setting: &str, message: &str) -> Warning {
        Warning::Deprecated {
            setting: setting.into(),
            message: message.into(),
        }
    }

    fn reference(provider: &str, id: &str) -> FileData {
        FileData::Reference {
            reference: SharedProviderReference::from([(provider.into(), id.into())]),
        }
    }

    // `:642-794`: each deprecated item becomes a `file` part, with
    // upstream's warning setting and message.
    #[test]
    fn deprecated_tool_output_items_become_file_parts() {
        let (parts, warnings) = convert_output(
            json!([
                { "type": "file-data", "data": "AAEC", "mediaType": "application/pdf",
                  "filename": "a.pdf" },
                { "type": "file-url", "url": "https://example.com/a.pdf",
                  "mediaType": "application/pdf" },
                { "type": "file-id", "fileId": "file-1" },
                { "type": "file-id", "fileId": { "anthropic": "file-2" } },
                { "type": "file-reference", "providerReference": { "openai": "file-3" } },
                { "type": "image-data", "data": "iVBO", "mediaType": "image/png" },
                { "type": "image-url", "url": "https://example.com/cat.png" },
                { "type": "image-file-id", "fileId": "file-4" },
                { "type": "image-file-reference", "providerReference": { "openai": "file-5" } },
            ]),
            Some("openai"),
        )
        .unwrap();

        let url = |url: &str| FileData::Url {
            url: url.into(),
            original_url: None,
        };
        assert_eq!(
            parts,
            vec![
                file(base64("AAEC"), "application/pdf", Some("a.pdf")),
                file(url("https://example.com/a.pdf"), "application/pdf", None),
                file(reference("openai", "file-1"), "application", None),
                file(reference("anthropic", "file-2"), "application", None),
                file(reference("openai", "file-3"), "application", None),
                file(base64("iVBO"), "image/png", None),
                file(url("https://example.com/cat.png"), "image", None),
                file(reference("openai", "file-4"), "image", None),
                file(reference("openai", "file-5"), "image", None),
            ]
        );

        let reference_message = |kind: &str| {
            format!(
                "The \"{kind}\" type for tool result content is deprecated. Use the \"file\" type with mediaType and {{ type: 'reference', reference }} instead."
            )
        };
        let setting = |kind: &str| format!("\"tool-result\" content of type \"{kind}\"");
        assert_eq!(
            warnings,
            vec![
                deprecated(
                    &setting("file-data"),
                    "The \"file-data\" type for tool result content is deprecated. Use the \"file\" type with mediaType and { type: 'data', data } instead.",
                ),
                deprecated(
                    &setting("file-url"),
                    "The \"file-url\" type for tool result content is deprecated. Use the \"file\" type with mediaType and { type: 'url', url } instead.",
                ),
                deprecated(&setting("file-id"), &reference_message("file-id")),
                deprecated(&setting("file-id"), &reference_message("file-id")),
                deprecated(
                    &setting("file-reference"),
                    &reference_message("file-reference")
                ),
                deprecated(
                    &setting("image-data"),
                    "The \"image-data\" type for tool result content is deprecated. Use the \"file\" type with mediaType and { type: 'data', data } instead.",
                ),
                deprecated(
                    &setting("image-url"),
                    "The \"image-url\" type for tool result content is deprecated. Use the \"file\" type with mediaType 'image' (or a specific image/* subtype) and { type: 'url', url } instead.",
                ),
                deprecated(
                    &setting("image-file-id"),
                    &reference_message("image-file-id")
                ),
                deprecated(
                    &setting("image-file-reference"),
                    &reference_message("image-file-reference")
                ),
            ]
        );
    }

    // `:658-690` and `getMediaTypeFromUrl` (`:821-865`): a `file-url` with no
    // media type gets one from the URL path's extension, or
    // `application/octet-stream`, and the warning says so; a URL that
    // normalizes differently keeps its given spelling as `original_url`.
    #[test]
    fn file_url_without_media_type_infers_it_from_the_url() {
        let (parts, warnings) = convert_output(
            json!([
                { "type": "file-url", "url": "HTTPS://example.com/photo.JPG" },
                { "type": "file-url", "url": "https://example.com/download" },
            ]),
            None,
        )
        .unwrap();
        assert_eq!(
            parts,
            vec![
                file(
                    FileData::Url {
                        url: "https://example.com/photo.JPG".into(),
                        original_url: Some("HTTPS://example.com/photo.JPG".into()),
                    },
                    "image/jpeg",
                    None,
                ),
                file(
                    FileData::Url {
                        url: "https://example.com/download".into(),
                        original_url: None,
                    },
                    "application/octet-stream",
                    None,
                ),
            ]
        );
        let tail = "The \"file-url\" type for tool result content is deprecated. Use the \"file\" type with mediaType and { type: 'url', url } instead.";
        assert_eq!(
            warnings,
            vec![
                deprecated(
                    "\"tool-result\" content of type \"file-url\"",
                    &format!(
                        "The \"file-url\" tool result content part with URL \"HTTPS://example.com/photo.JPG\" is missing a \"mediaType\". Inferred media type 'image/jpeg' from URL. {tail}"
                    ),
                ),
                deprecated(
                    "\"tool-result\" content of type \"file-url\"",
                    &format!(
                        "The \"file-url\" tool result content part with URL \"https://example.com/download\" is missing a \"mediaType\". Unable to infer media type from URL. Defaulting to 'application/octet-stream'. {tail}"
                    ),
                ),
            ]
        );
    }

    // `convertFileIdToProviderReference` (`:799-818`): a string id needs the
    // provider being called.
    #[test]
    fn string_file_id_needs_a_provider() {
        let error =
            convert_output(json!([{ "type": "file-id", "fileId": "file-1" }]), None).unwrap_err();
        assert_eq!(
            error.to_string(),
            AiMuxError::invalid_prompt(
                "Cannot convert string fileId to provider reference without a provider ID. Use a Record<string, string> fileId or switch to the file-reference type."
            )
            .to_string()
        );
    }

    // Non-deprecated items pass through with no warning (`:632-645`, `:792`).
    #[test]
    fn current_tool_output_items_log_no_warning() {
        let (parts, warnings) = convert_output(
            json!([
                { "type": "text", "text": "done" },
                { "type": "custom" },
            ]),
            None,
        )
        .unwrap();
        assert_eq!(parts.len(), 2);
        assert!(warnings.is_empty());
    }

    // `:229-238`: a user `image` part becomes a `file` part and logs a
    // deprecation warning.
    #[test]
    fn user_image_part_logs_a_deprecation_warning() {
        let messages: Vec<ModelMessage> = serde_json::from_value(json!([
            { "role": "user", "content": [
                { "type": "image", "image": "AQI=", "mediaType": "image/png" }
            ] }
        ]))
        .unwrap();
        let (prompt, warnings) =
            convert_with_warnings(&messages, None, None, &DownloadedAssets::new()).unwrap();
        let [LanguageModelMessage::User { content, .. }] = &prompt[..] else {
            panic!()
        };
        assert!(
            matches!(&content[..], [UserPart::File(FilePart { media_type, .. })] if media_type == "image/png")
        );
        assert_eq!(
            warnings,
            vec![deprecated(
                "\"image\" content part",
                "The \"image\" content part type is deprecated. Use a \"file\" part with mediaType: 'image' (or a more specific image/* subtype) instead.",
            )]
        );
    }
}

#[cfg(test)]
mod download_tests {
    //! Prompt downloads, against `ai@7.0.127`
    //! `prompt/convert-to-language-model-prompt.ts` (`downloadAssets`,
    //! `convertPartToLanguageModelPart`).

    use super::*;
    use std::sync::{Arc, Mutex};

    use regex::Regex;
    use serde_json::json;

    fn url(url: &str) -> FileData {
        FileData::Url {
            url: url.to_owned(),
            original_url: None,
        }
    }

    fn messages() -> Vec<ModelMessage> {
        serde_json::from_value(json!([
            { "role": "user", "content": [
                { "type": "file", "data": "https://model.test/a.png", "mediaType": "image/png" },
                { "type": "file", "data": { "type": "url", "url": "https://cdn.test/b" }, "mediaType": "image" },
                { "type": "file", "data": "data:image/png;base64,AAAA", "mediaType": "image/png" }
            ] },
            { "role": "assistant", "content": [
                { "type": "file", "data": "https://cdn.test/assistant.png", "mediaType": "image/png" },
                { "type": "tool-call", "toolCallId": "c1", "toolName": "t", "input": {} }
            ] },
            { "role": "tool", "content": [
                { "type": "tool-result", "toolCallId": "c1", "toolName": "t",
                  "output": { "type": "content", "value": [
                      { "type": "file", "data": { "type": "url", "url": "https://cdn.test/c.pdf" },
                        "mediaType": "application/pdf" },
                      { "type": "file-url", "url": "https://cdn.test/c.pdf", "mediaType": "application/pdf" }
                  ] } }
            ] }
        ]))
        .expect("valid messages")
    }

    fn supported_urls() -> SupportedUrls {
        SupportedUrls(HashMap::from([(
            "image/*".to_owned(),
            vec![Regex::new(r"^https://model\.test/").unwrap()],
        )]))
    }

    /// A download function that records what it is asked for and downloads
    /// every URL the model does not fetch itself.
    fn recording(planned: Arc<Mutex<Vec<PlannedDownload>>>) -> DownloadFunction {
        DownloadFunction::new(move |downloads: Vec<PlannedDownload>| {
            planned.lock().unwrap().extend(downloads.clone());
            async move {
                Ok(downloads
                    .into_iter()
                    .map(|download| {
                        (!download.is_url_supported_by_model).then(|| DownloadedFile {
                            data: download.url.into_bytes(),
                            media_type: Some("image/jpeg".into()),
                        })
                    })
                    .collect())
            }
        })
    }

    #[tokio::test]
    async fn user_file_urls_and_tool_output_files_are_planned_in_prompt_order() {
        let planned = Arc::new(Mutex::new(Vec::new()));
        download_assets(&messages(), &recording(planned.clone()), &supported_urls())
            .await
            .unwrap();
        let planned = planned.lock().unwrap().clone();
        assert_eq!(
            planned,
            [
                PlannedDownload {
                    url: "https://model.test/a.png".into(),
                    is_url_supported_by_model: true,
                },
                PlannedDownload {
                    url: "https://cdn.test/b".into(),
                    is_url_supported_by_model: false,
                },
                PlannedDownload {
                    url: "https://cdn.test/c.pdf".into(),
                    is_url_supported_by_model: false,
                },
            ],
            "no data: URL, no assistant file part, no deprecated file-url item"
        );
    }

    #[tokio::test]
    async fn downloaded_urls_become_inline_data_and_the_others_stay_urls() {
        let assets = download_assets(
            &messages(),
            &recording(Arc::new(Mutex::new(Vec::new()))),
            &supported_urls(),
        )
        .await
        .unwrap();
        let prompt = convert_with_downloaded_assets(&messages(), None, None, &assets).unwrap();

        let LanguageModelMessage::User { content, .. } = &prompt[0] else {
            panic!("user message: {:?}", prompt[0])
        };
        let files: Vec<_> = content
            .iter()
            .map(|part| match part {
                UserPart::File(file) => (file.data.clone(), file.media_type.as_str()),
                UserPart::Text(_) => panic!("file parts only"),
            })
            .collect();
        assert_eq!(
            files,
            [
                // The model fetches it: the URL is sent.
                (url("https://model.test/a.png"), "image/png"),
                // Downloaded: a top-level media type takes the downloaded one.
                (bytes(b"https://cdn.test/b"), "image/jpeg"),
                // A `data:` URL is inline data (`file-part-data.ts:31-41`).
                (base64("AAAA"), "image/png"),
            ]
        );

        let LanguageModelMessage::Assistant { content, .. } = &prompt[1] else {
            panic!("assistant message: {:?}", prompt[1])
        };
        assert!(
            matches!(&content[0], AssistantPart::File(file)
                if file.data == url("https://cdn.test/assistant.png")),
            "{content:?}"
        );

        let LanguageModelMessage::Tool { content, .. } = &prompt[2] else {
            panic!("tool message: {:?}", prompt[2])
        };
        let [ToolPart::ToolResult(result)] = &content[..] else {
            panic!("one tool result: {content:?}")
        };
        let ToolResultOutput::Content { value } = &result.output else {
            panic!("content output: {:?}", result.output)
        };
        let [
            ToolResultContent::File(file),
            ToolResultContent::File(deprecated),
        ] = &value[..]
        else {
            panic!("two files: {value:?}")
        };
        // A full media type is kept.
        assert_eq!(file.data, bytes(b"https://cdn.test/c.pdf"));
        assert_eq!(file.media_type, "application/pdf");
        // The deprecated file-url item is not downloaded.
        assert_eq!(deprecated.data, url("https://cdn.test/c.pdf"));
    }
}
